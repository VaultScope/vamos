use axum::extract::{Path, State};
use axum::routing::get;
use axum::{Json, Router};
use serde::Deserialize;
use uuid::Uuid;

use crate::auth::AuthAdmin;
use crate::error::ApiError;
use crate::state::AppState;
use vaultscope_db::models::InternalTool;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/", get(list).post(create))
        .route("/{id}", get(get_one).put(update).delete(delete))
}

async fn list(
    auth: AuthAdmin,
    State(state): State<AppState>,
) -> Result<Json<Vec<InternalTool>>, ApiError> {
    // Basic auth check
    crate::require_permission!(auth, state, "internal_tools.view");
    let tools: Vec<InternalTool> = sqlx::query_as("SELECT * FROM internal_tools ORDER BY name ASC")
        .fetch_all(&state.db)
        .await?;
    Ok(Json(tools))
}

async fn get_one(
    auth: AuthAdmin,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<InternalTool>, ApiError> {
    crate::require_permission!(auth, state, "internal_tools.view");
    let tool: InternalTool = sqlx::query_as("SELECT * FROM internal_tools WHERE id = $1")
        .bind(id)
        .fetch_optional(&state.db)
        .await?
        .ok_or(ApiError::NotFound("Internal tool not found".into()))?;
    Ok(Json(tool))
}

#[derive(Deserialize)]
struct CreateInternalTool {
    name: String,
    url: String,
    required_permission: String,
}

async fn create(
    auth: AuthAdmin,
    State(state): State<AppState>,
    Json(payload): Json<CreateInternalTool>,
) -> Result<Json<InternalTool>, ApiError> {
    crate::require_permission!(auth, state, "internal_tools.manage");
    let tool: InternalTool = sqlx::query_as(
        r#"INSERT INTO internal_tools (name, url, required_permission)
           VALUES ($1, $2, $3)
           RETURNING *"#,
    )
    .bind(payload.name)
    .bind(payload.url)
    .bind(payload.required_permission)
    .fetch_one(&state.db)
    .await?;

    Ok(Json(tool))
}

#[derive(Deserialize)]
struct UpdateInternalTool {
    name: Option<String>,
    url: Option<String>,
    required_permission: Option<String>,
}

async fn update(
    auth: AuthAdmin,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(payload): Json<UpdateInternalTool>,
) -> Result<Json<InternalTool>, ApiError> {
    crate::require_permission!(auth, state, "internal_tools.manage");
    let tool: InternalTool = sqlx::query_as(
        r#"UPDATE internal_tools
           SET name = COALESCE($1, name),
               url = COALESCE($2, url),
               required_permission = COALESCE($3, required_permission)
           WHERE id = $4
           RETURNING *"#,
    )
    .bind(payload.name)
    .bind(payload.url)
    .bind(payload.required_permission)
    .bind(id)
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound("Internal tool not found".into()))?;

    Ok(Json(tool))
}

async fn delete(
    auth: AuthAdmin,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<()>, ApiError> {
    crate::require_permission!(auth, state, "internal_tools.manage");
    let result = sqlx::query("DELETE FROM internal_tools WHERE id = $1")
        .bind(id)
        .execute(&state.db)
        .await?;

    if result.rows_affected() == 0 {
        return Err(ApiError::NotFound("Internal tool not found".into()));
    }

    Ok(Json(()))
}
