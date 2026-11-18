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
use vaultscope_db::models::Department;

pub fn routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(list, create))
        .routes(routes!(get_one, update, delete))
}

#[utoipa::path(
    get,
    path = "/",
    tag = "Admin Departments",
    security(("bearer_auth" = [])),
    responses((status = 200, description = "List support departments", body = Vec<Department>))
)]
pub(crate) async fn list(
    auth: AuthAdmin,
    State(state): State<AppState>,
) -> Result<Json<Vec<Department>>, ApiError> {
    crate::require_permission!(auth, state, "tickets.manage");
    let departments: Vec<Department> = sqlx::query_as("SELECT * FROM departments ORDER BY name")
        .fetch_all(&state.db)
        .await?;
    Ok(Json(departments))
}

#[utoipa::path(
    get,
    path = "/{id}",
    tag = "Admin Departments",
    security(("bearer_auth" = [])),
    params(("id" = Uuid, Path, description = "Department ID")),
    responses(
        (status = 200, description = "Support department details", body = Department),
        (status = 404, description = "Department not found", body = ErrorResponse)
    )
)]
pub(crate) async fn get_one(
    auth: AuthAdmin,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<Department>, ApiError> {
    crate::require_permission!(auth, state, "tickets.manage");
    let department: Department = sqlx::query_as("SELECT * FROM departments WHERE id = $1")
        .bind(id)
        .fetch_optional(&state.db)
        .await?
        .ok_or(ApiError::NotFound("department not found".into()))?;
    Ok(Json(department))
}

#[derive(Deserialize, ToSchema)]
pub(crate) struct CreateDepartment {
    name: String,
    mailbox: String,
    default_assignee_id: Option<Uuid>,
}

#[utoipa::path(
    post,
    path = "/",
    tag = "Admin Departments",
    security(("bearer_auth" = []), ("csrf_token" = [])),
    request_body = CreateDepartment,
    responses((status = 200, description = "Created support department", body = Department))
)]
pub(crate) async fn create(
    auth: AuthAdmin,
    State(state): State<AppState>,
    Json(payload): Json<CreateDepartment>,
) -> Result<Json<Department>, ApiError> {
    crate::require_permission!(auth, state, "tickets.manage");
    let department: Department = sqlx::query_as(
        r#"INSERT INTO departments (name, mailbox, default_assignee_id)
           VALUES ($1, $2, $3)
           RETURNING *"#,
    )
    .bind(&payload.name)
    .bind(&payload.mailbox)
    .bind(payload.default_assignee_id)
    .fetch_one(&state.db)
    .await?;

    Ok(Json(department))
}

#[derive(Deserialize, ToSchema)]
pub(crate) struct UpdateDepartment {
    name: Option<String>,
    mailbox: Option<String>,
    default_assignee_id: Option<Uuid>,
}

#[utoipa::path(
    put,
    path = "/{id}",
    tag = "Admin Departments",
    security(("bearer_auth" = []), ("csrf_token" = [])),
    params(("id" = Uuid, Path, description = "Department ID")),
    request_body = UpdateDepartment,
    responses(
        (status = 200, description = "Updated support department", body = Department),
        (status = 404, description = "Department not found", body = ErrorResponse)
    )
)]
pub(crate) async fn update(
    auth: AuthAdmin,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(payload): Json<UpdateDepartment>,
) -> Result<Json<Department>, ApiError> {
    crate::require_permission!(auth, state, "tickets.manage");
    let department: Department = sqlx::query_as(
        r#"UPDATE departments
           SET name = COALESCE($1, name),
               mailbox = COALESCE($2, mailbox),
               default_assignee_id = COALESCE($3, default_assignee_id)
           WHERE id = $4
           RETURNING *"#,
    )
    .bind(payload.name)
    .bind(payload.mailbox)
    .bind(payload.default_assignee_id)
    .bind(id)
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound("department not found".into()))?;

    Ok(Json(department))
}

#[utoipa::path(
    delete,
    path = "/{id}",
    tag = "Admin Departments",
    security(("bearer_auth" = []), ("csrf_token" = [])),
    params(("id" = Uuid, Path, description = "Department ID")),
    responses(
        (status = 200, description = "Department deleted"),
        (status = 404, description = "Department not found", body = ErrorResponse)
    )
)]
pub(crate) async fn delete(
    auth: AuthAdmin,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<()>, ApiError> {
    crate::require_permission!(auth, state, "tickets.manage");
    let result = sqlx::query("DELETE FROM departments WHERE id = $1")
        .bind(id)
        .execute(&state.db)
        .await?;

    if result.rows_affected() == 0 {
        return Err(ApiError::NotFound("department not found".into()));
    }

    Ok(Json(()))
}
