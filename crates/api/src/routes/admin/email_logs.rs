use axum::extract::{Path, State};
use axum::{Json, Router};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::auth::AuthAdmin;
use crate::error::ApiError;
use crate::state::AppState;
use vaultscope_db::models::{EmailDeliveryStatus, EmailLog};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/", axum::routing::get(list).post(create))
        .route("/send-test", axum::routing::post(send_test))
        .route(
            "/{id}",
            axum::routing::get(get_email_log).put(update).delete(delete),
        )
}

#[derive(Serialize)]
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

async fn list(
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

async fn get_email_log(
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

#[derive(Deserialize)]
struct SendTestPayload {
    to: String,
    subject: String,
    body: String,
}

async fn send_test(
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

#[derive(Deserialize)]
struct CreateEmailLog {
    recipient: String,
    subject: String,
    template_id: Option<Uuid>,
    status: EmailDeliveryStatus,
    error: Option<String>,
}

async fn create(
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

#[derive(Deserialize)]
struct UpdateEmailLog {
    recipient: Option<String>,
    subject: Option<String>,
    template_id: Option<Uuid>,
    status: Option<EmailDeliveryStatus>,
    error: Option<String>,
}

async fn update(
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

async fn delete(
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
