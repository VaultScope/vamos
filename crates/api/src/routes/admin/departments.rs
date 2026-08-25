use axum::extract::{Path, State};
use axum::routing::get;
use axum::{Json, Router};
use serde::Deserialize;
use uuid::Uuid;

use crate::auth::AuthAdmin;
use crate::error::ApiError;
use crate::state::AppState;
use vaultscope_db::models::Department;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/", get(list).post(create))
        .route("/{id}", get(get_one).put(update).delete(delete))
}

async fn list(
    _auth: AuthAdmin,
    State(state): State<AppState>,
) -> Result<Json<Vec<Department>>, ApiError> {
    let departments: Vec<Department> = sqlx::query_as(
        "SELECT * FROM departments ORDER BY name"
    )
    .fetch_all(&state.db)
    .await?;
    Ok(Json(departments))
}

async fn get_one(
    _auth: AuthAdmin,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<Department>, ApiError> {
    let department: Department = sqlx::query_as("SELECT * FROM departments WHERE id = $1")
        .bind(id)
        .fetch_optional(&state.db)
        .await?
        .ok_or(ApiError::NotFound("department not found".into()))?;
    Ok(Json(department))
}

#[derive(Deserialize)]
struct CreateDepartment {
    name: String,
    mailbox: String,
    default_assignee_id: Option<Uuid>,
}

async fn create(
    _auth: AuthAdmin,
    State(state): State<AppState>,
    Json(payload): Json<CreateDepartment>,
) -> Result<Json<Department>, ApiError> {
    let department: Department = sqlx::query_as(
        r#"INSERT INTO departments (name, mailbox, default_assignee_id)
           VALUES ($1, $2, $3)
           RETURNING *"#
    )
    .bind(&payload.name)
    .bind(&payload.mailbox)
    .bind(payload.default_assignee_id)
    .fetch_one(&state.db)
    .await?;

    Ok(Json(department))
}

#[derive(Deserialize)]
struct UpdateDepartment {
    name: Option<String>,
    mailbox: Option<String>,
    default_assignee_id: Option<Uuid>,
}

async fn update(
    _auth: AuthAdmin,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(payload): Json<UpdateDepartment>,
) -> Result<Json<Department>, ApiError> {
    let department: Department = sqlx::query_as(
        r#"UPDATE departments
           SET name = COALESCE($1, name),
               mailbox = COALESCE($2, mailbox),
               default_assignee_id = COALESCE($3, default_assignee_id)
           WHERE id = $4
           RETURNING *"#
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

async fn delete(
    _auth: AuthAdmin,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<()>, ApiError> {
    let result = sqlx::query("DELETE FROM departments WHERE id = $1")
        .bind(id)
        .execute(&state.db)
        .await?;

    if result.rows_affected() == 0 {
        return Err(ApiError::NotFound("department not found".into()));
    }

    Ok(Json(()))
}
