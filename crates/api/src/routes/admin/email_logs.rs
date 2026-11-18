use axum::extract::{Path, State};
use axum::Json;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use utoipa_axum::{router::OpenApiRouter, routes};
use uuid::Uuid;

use crate::auth::AuthAdmin;
use crate::error::ApiError;
use crate::openapi::ErrorResponse;
use crate::state::AppState;
use vaultscope_db::models::{EmailDeliveryStatus, EmailLog};

pub fn routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(list, create))
        .routes(routes!(send_test))
        .routes(routes!(get_email_log, update, delete))
}

#[derive(Serialize, ToSchema)]
pub struct EmailLogResponse {
    pub id: Uuid,
    pub recipient: String,
    pub to: String,
    pub subject: String,
    pub template_id: Option<Uuid>,
    pub status: String,
    pub raw_status: EmailDeliveryStatus,
    pub error: Option<String>,
    pub mailbox: String,
    pub sent_at: DateTime<Utc>,
    pub time: String,
}

fn to_response(log: EmailLog) -> EmailLogResponse {
    let status_str = match log.status {
        EmailDeliveryStatus::Delivered => "Delivered",
        EmailDeliveryStatus::Failed => "Failed",
        EmailDeliveryStatus::Queued => "Queued",
        EmailDeliveryStatus::Bounced => "Bounced",
    }
    .to_string();

    let mailbox = if log.subject.to_lowercase().contains("support")
        || log.subject.to_lowercase().contains("ticket")
    {
        "support@vaultscope.de".to_string()
    } else if log.subject.to_lowercase().contains("abuse") {
        "abuse@vaultscope.de".to_string()
    } else {
        "noreply@vaultscope.de".to_string()
    };

    let time = log.sent_at.format("%Y-%m-%d %H:%M").to_string();

    EmailLogResponse {
        id: log.id,
        recipient: log.recipient.clone(),
        to: log.recipient,
        subject: log.subject,
        template_id: log.template_id,
        status: status_str,
        raw_status: log.status,
        error: log.error,
        mailbox,
        sent_at: log.sent_at,
        time,
    }
}

#[utoipa::path(
    get,
    path = "/",
    tag = "Admin Email Logs",
    security(("bearer_auth" = [])),
    responses((status = 200, description = "List email delivery logs", body = Vec<EmailLogResponse>))
)]
pub(crate) async fn list(
    auth: AuthAdmin,
    State(state): State<AppState>,
) -> Result<Json<Vec<EmailLogResponse>>, ApiError> {
    if crate::middleware::rbac::check_permission(&auth, &state, "email_logs.view").await.is_err() {
        crate::require_permission!(auth, state, "settings.manage");
    }

    let logs: Vec<EmailLog> = sqlx::query_as("SELECT * FROM email_log ORDER BY sent_at DESC LIMIT 500")
        .fetch_all(&state.db)
        .await?;

    let resps = logs.into_iter().map(to_response).collect();
    Ok(Json(resps))
}

#[utoipa::path(
    get,
    path = "/{id}",
    tag = "Admin Email Logs",
    security(("bearer_auth" = [])),
    params(("id" = Uuid, Path, description = "Email log ID")),
    responses(
        (status = 200, description = "Email log details", body = EmailLogResponse),
        (status = 404, description = "Email log not found", body = ErrorResponse)
    )
)]
pub(crate) async fn get_email_log(
    auth: AuthAdmin,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<EmailLogResponse>, ApiError> {
    if crate::middleware::rbac::check_permission(&auth, &state, "email_logs.view").await.is_err() {
        crate::require_permission!(auth, state, "settings.manage");
    }

    let log: EmailLog = sqlx::query_as("SELECT * FROM email_log WHERE id = $1")
        .bind(id)
        .fetch_optional(&state.db)
        .await?
        .ok_or(ApiError::NotFound("email log not found".into()))?;

    Ok(Json(to_response(log)))
}

#[derive(Deserialize, ToSchema)]
pub(crate) struct SendTestPayload {
    to: String,
    subject: String,
    body: String,
}

#[utoipa::path(
    post,
    path = "/send-test",
    tag = "Admin Email Logs",
    security(("bearer_auth" = []), ("csrf_token" = [])),
    request_body = SendTestPayload,
    responses((status = 200, description = "Test email sent and logged", body = EmailLogResponse))
)]
pub(crate) async fn send_test(
    auth: AuthAdmin,
    State(state): State<AppState>,
    Json(payload): Json<SendTestPayload>,
) -> Result<Json<EmailLogResponse>, ApiError> {
    if crate::middleware::rbac::check_permission(&auth, &state, "email_logs.manage").await.is_err() {
        crate::require_permission!(auth, state, "settings.manage");
    }

    let to = payload.to.trim();
    let subject = payload.subject.trim();
    let body = payload.body.trim();

    let _ = crate::mailer::send_email(&state, to, subject, body, None).await;

    let log: EmailLog = sqlx::query_as(
        "SELECT * FROM email_log WHERE recipient = $1 ORDER BY sent_at DESC LIMIT 1",
    )
    .bind(to)
    .fetch_one(&state.db)
    .await?;

    Ok(Json(to_response(log)))
}

#[derive(Deserialize, ToSchema)]
pub(crate) struct CreateEmailLog {
    recipient: String,
    subject: String,
    template_id: Option<Uuid>,
    status: EmailDeliveryStatus,
    error: Option<String>,
}

#[utoipa::path(
    post,
    path = "/",
    tag = "Admin Email Logs",
    security(("bearer_auth" = []), ("csrf_token" = [])),
    request_body = CreateEmailLog,
    responses((status = 200, description = "Email log entry created", body = EmailLogResponse))
)]
pub(crate) async fn create(
    auth: AuthAdmin,
    State(state): State<AppState>,
    Json(payload): Json<CreateEmailLog>,
) -> Result<Json<EmailLogResponse>, ApiError> {
    if crate::middleware::rbac::check_permission(&auth, &state, "email_logs.manage").await.is_err() {
        crate::require_permission!(auth, state, "settings.manage");
    }

    let log: EmailLog = sqlx::query_as(
        r#"INSERT INTO email_log (id, recipient, subject, template_id, status, error, sent_at)
           VALUES ($1, $2, $3, $4, $5, $6, $7)
           RETURNING *"#,
    )
    .bind(Uuid::now_v7())
    .bind(&payload.recipient)
    .bind(&payload.subject)
    .bind(payload.template_id)
    .bind(&payload.status)
    .bind(&payload.error)
    .bind(Utc::now())
    .fetch_one(&state.db)
    .await?;

    Ok(Json(to_response(log)))
}

#[derive(Deserialize, ToSchema)]
pub(crate) struct UpdateEmailLog {
    recipient: Option<String>,
    subject: Option<String>,
    template_id: Option<Uuid>,
    status: Option<EmailDeliveryStatus>,
    error: Option<String>,
}

#[utoipa::path(
    put,
    path = "/{id}",
    tag = "Admin Email Logs",
    security(("bearer_auth" = []), ("csrf_token" = [])),
    params(("id" = Uuid, Path, description = "Email log ID")),
    request_body = UpdateEmailLog,
    responses(
        (status = 200, description = "Email log updated", body = EmailLogResponse),
        (status = 404, description = "Email log not found", body = ErrorResponse)
    )
)]
pub(crate) async fn update(
    auth: AuthAdmin,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(payload): Json<UpdateEmailLog>,
) -> Result<Json<EmailLogResponse>, ApiError> {
    if crate::middleware::rbac::check_permission(&auth, &state, "email_logs.manage").await.is_err() {
        crate::require_permission!(auth, state, "settings.manage");
    }

    let log: EmailLog = sqlx::query_as(
        r#"UPDATE email_log
           SET recipient = COALESCE($1, recipient),
               subject = COALESCE($2, subject),
               template_id = COALESCE($3, template_id),
               status = COALESCE($4, status),
               error = COALESCE($5, error)
           WHERE id = $6
           RETURNING *"#,
    )
    .bind(payload.recipient)
    .bind(payload.subject)
    .bind(payload.template_id)
    .bind(payload.status)
    .bind(payload.error)
    .bind(id)
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound("email log not found".into()))?;

    Ok(Json(to_response(log)))
}

#[utoipa::path(
    delete,
    path = "/{id}",
    tag = "Admin Email Logs",
    security(("bearer_auth" = []), ("csrf_token" = [])),
    params(("id" = Uuid, Path, description = "Email log ID")),
    responses(
        (status = 200, description = "Email log deleted"),
        (status = 404, description = "Email log not found", body = ErrorResponse)
    )
)]
pub(crate) async fn delete(
    auth: AuthAdmin,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<()>, ApiError> {
    if crate::middleware::rbac::check_permission(&auth, &state, "email_logs.manage").await.is_err() {
        crate::require_permission!(auth, state, "settings.manage");
    }

    let result = sqlx::query("DELETE FROM email_log WHERE id = $1")
        .bind(id)
        .execute(&state.db)
        .await?;

    if result.rows_affected() == 0 {
        return Err(ApiError::NotFound("email log not found".into()));
    }
    Ok(Json(()))
}
