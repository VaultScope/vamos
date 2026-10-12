use axum::extract::{Path, Query, State};
use axum::routing::get;
use axum::{Json, Router};
use serde::Deserialize;
use uuid::Uuid;

use crate::auth::AuthAdmin;
use crate::error::ApiError;
use crate::state::AppState;
use vaultscope_db::models::Service;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/", get(list).post(create))
        .route("/{id}", get(get_one))
        .route("/{id}/suspend", axum::routing::post(suspend))
        .route("/{id}/reactivate", axum::routing::post(reactivate))
        .route("/{id}/terminate", axum::routing::post(terminate))
        .route("/{id}/ipmi", axum::routing::post(admin_ipmi).get(admin_ipmi))
}

#[derive(Deserialize)]
struct ListParams {
    customer_id: Option<Uuid>,
    status: Option<String>,
    limit: Option<i64>,
    offset: Option<i64>,
}

async fn list(
    auth: AuthAdmin,
    State(state): State<AppState>,
    Query(params): Query<ListParams>,
) -> Result<Json<Vec<Service>>, ApiError> {
    crate::require_permission!(auth, state, "services.view");
    let limit = params.limit.unwrap_or(50).min(200);
    let offset = params.offset.unwrap_or(0);

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

async fn get_one(
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

#[derive(serde::Deserialize)]
pub struct CreateService {
    pub customer_id: uuid::Uuid,
    pub product_id: uuid::Uuid,
    pub name: Option<String>,
    pub hostname: Option<String>,
    pub price: rust_decimal::Decimal,
}

pub async fn create(
    auth: AuthAdmin,
    State(state): State<AppState>,
    axum::Json(payload): axum::Json<CreateService>,
) -> Result<axum::Json<Service>, ApiError> {
    crate::require_permission!(auth, state, "services.power");
    let service: Service = sqlx::query_as(
        r#"INSERT INTO services (customer_id, product_id, name, hostname, price, status)
           VALUES ($1, $2, $3, $4, $5, 'running'::service_status)
           RETURNING *"#,
    )
    .bind(payload.customer_id)
    .bind(payload.product_id)
    .bind(payload.name.unwrap_or_else(|| "Manual Service".to_string()))
    .bind(payload.hostname)
    .bind(payload.price)
    .fetch_one(&state.db)
    .await?;

    Ok(axum::Json(service))
}

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

pub async fn admin_ipmi(
    auth: AuthAdmin,
    State(state): State<AppState>,
    axum::extract::Path(id): axum::extract::Path<uuid::Uuid>,
) -> Result<axum::Json<serde_json::Value>, ApiError> {
    crate::require_permission!(auth, state, "services.view");
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

    let api_key = state.config.reliablesite_api_key.as_deref().unwrap_or("").trim().to_string();
    let base_url = state.config.reliablesite_api_url.as_deref().unwrap_or("https://api.reliablesite.net").trim().to_string();
    let rs = vaultscope_connectors::reliablesite::ReliableSite::new(api_key, base_url);

    let session = rs.request_ipmi_session(&resource_id, Some("Admin Staff Session")).await
        .map_err(|e| ApiError::Internal(format!("Failed to generate IPMI session: {e}")))?;

    Ok(axum::Json(session))
}

