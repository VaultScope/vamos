use axum::extract::{Path, State};
use axum::Json;
use serde_json::{json, Value};
use utoipa_axum::{router::OpenApiRouter, routes};
use uuid::Uuid;

use crate::auth::AuthAdmin;
use crate::error::ApiError;
use crate::openapi::{ErrorResponse, OkResponse};
use crate::state::AppState;
use vaultscope_db::models::Notification;

pub fn routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(list))
        .routes(routes!(mark_read))
        .routes(routes!(resolve))
}

#[utoipa::path(
    get,
    path = "/",
    tag = "Admin Notifications",
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "Admin notifications", body = Vec<Notification>),
        (status = 403, description = "Forbidden", body = ErrorResponse)
    )
)]
pub(crate) async fn list(
    auth: AuthAdmin,
    State(state): State<AppState>,
) -> Result<Json<Vec<Notification>>, ApiError> {
    crate::require_permission!(auth, state, "notifications.view");
    let notifications: Vec<Notification> =
        sqlx::query_as("SELECT * FROM notifications ORDER BY created_at DESC LIMIT 100")
            .fetch_all(&state.db)
            .await?;
    Ok(Json(notifications))
}

#[utoipa::path(
    put,
    path = "/{id}/read",
    tag = "Admin Notifications",
    security(("bearer_auth" = []), ("csrf_token" = [])),
    params(("id" = Uuid, Path, description = "Notification ID")),
    responses((status = 200, description = "Notification marked read", body = OkResponse))
)]
pub(crate) async fn mark_read(
    auth: AuthAdmin,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, ApiError> {
    crate::require_permission!(auth, state, "notifications.manage");
    sqlx::query("UPDATE notifications SET read = true WHERE id = $1")
        .bind(id)
        .execute(&state.db)
        .await?;
    Ok(Json(json!({ "ok": true })))
}

#[utoipa::path(
    put,
    path = "/{id}/resolve",
    tag = "Admin Notifications",
    security(("bearer_auth" = []), ("csrf_token" = [])),
    params(("id" = Uuid, Path, description = "Notification ID")),
    responses((status = 200, description = "Notification resolved", body = OkResponse))
)]
pub(crate) async fn resolve(
    auth: AuthAdmin,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, ApiError> {
    crate::require_permission!(auth, state, "notifications.manage");
    sqlx::query("UPDATE notifications SET resolved = true, read = true WHERE id = $1")
        .bind(id)
        .execute(&state.db)
        .await?;
    Ok(Json(json!({ "ok": true })))
}
