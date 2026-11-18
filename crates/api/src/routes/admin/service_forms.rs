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
use vaultscope_db::models::ServiceForm;

pub fn routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(list, create))
        .routes(routes!(get_one, update, delete))
}

#[utoipa::path(
    get,
    path = "/",
    tag = "Admin Service Forms",
    security(("bearer_auth" = [])),
    responses((status = 200, description = "List service forms", body = Vec<ServiceForm>))
)]
pub(crate) async fn list(
    auth: AuthAdmin,
    State(state): State<AppState>,
) -> Result<Json<Vec<ServiceForm>>, ApiError> {
    crate::require_permission!(auth, state, "forms.manage");
    let forms: Vec<ServiceForm> =
        sqlx::query_as("SELECT * FROM service_forms ORDER BY created_at DESC")
            .fetch_all(&state.db)
            .await?;
    Ok(Json(forms))
}

#[utoipa::path(
    get,
    path = "/{id}",
    tag = "Admin Service Forms",
    security(("bearer_auth" = [])),
    params(("id" = Uuid, Path, description = "Service form ID")),
    responses(
        (status = 200, description = "Service form details", body = ServiceForm),
        (status = 404, description = "Service form not found", body = ErrorResponse)
    )
)]
pub(crate) async fn get_one(
    auth: AuthAdmin,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<ServiceForm>, ApiError> {
    crate::require_permission!(auth, state, "forms.manage");
    let form: ServiceForm = sqlx::query_as("SELECT * FROM service_forms WHERE id = $1")
        .bind(id)
        .fetch_optional(&state.db)
        .await?
        .ok_or(ApiError::NotFound("service form not found".into()))?;
    Ok(Json(form))
}

#[derive(Deserialize, ToSchema)]
pub(crate) struct CreateServiceForm {
    name: String,
    description: String,
    fields: serde_json::Value,
}

#[utoipa::path(
    post,
    path = "/",
    tag = "Admin Service Forms",
    security(("bearer_auth" = []), ("csrf_token" = [])),
    request_body = CreateServiceForm,
    responses(
        (status = 200, description = "Service form created", body = ServiceForm),
        (status = 400, description = "Invalid input", body = ErrorResponse)
    )
)]
pub(crate) async fn create(
    auth: AuthAdmin,
    State(state): State<AppState>,
    Json(payload): Json<CreateServiceForm>,
) -> Result<Json<ServiceForm>, ApiError> {
    crate::require_permission!(auth, state, "forms.manage");
    validate_required_string(&payload.name, "Name")?;

    let form: ServiceForm = sqlx::query_as(
        r#"INSERT INTO service_forms (name, description, fields)
           VALUES ($1, $2, $3)
           RETURNING *"#,
    )
    .bind(payload.name)
    .bind(payload.description)
    .bind(payload.fields)
    .fetch_one(&state.db)
    .await?;

    Ok(Json(form))
}

#[derive(Deserialize, ToSchema)]
pub(crate) struct UpdateServiceForm {
    name: Option<String>,
    description: Option<String>,
    fields: Option<serde_json::Value>,
}

#[utoipa::path(
    put,
    path = "/{id}",
    tag = "Admin Service Forms",
    security(("bearer_auth" = []), ("csrf_token" = [])),
    params(("id" = Uuid, Path, description = "Service form ID")),
    request_body = UpdateServiceForm,
    responses(
        (status = 200, description = "Service form updated", body = ServiceForm),
        (status = 404, description = "Service form not found", body = ErrorResponse)
    )
)]
pub(crate) async fn update(
    auth: AuthAdmin,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(payload): Json<UpdateServiceForm>,
) -> Result<Json<ServiceForm>, ApiError> {
    crate::require_permission!(auth, state, "forms.manage");
    let form: ServiceForm = sqlx::query_as(
        r#"UPDATE service_forms
           SET name = COALESCE($1, name),
               description = COALESCE($2, description),
               fields = COALESCE($3, fields),
               updated_at = now()
           WHERE id = $4
           RETURNING *"#,
    )
    .bind(payload.name)
    .bind(payload.description)
    .bind(payload.fields)
    .bind(id)
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound("service form not found".into()))?;

    Ok(Json(form))
}

#[utoipa::path(
    delete,
    path = "/{id}",
    tag = "Admin Service Forms",
    security(("bearer_auth" = []), ("csrf_token" = [])),
    params(("id" = Uuid, Path, description = "Service form ID")),
    responses(
        (status = 200, description = "Service form deleted"),
        (status = 404, description = "Service form not found", body = ErrorResponse)
    )
)]
pub(crate) async fn delete(
    auth: AuthAdmin,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<()>, ApiError> {
    crate::require_permission!(auth, state, "forms.manage");
    let result = sqlx::query("DELETE FROM service_forms WHERE id = $1")
        .bind(id)
        .execute(&state.db)
        .await?;

    if result.rows_affected() == 0 {
        return Err(ApiError::NotFound("service form not found".into()));
    }

    Ok(Json(()))
}
