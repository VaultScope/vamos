use axum::extract::{Path, State};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::auth::AuthCustomer;
use crate::error::ApiError;
use crate::state::AppState;
use vaultscope_common::types::PowerAction;
use vaultscope_db::models::Service;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/", get(list_mine))
        .route("/{id}", get(get_one).delete(delete_service))
        .route("/{id}/start", post(power_start))
        .route("/{id}/stop", post(power_stop))
        .route("/{id}/restart", post(power_restart))
        .route("/{id}/reinstall", post(reinstall))
        .route("/{id}/console", post(vnc_console))
        .route("/{id}/ipmi", post(ipmi_access).get(ipmi_access))
        .route("/{id}/cancel", post(cancel_service))
        .route("/{id}/uncancel", post(uncancel_service))
}

async fn list_mine(
    auth: AuthCustomer,
    State(state): State<AppState>,
) -> Result<Json<Vec<Service>>, ApiError> {
    let services: Vec<Service> =
        sqlx::query_as("SELECT * FROM services WHERE customer_id = $1 ORDER BY created_at DESC")
            .bind(auth.0.sub)
            .fetch_all(&state.db)
            .await?;
    Ok(Json(services))
}

async fn get_one(
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
    Ok(Json(service))
}

async fn power_start(
    auth: AuthCustomer,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, ApiError> {
    power_action_helper(auth, state, id, PowerAction::Start).await
}

async fn power_stop(
    auth: AuthCustomer,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, ApiError> {
    power_action_helper(auth, state, id, PowerAction::Stop).await
}

async fn power_restart(
    auth: AuthCustomer,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, ApiError> {
    power_action_helper(auth, state, id, PowerAction::Restart).await
}

async fn get_service_provider(db: &sqlx::PgPool, service: &Service) -> String {
    if let Some(p) = service.config.get("provider").and_then(|v| v.as_str()) {
        return p.to_string();
    }
    let prod_provider: Option<(String,)> = sqlx::query_as(
        "SELECT provider FROM products WHERE id = $1"
    )
    .bind(service.product_id)
    .fetch_optional(db)
    .await
    .unwrap_or(None);

    prod_provider.map(|p| p.0).unwrap_or_else(|| {
        if service.name.to_lowercase().contains("dedicated") {
            "reliablesite".to_string()
        } else {
            "hetzner_cloud".to_string()
        }
    })
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

#[derive(Deserialize)]
struct ReinstallPayload {
    os: String,
}

async fn reinstall(
    auth: AuthCustomer,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(payload): Json<ReinstallPayload>,
) -> Result<Json<Value>, ApiError> {
    let service: Service =
        sqlx::query_as("SELECT * FROM services WHERE id = $1 AND customer_id = $2")
            .bind(id)
            .bind(auth.0.sub)
            .fetch_optional(&state.db)
            .await?
            .ok_or(ApiError::NotFound("service not found".into()))?;

    if service.provider_resource_id.is_empty() {
        return Err(ApiError::BadRequest("service not yet provisioned".into()));
    }

    let provider = get_service_provider(&state.db, &service).await;
    let job_id = Uuid::now_v7();
    let reinstall_payload = json!({
        "resource_id": service.provider_resource_id,
        "params": {
            "image": payload.os
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

async fn delete_service(
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

    if service.status == vaultscope_db::models::ServiceStatus::Running {
        return Err(ApiError::BadRequest(
            "Cannot delete a running service. Please stop or suspend it first.".into(),
        ));
    }

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

#[derive(Deserialize)]
struct CancelServicePayload {
    mode: String,
    reason: Option<String>,
    feedback: Option<String>,
}

async fn cancel_service(
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

    let is_immediate = payload.mode.eq_ignore_ascii_case("immediate");

    if is_immediate {
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

async fn uncancel_service(
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
    let smtp_pass = state.config.smtp_password.clone().unwrap_or_else(|| "".to_string());
    let smtp_from = state.config.smtp_from.clone().unwrap_or_else(|| "system@vaultscope.de".to_string());

    let subject = format!("Cancellation Confirmation: {}", service_name);
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
                let _ = lettre::Transport::send(&mailer, &email);
            }
        }
    }
}

#[derive(Deserialize, Default)]
struct IpmiPayload {
    client_ip: Option<String>,
}

async fn ipmi_access(
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

    let client_ip = payload.and_then(|p| p.0.client_ip);
    ipmi_access_inner(&state, &service, client_ip.as_deref()).await
}

async fn ipmi_access_inner(
    state: &AppState,
    service: &Service,
    client_ip: Option<&str>,
) -> Result<Json<Value>, ApiError> {
    let resource_id = if service.provider_resource_id.is_empty() {
        format!("rs-ds-{}", &service.id.to_string()[..8])
    } else {
        service.provider_resource_id.clone()
    };

    let api_key = state.config.reliablesite_api_key.as_deref().unwrap_or("").trim().to_string();
    let base_url = state.config.reliablesite_api_url.as_deref().unwrap_or("https://api.reliablesite.net").trim().to_string();
    let rs = vaultscope_connectors::reliablesite::ReliableSite::new(api_key, base_url);

    let session = rs.request_ipmi_session(&resource_id, client_ip).await
        .map_err(|e| ApiError::Internal(format!("Failed to generate IPMI session: {e}")))?;

    Ok(Json(session))
}

async fn vnc_console(
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
            if service.provider_resource_id.starts_with("srv-") {
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
