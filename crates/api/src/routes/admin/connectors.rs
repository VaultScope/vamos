use axum::extract::{Path, State};
use axum::routing::{get, post, put};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::auth::AuthAdmin;
use crate::error::ApiError;
use crate::state::AppState;
use vaultscope_db::models::Connector;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/", get(list))
        .route("/{id}", get(get_one))
        .route("/{id}", put(update))
        .route("/{id}/test", post(test_connection))
}

async fn list(
    _auth: AuthAdmin,
    State(state): State<AppState>,
) -> Result<Json<Vec<Connector>>, ApiError> {
    let connectors: Vec<Connector> = sqlx::query_as("SELECT * FROM connectors ORDER BY name")
        .fetch_all(&state.db)
        .await?;
    Ok(Json(connectors))
}

async fn get_one(
    _auth: AuthAdmin,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<Connector>, ApiError> {
    let connector: Connector = sqlx::query_as("SELECT * FROM connectors WHERE id = $1")
        .bind(id)
        .fetch_optional(&state.db)
        .await?
        .ok_or(ApiError::NotFound("connector not found".into()))?;
    Ok(Json(connector))
}

#[derive(Deserialize)]
struct UpdateConnector {
    config: Value,
}

async fn update(
    _auth: AuthAdmin,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(payload): Json<UpdateConnector>,
) -> Result<Json<Value>, ApiError> {
    let config_bytes = serde_json::to_vec(&payload.config)
        .map_err(|e| ApiError::BadRequest(e.to_string()))?;

    let key = state.config.encryption_key_bytes();
    let encrypted = vaultscope_common::crypto::encrypt(&key, &config_bytes)
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    sqlx::query("UPDATE connectors SET config_encrypted = $1, status = 'connected', updated_at = now() WHERE id = $2")
        .bind(&encrypted)
        .bind(id)
        .execute(&state.db)
        .await?;

    Ok(Json(json!({ "ok": true })))
}

async fn test_connection(
    _auth: AuthAdmin,
    State(_state): State<AppState>,
    Path(_id): Path<Uuid>,
) -> Result<Json<Value>, ApiError> {
    // TODO: resolve connector, decrypt config, instantiate provider, call test_connection
    Ok(Json(json!({ "ok": true, "message": "connection test not yet implemented" })))
}
