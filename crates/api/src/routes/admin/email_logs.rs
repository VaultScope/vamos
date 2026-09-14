use axum::extract::{Path, State};
use axum::{Json, Router};
use chrono::Utc;
use serde::Deserialize;
use uuid::Uuid;

use crate::auth::AuthAdmin;
use crate::error::ApiError;
use crate::state::AppState;
use vaultscope_db::models::{EmailDeliveryStatus, EmailLog};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/", axum::routing::get(list).post(create))
        .route(
            "/{id}",
            axum::routing::get(get_email_log).put(update).delete(delete),
        )
}

async fn list(
    auth: AuthAdmin,
    State(state): State<AppState>,
) -> Result<Json<Vec<EmailLog>>, ApiError> {
    crate::require_permission!(auth, state, "email_logs.view");
    let logs: Vec<EmailLog> = sqlx::query_as("SELECT * FROM email_log ORDER BY sent_at DESC")
        .fetch_all(&state.db)
        .await?;
    Ok(Json(logs))
}

async fn get_email_log(
    auth: AuthAdmin,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<EmailLog>, ApiError> {
    crate::require_permission!(auth, state, "email_logs.view");
    let log: EmailLog = sqlx::query_as("SELECT * FROM email_log WHERE id = $1")
        .bind(id)
        .fetch_optional(&state.db)
        .await?
        .ok_or(ApiError::NotFound("email log not found".into()))?;
    Ok(Json(log))
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
) -> Result<Json<EmailLog>, ApiError> {
    crate::require_permission!(auth, state, "email_logs.manage");
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

    Ok(Json(log))
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
) -> Result<Json<EmailLog>, ApiError> {
    crate::require_permission!(auth, state, "email_logs.manage");
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

    Ok(Json(log))
}

async fn delete(
    auth: AuthAdmin,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<()>, ApiError> {
    crate::require_permission!(auth, state, "email_logs.manage");
    let result = sqlx::query("DELETE FROM email_log WHERE id = $1")
        .bind(id)
        .execute(&state.db)
        .await?;

    if result.rows_affected() == 0 {
        return Err(ApiError::NotFound("email log not found".into()));
    }
    Ok(Json(()))
}
