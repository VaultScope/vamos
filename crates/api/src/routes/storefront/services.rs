use axum::extract::{Path, State};
use axum::http::header;
use axum::response::IntoResponse;
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use utoipa::ToSchema;
use utoipa_axum::{router::OpenApiRouter, routes};
use uuid::Uuid;

use crate::auth::AuthCustomer;
use crate::error::ApiError;
use crate::openapi::{ErrorResponse, OkMessageResponse, OkResponse};
use crate::state::AppState;
use vaultscope_common::types::PowerAction;
use vaultscope_db::models::Service;

pub fn routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(list_mine))
        .routes(routes!(get_one, delete_service))
        .routes(routes!(get_credentials))
        .routes(routes!(power_start))
        .routes(routes!(power_stop))
        .routes(routes!(power_restart))
        .routes(routes!(reinstall))
        .routes(routes!(vnc_console))
        .routes(routes!(ipmi_access, ipmi_access_get))
        .routes(routes!(cancel_service))
        .routes(routes!(uncancel_service))
}

#[derive(Serialize, ToSchema)]
pub struct ServiceCredentialsResponse {
    pub username: String,
    pub password: Option<String>,
}

#[allow(dead_code)]
#[derive(Serialize, ToSchema)]
pub struct JobQueuedResponse {
    pub job_id: Uuid,
    pub status: String,
}

#[utoipa::path(
    get,
    path = "/",
    tag = "Storefront Services",
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "Customer services", body = Vec<Service>),
        (status = 401, description = "Unauthorized", body = ErrorResponse)
    )
)]
pub(crate) async fn list_mine(
    auth: AuthCustomer,
    State(state): State<AppState>,
) -> Result<Json<Vec<Service>>, ApiError> {
    let services: Vec<Service> =
        sqlx::query_as("SELECT * FROM services WHERE customer_id = $1 ORDER BY created_at DESC")
            .bind(auth.0.sub)
            .fetch_all(&state.db)
            .await?;
    Ok(Json(
        services
            .into_iter()
            .map(crate::reliablesite_catalog::sanitize_service_for_storefront)
            .collect(),
    ))
}

#[utoipa::path(
    get,
    path = "/{id}",
    tag = "Storefront Services",
    security(("bearer_auth" = [])),
    params(("id" = Uuid, Path, description = "Service ID")),
    responses(
        (status = 200, description = "Customer service details", body = Service),
        (status = 404, description = "Service not found", body = ErrorResponse)
    )
)]
pub(crate) async fn get_one(
    auth: AuthCustomer,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<Service>, ApiError> {
    let service: Service =
        sqlx::query_as("SELECT * FROM services WHERE id = $1 AND customer_id = $2")
            .bind(id)
            .bind(auth.0.sub)
            .fetch_optional(&state.db)
            .await?
            .ok_or(ApiError::NotFound("service not found".into()))?;
    Ok(Json(
        crate::reliablesite_catalog::sanitize_service_for_storefront(service),
    ))
}

#[utoipa::path(
    get,
    path = "/{id}/credentials",
    tag = "Storefront Services",
    security(("bearer_auth" = [])),
    params(("id" = Uuid, Path, description = "Service ID")),
    responses(
        (status = 200, description = "Decrypted service credentials", body = ServiceCredentialsResponse),
        (status = 404, description = "Service not found", body = ErrorResponse)
    )
)]
pub(crate) async fn get_credentials(
    auth: AuthCustomer,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<impl IntoResponse, ApiError> {
    let (provider_resource_id, config, encrypted_password): (
        String,
        Value,
        Option<String>,
    ) = sqlx::query_as(
        "SELECT provider_resource_id, config, encrypted_password FROM services WHERE id = $1 AND customer_id = $2",
    )
    .bind(id)
    .bind(auth.0.sub)
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound("service not found".into()))?;

    let password = match encrypted_password.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        Some(enc) => {
            let key = state.config.encryption_key_bytes();
            let decrypted = vaultscope_common::crypto::decrypt_secret_string(&key, enc).map_err(|e| {
                tracing::error!(service_id = %id, "Failed to decrypt service credentials: {}", e);
                ApiError::Internal("Failed to decrypt service credentials".into())
            })?;
            Some(decrypted)
        }
        None => config
            .get("raw_response")
            .and_then(|r| r.get("password"))
            .and_then(|v| v.as_str())
            .or_else(|| {
                config
                    .get("raw_response")
                    .and_then(|r| r.get("root_password"))
                    .and_then(|v| v.as_str())
            })
            .or_else(|| config.get("password").and_then(|v| v.as_str()))
            .or_else(|| config.get("root_password").and_then(|v| v.as_str()))
            .filter(|s| !s.is_empty())
            .map(str::to_string),
    };

    Ok((
        [(header::CACHE_CONTROL, "no-store")],
        Json(ServiceCredentialsResponse {
            username: provider_resource_id,
            password,
        }),
    ))
}

#[utoipa::path(
    post,
    path = "/{id}/start",
    tag = "Storefront Services",
    security(("bearer_auth" = []), ("csrf_token" = [])),
    params(("id" = Uuid, Path, description = "Service ID")),
    responses(
        (status = 200, description = "Power-on job queued", body = JobQueuedResponse),
        (status = 404, description = "Service not found", body = ErrorResponse)
    )
)]
pub(crate) async fn power_start(
    auth: AuthCustomer,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, ApiError> {
    power_action_helper(auth, state, id, PowerAction::Start).await
}

#[utoipa::path(
    post,
    path = "/{id}/stop",
    tag = "Storefront Services",
    security(("bearer_auth" = []), ("csrf_token" = [])),
    params(("id" = Uuid, Path, description = "Service ID")),
    responses(
        (status = 200, description = "Power-off job queued", body = JobQueuedResponse),
        (status = 404, description = "Service not found", body = ErrorResponse)
    )
)]
pub(crate) async fn power_stop(
    auth: AuthCustomer,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, ApiError> {
    power_action_helper(auth, state, id, PowerAction::Stop).await
}

#[utoipa::path(
    post,
    path = "/{id}/restart",
    tag = "Storefront Services",
    security(("bearer_auth" = []), ("csrf_token" = [])),
    params(("id" = Uuid, Path, description = "Service ID")),
    responses(
        (status = 200, description = "Reboot job queued", body = JobQueuedResponse),
        (status = 404, description = "Service not found", body = ErrorResponse)
    )
)]
pub(crate) async fn power_restart(
    auth: AuthCustomer,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, ApiError> {
    power_action_helper(auth, state, id, PowerAction::Restart).await
}

async fn get_service_provider(db: &sqlx::PgPool, service: &Service) -> String {
    let prod_provider: Option<(String,)> =
        sqlx::query_as("SELECT provider FROM products WHERE id = $1")
            .bind(service.product_id)
            .fetch_optional(db)
            .await
            .unwrap_or(None);

    if let Some((p,)) = prod_provider {
        if !p.trim().is_empty() {
            return p;
        }
    }
    if let Some(p) = service.config.get("provider").and_then(|v| v.as_str()) {
        if !p.trim().is_empty() {
            return p.to_string();
        }
    }
    if service.name.to_lowercase().contains("dedicated") {
        "reliablesite".to_string()
    } else {
        "hetzner_cloud".to_string()
    }
}

async fn power_action_helper(
    auth: AuthCustomer,
    state: AppState,
    service_id: Uuid,
    action: PowerAction,
) -> Result<Json<Value>, ApiError> {
    let service: Service =
        sqlx::query_as("SELECT * FROM services WHERE id = $1 AND customer_id = $2")
            .bind(service_id)
            .bind(auth.0.sub)
            .fetch_optional(&state.db)
            .await?
            .ok_or(ApiError::NotFound("service not found".into()))?;

    if service.status != vaultscope_db::models::ServiceStatus::Running {
        return Err(ApiError::BadRequest(
            "Power actions are only available on active running services".into(),
        ));
    }

    if service.provider_resource_id.is_empty() {
        return Err(ApiError::BadRequest("service not yet provisioned".into()));
    }

    let provider = get_service_provider(&state.db, &service).await;
    let job_id = Uuid::now_v7();
    let payload = json!({
        "resource_id": service.provider_resource_id,
        "action": action
    });

    sqlx::query(
        "INSERT INTO jobs (id, task, target_api, customer_id, service_id, request_payload) VALUES ($1, 'power_action', $2, $3, $4, $5)"
    )
    .bind(job_id)
    .bind(provider)
    .bind(auth.0.sub)
    .bind(service_id)
    .bind(&payload)
    .execute(&state.db)
    .await?;

    Ok(Json(json!({ "job_id": job_id, "status": "queued" })))
}

#[derive(Deserialize, ToSchema)]
pub(crate) struct ReinstallPayload {
    os: String,
}

#[utoipa::path(
    post,
    path = "/{id}/reinstall",
    tag = "Storefront Services",
    security(("bearer_auth" = []), ("csrf_token" = [])),
    params(("id" = Uuid, Path, description = "Service ID")),
    request_body = ReinstallPayload,
    responses(
        (status = 200, description = "OS reinstall job queued", body = JobQueuedResponse),
        (status = 400, description = "Service not yet provisioned", body = ErrorResponse),
        (status = 404, description = "Service not found", body = ErrorResponse)
    )
)]
pub(crate) async fn reinstall(
    auth: AuthCustomer,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(payload): Json<ReinstallPayload>,
) -> Result<Json<Value>, ApiError> {
    let os_trimmed = payload.os.trim();
    const ALLOWED_OS_IMAGES: &[&str] = &[
        "ubuntu-22.04",
        "ubuntu-24.04",
        "debian-11",
        "debian-12",
        "centos-stream-9",
        "almalinux-9",
        "rockylinux-9",
        "fedora-39",
        "fedora-40",
    ];
    if !ALLOWED_OS_IMAGES.contains(&os_trimmed) {
        return Err(ApiError::BadRequest(format!(
            "Unsupported OS image '{}'",
            os_trimmed
        )));
    }

    let service: Service =
        sqlx::query_as("SELECT * FROM services WHERE id = $1 AND customer_id = $2")
            .bind(id)
            .bind(auth.0.sub)
            .fetch_optional(&state.db)
            .await?
            .ok_or(ApiError::NotFound("service not found".into()))?;

    if service.status != vaultscope_db::models::ServiceStatus::Running {
        return Err(ApiError::BadRequest(
            "Service must be running to reinstall the operating system".into(),
        ));
    }

    if service.provider_resource_id.is_empty() {
        return Err(ApiError::BadRequest("service not yet provisioned".into()));
    }

    let provider = get_service_provider(&state.db, &service).await;
    let job_id = Uuid::now_v7();
    let reinstall_payload = json!({
        "resource_id": service.provider_resource_id,
        "params": {
            "image": os_trimmed
        }
    });

    sqlx::query(
        "INSERT INTO jobs (id, task, target_api, customer_id, service_id, request_payload) VALUES ($1, 'reinstall', $2, $3, $4, $5)"
    )
    .bind(job_id)
    .bind(provider)
    .bind(auth.0.sub)
    .bind(id)
    .bind(&reinstall_payload)
    .execute(&state.db)
    .await?;

    Ok(Json(json!({ "job_id": job_id, "status": "queued" })))
}

#[utoipa::path(
    delete,
    path = "/{id}",
    tag = "Storefront Services",
    security(("bearer_auth" = []), ("csrf_token" = [])),
    params(("id" = Uuid, Path, description = "Service ID")),
    responses(
        (status = 200, description = "Service deleted/terminated", body = OkResponse),
        (status = 400, description = "Cannot delete a running service", body = ErrorResponse),
        (status = 404, description = "Service not found", body = ErrorResponse)
    )
)]
pub(crate) async fn delete_service(
    auth: AuthCustomer,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, ApiError> {
    let service: Service =
        sqlx::query_as("SELECT * FROM services WHERE id = $1 AND customer_id = $2")
            .bind(id)
            .bind(auth.0.sub)
            .fetch_optional(&state.db)
            .await?
            .ok_or(ApiError::NotFound("service not found".into()))?;

    if matches!(
        service.status,
        vaultscope_db::models::ServiceStatus::Running
            | vaultscope_db::models::ServiceStatus::Suspended
            | vaultscope_db::models::ServiceStatus::Terminated
    ) {
        return Err(ApiError::BadRequest(
            "Cannot delete a running, suspended, or already terminated service.".into(),
        ));
    }

    sqlx::query(
        "UPDATE jobs SET status = 'cancelled'::job_status, completed_at = now() WHERE service_id = $1 AND status = 'queued'::job_status",
    )
    .bind(id)
    .execute(&state.db)
    .await?;

    if !service.provider_resource_id.is_empty() {
        let provider = get_service_provider(&state.db, &service).await;
        let job_id = Uuid::now_v7();
        let delete_payload = json!({
            "resource_id": service.provider_resource_id
        });

        sqlx::query(
            "INSERT INTO jobs (id, task, target_api, customer_id, service_id, request_payload) VALUES ($1, 'delete', $2, $3, $4, $5)"
        )
        .bind(job_id)
        .bind(provider)
        .bind(auth.0.sub)
        .bind(id)
        .bind(&delete_payload)
        .execute(&state.db)
        .await?;
    }

    sqlx::query("UPDATE services SET status = 'terminated' WHERE id = $1")
        .bind(id)
        .execute(&state.db)
        .await?;

    Ok(Json(json!({ "ok": true })))
}

#[derive(Deserialize, ToSchema)]
pub(crate) struct CancelServicePayload {
    mode: String,
    reason: Option<String>,
    feedback: Option<String>,
}

#[utoipa::path(
    post,
    path = "/{id}/cancel",
    tag = "Storefront Services",
    security(("bearer_auth" = []), ("csrf_token" = [])),
    params(("id" = Uuid, Path, description = "Service ID")),
    request_body = CancelServicePayload,
    responses(
        (status = 200, description = "Cancellation processed or scheduled", body = OkMessageResponse),
        (status = 400, description = "Service already terminated", body = ErrorResponse),
        (status = 404, description = "Service not found", body = ErrorResponse)
    )
)]
pub(crate) async fn cancel_service(
    auth: AuthCustomer,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(payload): Json<CancelServicePayload>,
) -> Result<Json<Value>, ApiError> {
    let service: Service =
        sqlx::query_as("SELECT * FROM services WHERE id = $1 AND customer_id = $2")
            .bind(id)
            .bind(auth.0.sub)
            .fetch_optional(&state.db)
            .await?
            .ok_or(ApiError::NotFound("service not found".into()))?;

    if service.status == vaultscope_db::models::ServiceStatus::Terminated {
        return Err(ApiError::BadRequest("Service is already terminated".into()));
    }
    if service.status == vaultscope_db::models::ServiceStatus::Suspended {
        return Err(ApiError::BadRequest(
            "Suspended services cannot be self-cancelled. Please contact support.".into(),
        ));
    }

    let is_immediate = payload.mode.eq_ignore_ascii_case("immediate");

    if is_immediate {
        sqlx::query(
            "UPDATE jobs SET status = 'cancelled'::job_status, completed_at = now() WHERE service_id = $1 AND status = 'queued'::job_status",
        )
        .bind(id)
        .execute(&state.db)
        .await?;

        if !service.provider_resource_id.is_empty() {
            let job_id = Uuid::now_v7();
            let delete_payload = json!({
                "resource_id": service.provider_resource_id
            });

            let provider: Option<(String,)> = sqlx::query_as(
                "SELECT provider FROM products WHERE id = $1"
            )
            .bind(service.product_id)
            .fetch_optional(&state.db)
            .await?;

            let target_api = provider.map(|p| p.0).unwrap_or_else(|| "sftpgo".to_string());

            sqlx::query(
                "INSERT INTO jobs (id, task, target_api, customer_id, service_id, request_payload) VALUES ($1, 'delete', $2, $3, $4, $5)"
            )
            .bind(job_id)
            .bind(target_api)
            .bind(auth.0.sub)
            .bind(id)
            .bind(&delete_payload)
            .execute(&state.db)
            .await?;
        }

        sqlx::query("UPDATE services SET status = 'terminated', cancellation_reason = $1, cancellation_feedback = $2, cancel_at_period_end = false WHERE id = $3")
            .bind(payload.reason.as_deref())
            .bind(payload.feedback.as_deref())
            .bind(id)
            .execute(&state.db)
            .await?;

        send_cancellation_email(&state, &auth.0.email, &service.name, "immediately").await;

        Ok(Json(json!({ "ok": true, "message": "Service cancelled and terminated immediately" })))
    } else {
        sqlx::query("UPDATE services SET cancel_at_period_end = true, cancellation_reason = $1, cancellation_feedback = $2 WHERE id = $3")
            .bind(payload.reason.as_deref())
            .bind(payload.feedback.as_deref())
            .bind(id)
            .execute(&state.db)
            .await?;

        let end_date = service.next_due
            .map(|d| d.format("%Y-%m-%d").to_string())
            .unwrap_or_else(|| "the end of the current billing cycle".to_string());

        send_cancellation_email(&state, &auth.0.email, &service.name, &format!("at the end of your billing cycle ({})", end_date)).await;

        Ok(Json(json!({ "ok": true, "message": "Service scheduled for cancellation at end of billing cycle" })))
    }
}

#[utoipa::path(
    post,
    path = "/{id}/uncancel",
    tag = "Storefront Services",
    security(("bearer_auth" = []), ("csrf_token" = [])),
    params(("id" = Uuid, Path, description = "Service ID")),
    responses(
        (status = 200, description = "Pending cancellation revoked", body = OkMessageResponse),
        (status = 400, description = "Cannot restore a terminated service", body = ErrorResponse),
        (status = 404, description = "Service not found", body = ErrorResponse)
    )
)]
pub(crate) async fn uncancel_service(
    auth: AuthCustomer,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, ApiError> {
    let service: Service =
        sqlx::query_as("SELECT * FROM services WHERE id = $1 AND customer_id = $2")
            .bind(id)
            .bind(auth.0.sub)
            .fetch_optional(&state.db)
            .await?
            .ok_or(ApiError::NotFound("service not found".into()))?;

    if service.status == vaultscope_db::models::ServiceStatus::Terminated {
        return Err(ApiError::BadRequest("Cannot restore a terminated service".into()));
    }

    sqlx::query("UPDATE services SET cancel_at_period_end = false, cancellation_reason = NULL, cancellation_feedback = NULL WHERE id = $1")
        .bind(id)
        .execute(&state.db)
        .await?;

    Ok(Json(json!({ "ok": true, "message": "Cancellation has been revoked" })))
}

async fn send_cancellation_email(state: &AppState, email_to: &str, service_name: &str, effective_date: &str) {
    let smtp_host = state.config.smtp_host.clone().unwrap_or_else(|| "mail.vaultscope.de".to_string());
    let smtp_user = state.config.smtp_username.clone().unwrap_or_else(|| "system@vaultscope.de".to_string());
    let smtp_pass = state.config.smtp_password.clone().unwrap_or_default();
    let smtp_from = state.config.smtp_from.clone().unwrap_or_else(|| "system@vaultscope.de".to_string());

    let subject = crate::mailer::sanitize_email_header(&format!("Cancellation Confirmation: {}", service_name));
    let body = format!(
        "Hello,\n\nWe have received your cancellation request for '{}'.\n\nEffective cancellation: {}\n\nIf this was done in error or you have any questions, you can undo this directly in your dashboard or contact our support team.\n\nBest regards,\nVaultScope Team",
        service_name, effective_date
    );

    if let (Ok(from_addr), Ok(to_addr)) = (smtp_from.parse(), email_to.parse()) {
        if let Ok(email) = lettre::Message::builder()
            .from(from_addr)
            .to(to_addr)
            .subject(subject)
            .body(body)
        {
            let creds = lettre::transport::smtp::authentication::Credentials::new(smtp_user, smtp_pass);
            if let Ok(mailer) = lettre::SmtpTransport::relay(&smtp_host) {
                let mailer = mailer.credentials(creds).build();
                let _ = tokio::task::spawn_blocking(move || lettre::Transport::send(&mailer, &email)).await;
            }
        }
    }
}

#[derive(Deserialize, Default, ToSchema)]
pub(crate) struct IpmiPayload {
    client_ip: Option<String>,
}

#[utoipa::path(
    post,
    path = "/{id}/ipmi",
    tag = "Storefront Services",
    security(("bearer_auth" = []), ("csrf_token" = [])),
    params(("id" = Uuid, Path, description = "Service ID")),
    request_body = Option<IpmiPayload>,
    responses(
        (status = 200, description = "IPMI / KVM session details", body = Value),
        (status = 404, description = "Service not found", body = ErrorResponse)
    )
)]
pub(crate) async fn ipmi_access(
    auth: AuthCustomer,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    payload: Option<Json<IpmiPayload>>,
) -> Result<Json<Value>, ApiError> {
    let service: Service =
        sqlx::query_as("SELECT * FROM services WHERE id = $1 AND customer_id = $2")
            .bind(id)
            .bind(auth.0.sub)
            .fetch_optional(&state.db)
            .await?
            .ok_or(ApiError::NotFound("service not found".into()))?;

    if service.status != vaultscope_db::models::ServiceStatus::Running {
        return Err(ApiError::BadRequest(
            "Service must be running to request IPMI access".into(),
        ));
    }

    let client_ip = crate::routes::admin::services::validate_optional_client_ip(
        payload.as_ref().and_then(|p| p.0.client_ip.as_deref()),
    )?;
    ipmi_access_inner(&state, &service, client_ip.as_deref()).await
}

#[utoipa::path(
    get,
    path = "/{id}/ipmi",
    tag = "Storefront Services",
    security(("bearer_auth" = [])),
    params(("id" = Uuid, Path, description = "Service ID")),
    responses(
        (status = 200, description = "IPMI / KVM session details", body = Value),
        (status = 404, description = "Service not found", body = ErrorResponse)
    )
)]
pub(crate) async fn ipmi_access_get(
    auth: AuthCustomer,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, ApiError> {
    ipmi_access(auth, State(state), Path(id), None).await
}

async fn ipmi_access_inner(
    state: &AppState,
    service: &Service,
    client_ip: Option<&str>,
) -> Result<Json<Value>, ApiError> {
    let resource_id = if service.provider_resource_id.is_empty() {
        format!("ds-{}", &service.id.to_string()[..8])
    } else {
        service.provider_resource_id.clone()
    };

    let rs = state.config.reliablesite_connector();

    let mut session = rs.request_ipmi_session(&resource_id, client_ip).await
        .map_err(|e| ApiError::Internal(format!("Failed to generate IPMI session: {e}")))?;

    if let Some(obj) = session.as_object_mut() {
        obj.insert(
            "provider".to_string(),
            Value::String("vaultscope_bmc".to_string()),
        );
    }

    Ok(Json(session))
}

#[utoipa::path(
    post,
    path = "/{id}/console",
    tag = "Storefront Services",
    security(("bearer_auth" = []), ("csrf_token" = [])),
    params(("id" = Uuid, Path, description = "Service ID")),
    responses(
        (status = 200, description = "VNC or IPMI console session payload", body = Value),
        (status = 400, description = "Service is not running or not provisioned", body = ErrorResponse),
        (status = 404, description = "Service not found", body = ErrorResponse)
    )
)]
pub(crate) async fn vnc_console(
    auth: AuthCustomer,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, ApiError> {
    let service: Service =
        sqlx::query_as("SELECT * FROM services WHERE id = $1 AND customer_id = $2")
            .bind(id)
            .bind(auth.0.sub)
            .fetch_optional(&state.db)
            .await?
            .ok_or(ApiError::NotFound("service not found".into()))?;

    if service.status != vaultscope_db::models::ServiceStatus::Running {
        return Err(ApiError::BadRequest("Service must be running to access the console".into()));
    }

    let provider = get_service_provider(&state.db, &service).await;
    if provider == "reliablesite" || service.name.to_lowercase().contains("dedicated") {
        return ipmi_access_inner(&state, &service, None).await;
    }

    if service.provider_resource_id.is_empty() {
        return Err(ApiError::BadRequest("Service is not yet provisioned".into()));
    }

    let token = state.config.hetzner_api_token.as_ref().ok_or_else(|| {
        ApiError::Internal("Hetzner provider not configured in system".into())
    })?;

    let hetzner = vaultscope_connectors::hetzner_cloud::HetznerCloud::new(token.trim().to_string());
    use vaultscope_connectors::ProvisioningProvider;

    let res = hetzner.console(&service.provider_resource_id).await;
    match res {
        Ok(info) => Ok(Json(info)),
        Err(e) => {
            if state.config.dev_mode && service.provider_resource_id.starts_with("srv-") {
                tracing::warn!("Hetzner console request failed for mock ID {}: {}. Returning simulated session.", service.provider_resource_id, e);
                Ok(Json(json!({
                    "wss_url": "wss://console.hetzner.cloud/?token=mock_session_token",
                    "password": "mock_password_dev_1234",
                    "is_simulated": true,
                    "server_name": service.name,
                })))
            } else {
                Err(ApiError::Internal(format!("Failed to request console from Hetzner: {e}")))
            }
        }
    }
}
