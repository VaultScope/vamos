use axum::extract::{Path, State};
use axum::routing::{get, put};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::auth::AuthAdmin;
use crate::error::ApiError;
use crate::state::AppState;
use vaultscope_db::models::EmailTemplate;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/", get(list))
        .route("/{id}", get(get_one))
        .route("/{id}", put(update))
}

async fn list(
    _auth: AuthAdmin,
    State(state): State<AppState>,
) -> Result<Json<Vec<EmailTemplate>>, ApiError> {
    let templates: Vec<EmailTemplate> =
        sqlx::query_as("SELECT * FROM email_templates ORDER BY category, name")
            .fetch_all(&state.db)
            .await?;
    Ok(Json(templates))
}

async fn get_one(
    _auth: AuthAdmin,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<EmailTemplate>, ApiError> {
    let template: EmailTemplate = sqlx::query_as("SELECT * FROM email_templates WHERE id = $1")
        .bind(id)
        .fetch_optional(&state.db)
        .await?
        .ok_or(ApiError::NotFound("template not found".into()))?;
    Ok(Json(template))
}

#[derive(Deserialize)]
struct UpdateTemplate {
    subject: Option<String>,
    body: Option<String>,
    enabled: Option<bool>,
}

async fn update(
    _auth: AuthAdmin,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(payload): Json<UpdateTemplate>,
) -> Result<Json<Value>, ApiError> {
    if let Some(subject) = &payload.subject {
        sqlx::query("UPDATE email_templates SET subject = $1, updated_at = now() WHERE id = $2")
            .bind(subject)
            .bind(id)
            .execute(&state.db)
            .await?;
    }
    if let Some(body) = &payload.body {
        sqlx::query("UPDATE email_templates SET body = $1, updated_at = now() WHERE id = $2")
            .bind(body)
            .bind(id)
            .execute(&state.db)
            .await?;
    }
    if let Some(enabled) = payload.enabled {
        sqlx::query("UPDATE email_templates SET enabled = $1, updated_at = now() WHERE id = $2")
            .bind(enabled)
            .bind(id)
            .execute(&state.db)
            .await?;
    }
    Ok(Json(json!({ "ok": true })))
}
