use axum::extract::State;
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};
use utoipa::ToSchema;
use utoipa_axum::{router::OpenApiRouter, routes};

use crate::auth::AuthAdmin;
use crate::error::ApiError;
use crate::openapi::OkResponse;
use crate::state::AppState;
use vaultscope_db::models::Setting;

pub fn routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new().routes(routes!(list, update_batch))
}

#[utoipa::path(
    get,
    path = "/",
    tag = "Admin Settings",
    security(("bearer_auth" = [])),
    responses((status = 200, description = "List platform settings", body = Vec<Setting>))
)]
pub(crate) async fn list(
    auth: AuthAdmin,
    State(state): State<AppState>,
) -> Result<Json<Vec<Setting>>, ApiError> {
    crate::require_permission!(auth, state, "settings.manage");
    let settings: Vec<Setting> = sqlx::query_as("SELECT * FROM settings ORDER BY key")
        .fetch_all(&state.db)
        .await?;
    Ok(Json(settings))
}

#[derive(Deserialize, ToSchema)]
pub(crate) struct SettingEntry {
    key: String,
    value: Value,
}

#[utoipa::path(
    put,
    path = "/",
    tag = "Admin Settings",
    security(("bearer_auth" = []), ("csrf_token" = [])),
    request_body = Vec<SettingEntry>,
    responses((status = 200, description = "Settings updated", body = OkResponse))
)]
pub(crate) async fn update_batch(
    auth: AuthAdmin,
    State(state): State<AppState>,
    Json(entries): Json<Vec<SettingEntry>>,
) -> Result<Json<Value>, ApiError> {
    crate::require_permission!(auth, state, "settings.manage");
    for entry in entries {
        sqlx::query(
            "INSERT INTO settings (key, value, updated_at) VALUES ($1, $2, now()) ON CONFLICT (key) DO UPDATE SET value = $2, updated_at = now()"
        )
        .bind(&entry.key)
        .bind(&entry.value)
        .execute(&state.db)
        .await?;
    }

    Ok(Json(json!({ "ok": true })))
}
