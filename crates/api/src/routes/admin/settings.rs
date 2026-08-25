use axum::extract::State;
use axum::routing::get;
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::auth::AuthAdmin;
use crate::error::ApiError;
use crate::state::AppState;
use vaultscope_db::models::Setting;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/", get(list).put(update_batch))
}

async fn list(
    _auth: AuthAdmin,
    State(state): State<AppState>,
) -> Result<Json<Vec<Setting>>, ApiError> {
    let settings: Vec<Setting> = sqlx::query_as("SELECT * FROM settings ORDER BY key")
        .fetch_all(&state.db)
        .await?;
    Ok(Json(settings))
}

#[derive(Deserialize)]
struct SettingEntry {
    key: String,
    value: Value,
}

async fn update_batch(
    _auth: AuthAdmin,
    State(state): State<AppState>,
    Json(entries): Json<Vec<SettingEntry>>,
) -> Result<Json<Value>, ApiError> {
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
