use axum::extract::{Path, Query, State};
use axum::Json;
use serde::Deserialize;
use utoipa::{IntoParams, ToSchema};
use utoipa_axum::{router::OpenApiRouter, routes};
use uuid::Uuid;

use crate::auth::AuthAdmin;
use crate::error::ApiError;
use crate::openapi::ErrorResponse;
use crate::state::AppState;
use vaultscope_db::models::Service;

pub fn routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(list, create))
        .routes(routes!(get_one))
        .routes(routes!(suspend))
        .routes(routes!(reactivate))
        .routes(routes!(terminate))
        .routes(routes!(admin_ipmi, admin_ipmi_get))
}

#[derive(Deserialize, IntoParams)]
pub(crate) struct ListParams {
    customer_id: Option<Uuid>,
    status: Option<String>,
    limit: Option<i64>,
    offset: Option<i64>,
}

#[utoipa::path(
    get,
    path = "/",
    tag = "Admin Services",
    security(("bearer_auth" = [])),
    params(ListParams),
    responses((status = 200, description = "List customer services", body = Vec<Service>))
)]
pub(crate) async fn list(
    auth: AuthAdmin,
    State(state): State<AppState>,
    Query(params): Query<ListParams>,
) -> Result<Json<Vec<Service>>, ApiError> {
    crate::require_permission!(auth, state, "services.view");
    let limit = params.limit.unwrap_or(50).clamp(1, 500);
    let offset = params.offset.unwrap_or(0).max(0);

    let services: Vec<Service> = match (params.customer_id, &params.status) {
        (Some(cid), Some(status)) => sqlx::query_as(
            "SELECT * FROM services WHERE customer_id = $1 AND status = $2::service_status ORDER BY created_at DESC LIMIT $3 OFFSET $4"
        )
        .bind(cid)
        .bind(status)
        .bind(limit)
        .bind(offset)
        .fetch_all(&state.db)
        .await?,
        (Some(cid), None) => sqlx::query_as(
            "SELECT * FROM services WHERE customer_id = $1 ORDER BY created_at DESC LIMIT $2 OFFSET $3"
        )
        .bind(cid)
        .bind(limit)
        .bind(offset)
        .fetch_all(&state.db)
        .await?,
        (None, Some(status)) => sqlx::query_as(
            "SELECT * FROM services WHERE status = $1::service_status ORDER BY created_at DESC LIMIT $2 OFFSET $3"
        )
        .bind(status)
        .bind(limit)
        .bind(offset)
        .fetch_all(&state.db)
        .await?,
        (None, None) => sqlx::query_as(
            "SELECT * FROM services ORDER BY created_at DESC LIMIT $1 OFFSET $2"
        )
        .bind(limit)
        .bind(offset)
        .fetch_all(&state.db)
        .await?,
    };

    Ok(Json(services))
}

#[utoipa::path(
    get,
    path = "/{id}",
    tag = "Admin Services",
    security(("bearer_auth" = [])),
    params(("id" = Uuid, Path, description = "Service ID")),
    responses(
        (status = 200, description = "Service details", body = Service),
        (status = 404, description = "Service not found", body = ErrorResponse)
    )
)]
pub(crate) async fn get_one(
    auth: AuthAdmin,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<Service>, ApiError> {
    crate::require_permission!(auth, state, "services.view");
    let service: Service = sqlx::query_as("SELECT * FROM services WHERE id = $1")
        .bind(id)
        .fetch_optional(&state.db)
        .await?
        .ok_or(ApiError::NotFound("service not found".into()))?;
    Ok(Json(service))
}

#[derive(serde::Deserialize, ToSchema)]
pub struct CreateService {
    pub customer_id: uuid::Uuid,
    pub product_id: uuid::Uuid,
    pub name: Option<String>,
    pub hostname: Option<String>,
    pub price: rust_decimal::Decimal,
    pub trial_days: Option<i32>,
    pub is_trial: Option<bool>,
}

#[utoipa::path(
    post,
    path = "/",
    tag = "Admin Services",
    security(("bearer_auth" = []), ("csrf_token" = [])),
    request_body = CreateService,
    responses((status = 200, description = "Manual service created", body = Service))
)]
pub async fn create(
    auth: AuthAdmin,
    State(state): State<AppState>,
    axum::Json(payload): axum::Json<CreateService>,
) -> Result<axum::Json<Service>, ApiError> {
    crate::require_permission!(auth, state, "services.manage");
    if payload.price < rust_decimal::Decimal::ZERO {
        return Err(ApiError::BadRequest("Service price cannot be negative".into()));
    }

    let product_trial_days: Option<i32> = sqlx::query_scalar("SELECT trial_days FROM products WHERE id = $1")
        .bind(payload.product_id)
        .fetch_optional(&state.db)
        .await?
        .flatten();

    let trial_days = payload.trial_days.or(product_trial_days);
    let is_trial = payload.is_trial.unwrap_or_else(|| trial_days.unwrap_or(0) > 0);

    let (next_due, trial_ends_at) = if is_trial {
        let days = trial_days.unwrap_or(14).max(1) as i64;
        let expiry = chrono::Utc::now() + chrono::Duration::days(days);
        (Some(expiry), Some(expiry))
    } else {
        (Some(chrono::Utc::now() + chrono::Duration::days(30)), None)
    };

    let service: Service = sqlx::query_as(
        r#"INSERT INTO services (customer_id, product_id, name, hostname, price, status, next_due, is_trial, trial_ends_at)
           VALUES ($1, $2, $3, $4, $5, 'running'::service_status, $6, $7, $8)
           RETURNING *"#,
    )
    .bind(payload.customer_id)
    .bind(payload.product_id)
    .bind(payload.name.unwrap_or_else(|| "Manual Service".to_string()))
    .bind(payload.hostname)
    .bind(payload.price)
    .bind(next_due)
    .bind(is_trial)
    .bind(trial_ends_at)
    .fetch_one(&state.db)
    .await?;

    Ok(axum::Json(service))
}

#[utoipa::path(
    post,
    path = "/{id}/suspend",
    tag = "Admin Services",
    security(("bearer_auth" = []), ("csrf_token" = [])),
    params(("id" = Uuid, Path, description = "Service ID")),
    responses(
        (status = 200, description = "Service suspended", body = Service),
        (status = 404, description = "Service not found", body = ErrorResponse)
    )
)]
pub async fn suspend(
    auth: AuthAdmin,
    State(state): State<AppState>,
    axum::extract::Path(id): axum::extract::Path<uuid::Uuid>,
) -> Result<axum::Json<Service>, ApiError> {
    crate::require_permission!(auth, state, "services.power");
    let service: Service = sqlx::query_as(
        r#"UPDATE services SET status = 'suspended', updated_at = now() WHERE id = $1 RETURNING *"#,
    )
    .bind(id)
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| ApiError::NotFound("Service not found".into()))?;
    Ok(axum::Json(service))
}

#[utoipa::path(
    post,
    path = "/{id}/reactivate",
    tag = "Admin Services",
    security(("bearer_auth" = []), ("csrf_token" = [])),
    params(("id" = Uuid, Path, description = "Service ID")),
    responses(
        (status = 200, description = "Service reactivated", body = Service),
        (status = 404, description = "Service not found", body = ErrorResponse)
    )
)]
pub async fn reactivate(
    auth: AuthAdmin,
    State(state): State<AppState>,
    axum::extract::Path(id): axum::extract::Path<uuid::Uuid>,
) -> Result<axum::Json<Service>, ApiError> {
    crate::require_permission!(auth, state, "services.power");
    let service: Service = sqlx::query_as(
        r#"UPDATE services SET status = 'running', updated_at = now() WHERE id = $1 RETURNING *"#,
    )
    .bind(id)
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| ApiError::NotFound("Service not found".into()))?;
    Ok(axum::Json(service))
}

#[utoipa::path(
    post,
    path = "/{id}/terminate",
    tag = "Admin Services",
    security(("bearer_auth" = []), ("csrf_token" = [])),
    params(("id" = Uuid, Path, description = "Service ID")),
    responses(
        (status = 200, description = "Service terminated", body = Service),
        (status = 404, description = "Service not found", body = ErrorResponse)
    )
)]
pub async fn terminate(
    auth: AuthAdmin,
    State(state): State<AppState>,
    axum::extract::Path(id): axum::extract::Path<uuid::Uuid>,
) -> Result<axum::Json<Service>, ApiError> {
    crate::require_permission!(auth, state, "services.power");
    let service: Service = sqlx::query_as(
        r#"UPDATE services SET status = 'terminated', updated_at = now() WHERE id = $1 RETURNING *"#,
    )
    .bind(id)
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| ApiError::NotFound("Service not found".into()))?;
    Ok(axum::Json(service))
}

#[derive(Deserialize, Default, ToSchema)]
pub struct AdminIpmiPayload {
    pub client_ip: Option<String>,
}

pub fn validate_optional_client_ip(raw: Option<&str>) -> Result<Option<String>, ApiError> {
    let Some(ip_str) = raw.map(str::trim).filter(|s| !s.is_empty()) else {
        return Ok(None);
    };
    let parsed: std::net::IpAddr = ip_str
        .parse()
        .map_err(|_| ApiError::BadRequest("Invalid client_ip address".into()))?;
    Ok(Some(parsed.to_string()))
}

#[utoipa::path(
    post,
    path = "/{id}/ipmi",
    tag = "Admin Services",
    security(("bearer_auth" = []), ("csrf_token" = [])),
    params(("id" = Uuid, Path, description = "Service ID")),
    request_body = Option<AdminIpmiPayload>,
    responses(
        (status = 200, description = "IPMI KVM session details", body = serde_json::Value),
        (status = 400, description = "Invalid client_ip", body = ErrorResponse),
        (status = 403, description = "Insufficient permissions", body = ErrorResponse),
        (status = 404, description = "Service not found", body = ErrorResponse)
    )
)]
pub async fn admin_ipmi(
    auth: AuthAdmin,
    State(state): State<AppState>,
    axum::extract::Path(id): axum::extract::Path<uuid::Uuid>,
    payload: Option<Json<AdminIpmiPayload>>,
) -> Result<axum::Json<serde_json::Value>, ApiError> {
    crate::require_permission!(auth, state, "services.power");
    let validated_ip = validate_optional_client_ip(
        payload.as_ref().and_then(|p| p.0.client_ip.as_deref()),
    )?;

    let service: Service = sqlx::query_as(
        "SELECT * FROM services WHERE id = $1"
    )
    .bind(id)
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| ApiError::NotFound("Service not found".into()))?;

    let resource_id = if service.provider_resource_id.is_empty() {
        format!("rs-ds-{}", &service.id.to_string()[..8])
    } else {
        service.provider_resource_id.clone()
    };

    let rs = state.config.reliablesite_connector();
    let effective_ip = validated_ip
        .as_deref()
        .unwrap_or("Admin Staff Session");

    let session = rs.request_ipmi_session(&resource_id, Some(effective_ip)).await
        .map_err(|e| ApiError::Internal(format!("Failed to generate IPMI session: {e}")))?;

    Ok(axum::Json(session))
}

#[utoipa::path(
    get,
    path = "/{id}/ipmi",
    tag = "Admin Services",
    security(("bearer_auth" = [])),
    params(("id" = Uuid, Path, description = "Service ID")),
    responses(
        (status = 200, description = "IPMI KVM session details", body = serde_json::Value),
        (status = 403, description = "Insufficient permissions", body = ErrorResponse),
        (status = 404, description = "Service not found", body = ErrorResponse)
    )
)]
pub(crate) async fn admin_ipmi_get(
    auth: AuthAdmin,
    state: State<AppState>,
    path: axum::extract::Path<uuid::Uuid>,
) -> Result<axum::Json<serde_json::Value>, ApiError> {
    admin_ipmi(auth, state, path, None).await
}

