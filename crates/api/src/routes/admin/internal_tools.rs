use axum::extract::{Path, State};
use axum::Json;
use serde::Deserialize;
use utoipa::ToSchema;
use utoipa_axum::{router::OpenApiRouter, routes};
use uuid::Uuid;

use crate::auth::AuthAdmin;
use crate::error::ApiError;
use crate::openapi::ErrorResponse;
use crate::state::AppState;
use crate::validation::validate_required_string;
use vaultscope_db::models::InternalTool;

pub fn validate_tool_url(raw: &str) -> Result<String, ApiError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed.len() > 2048 {
        return Err(ApiError::BadRequest(
            "Tool URL must be between 1 and 2048 characters".into(),
        ));
    }
    if trimmed.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err(ApiError::BadRequest(
            "Tool URL must not contain whitespace or control characters".into(),
        ));
    }
    let lower = trimmed.to_ascii_lowercase();
    if !lower.starts_with("https://") && !lower.starts_with("http://") {
        return Err(ApiError::BadRequest(
            "Tool URL must use http:// or https:// scheme".into(),
        ));
    }
    let parsed = reqwest::Url::parse(trimmed)
        .map_err(|_| ApiError::BadRequest("Invalid tool URL format".into()))?;
    if !matches!(parsed.scheme(), "http" | "https")
        || !parsed.has_host()
        || parsed.host_str().unwrap_or("").is_empty()
    {
        return Err(ApiError::BadRequest("Invalid tool URL host".into()));
    }
    Ok(trimmed.to_string())
}

pub fn routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(list, create))
        .routes(routes!(get_one, update, delete))
}

#[utoipa::path(
    get,
    path = "/",
    tag = "Admin Internal Tools",
    security(("bearer_auth" = [])),
    responses((status = 200, description = "List internal admin links/tools", body = Vec<InternalTool>))
)]
pub(crate) async fn list(
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

#[utoipa::path(
    get,
    path = "/{id}",
    tag = "Admin Internal Tools",
    security(("bearer_auth" = [])),
    params(("id" = Uuid, Path, description = "Internal tool ID")),
    responses(
        (status = 200, description = "Internal tool details", body = InternalTool),
        (status = 404, description = "Internal tool not found", body = ErrorResponse)
    )
)]
pub(crate) async fn get_one(
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

#[derive(Deserialize, ToSchema)]
pub(crate) struct CreateInternalTool {
    name: String,
    url: String,
    required_permission: String,
}

#[utoipa::path(
    post,
    path = "/",
    tag = "Admin Internal Tools",
    security(("bearer_auth" = []), ("csrf_token" = [])),
    request_body = CreateInternalTool,
    responses((status = 200, description = "Created internal tool", body = InternalTool))
)]
pub(crate) async fn create(
    auth: AuthAdmin,
    State(state): State<AppState>,
    Json(payload): Json<CreateInternalTool>,
) -> Result<Json<InternalTool>, ApiError> {
    crate::require_permission!(auth, state, "internal_tools.manage");
    validate_required_string(&payload.name, "name")?;
    let valid_url = validate_tool_url(&payload.url)?;

    let tool: InternalTool = sqlx::query_as(
        r#"INSERT INTO internal_tools (name, url, required_permission)
           VALUES ($1, $2, $3)
           RETURNING *"#,
    )
    .bind(payload.name.trim())
    .bind(valid_url)
    .bind(payload.required_permission.trim())
    .fetch_one(&state.db)
    .await?;

    Ok(Json(tool))
}

#[derive(Deserialize, ToSchema)]
pub(crate) struct UpdateInternalTool {
    name: Option<String>,
    url: Option<String>,
    required_permission: Option<String>,
}

#[utoipa::path(
    put,
    path = "/{id}",
    tag = "Admin Internal Tools",
    security(("bearer_auth" = []), ("csrf_token" = [])),
    params(("id" = Uuid, Path, description = "Internal tool ID")),
    request_body = UpdateInternalTool,
    responses(
        (status = 200, description = "Updated internal tool", body = InternalTool),
        (status = 404, description = "Internal tool not found", body = ErrorResponse)
    )
)]
pub(crate) async fn update(
    auth: AuthAdmin,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(payload): Json<UpdateInternalTool>,
) -> Result<Json<InternalTool>, ApiError> {
    crate::require_permission!(auth, state, "internal_tools.manage");
    if let Some(ref name) = payload.name {
        validate_required_string(name, "name")?;
    }
    let valid_url = match payload.url {
        Some(ref u) => Some(validate_tool_url(u)?),
        None => None,
    };

    let tool: InternalTool = sqlx::query_as(
        r#"UPDATE internal_tools
           SET name = COALESCE($1, name),
               url = COALESCE($2, url),
               required_permission = COALESCE($3, required_permission)
           WHERE id = $4
           RETURNING *"#,
    )
    .bind(payload.name.map(|s| s.trim().to_string()))
    .bind(valid_url)
    .bind(payload.required_permission.map(|s| s.trim().to_string()))
    .bind(id)
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound("Internal tool not found".into()))?;

    Ok(Json(tool))
}

#[utoipa::path(
    delete,
    path = "/{id}",
    tag = "Admin Internal Tools",
    security(("bearer_auth" = []), ("csrf_token" = [])),
    params(("id" = Uuid, Path, description = "Internal tool ID")),
    responses(
        (status = 200, description = "Internal tool deleted"),
        (status = 404, description = "Internal tool not found", body = ErrorResponse)
    )
)]
pub(crate) async fn delete(
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
