use axum::extract::{Path, State};
use axum::routing::get;
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::auth::AuthAdmin;
use crate::error::ApiError;
use crate::state::AppState;
use vaultscope_db::models::{Role, Staff};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/", get(list))
        .route("/me", get(me))
        .route("/roles", get(list_roles).post(create_role))
        .route("/roles/{id}", get(get_role).put(update_role).delete(delete_role))
}

#[derive(Serialize)]
struct StaffWithRole {
    #[serde(flatten)]
    staff: Staff,
    role_name: String,
}

async fn list(
    _auth: AuthAdmin,
    State(state): State<AppState>,
) -> Result<Json<Vec<StaffWithRole>>, ApiError> {
    let staff: Vec<Staff> = sqlx::query_as("SELECT * FROM staff ORDER BY created_at")
        .fetch_all(&state.db)
        .await?;

    let roles: Vec<Role> = sqlx::query_as("SELECT * FROM roles")
        .fetch_all(&state.db)
        .await?;

    let result = staff
        .into_iter()
        .map(|s| {
            let role_name = roles
                .iter()
                .find(|r| r.id == s.role_id)
                .map(|r| r.name.clone())
                .unwrap_or_default();
            StaffWithRole { staff: s, role_name }
        })
        .collect();

    Ok(Json(result))
}

async fn me(
    auth: AuthAdmin,
    State(state): State<AppState>,
) -> Result<Json<StaffWithRole>, ApiError> {
    let staff: Staff = sqlx::query_as("SELECT * FROM staff WHERE id = $1")
        .bind(auth.0.sub)
        .fetch_optional(&state.db)
        .await?
        .ok_or(ApiError::NotFound("staff not found".into()))?;

    let role: Role = sqlx::query_as("SELECT * FROM roles WHERE id = $1")
        .bind(staff.role_id)
        .fetch_optional(&state.db)
        .await?
        .ok_or(ApiError::NotFound("role not found".into()))?;

    Ok(Json(StaffWithRole {
        staff,
        role_name: role.name,
    }))
}

async fn list_roles(
    _auth: AuthAdmin,
    State(state): State<AppState>,
) -> Result<Json<Vec<Role>>, ApiError> {
    let roles: Vec<Role> = sqlx::query_as("SELECT * FROM roles ORDER BY name")
        .fetch_all(&state.db)
        .await?;

    Ok(Json(roles))
}

async fn get_role(
    _auth: AuthAdmin,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<Role>, ApiError> {
    let role: Role = sqlx::query_as("SELECT * FROM roles WHERE id = $1")
        .bind(id)
        .fetch_optional(&state.db)
        .await?
        .ok_or(ApiError::NotFound("role not found".into()))?;

    Ok(Json(role))
}

#[derive(Deserialize)]
struct CreateRole {
    name: String,
    permissions: Vec<String>,
    mapped_group: String,
}

async fn create_role(
    _auth: AuthAdmin,
    State(state): State<AppState>,
    Json(payload): Json<CreateRole>,
) -> Result<Json<Role>, ApiError> {
    let role: Role = sqlx::query_as(
        r#"INSERT INTO roles (name, permissions, mapped_group)
           VALUES ($1, $2, $3)
           RETURNING *"#
    )
    .bind(&payload.name)
    .bind(&payload.permissions)
    .bind(&payload.mapped_group)
    .fetch_one(&state.db)
    .await?;

    Ok(Json(role))
}

#[derive(Deserialize)]
struct UpdateRole {
    name: Option<String>,
    permissions: Option<Vec<String>>,
    mapped_group: Option<String>,
}

async fn update_role(
    _auth: AuthAdmin,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(payload): Json<UpdateRole>,
) -> Result<Json<Role>, ApiError> {
    let role: Role = sqlx::query_as(
        r#"UPDATE roles
           SET name = COALESCE($1, name),
               permissions = COALESCE($2, permissions),
               mapped_group = COALESCE($3, mapped_group),
               updated_at = now()
           WHERE id = $4
           RETURNING *"#
    )
    .bind(payload.name)
    .bind(payload.permissions)
    .bind(payload.mapped_group)
    .bind(id)
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound("role not found".into()))?;

    Ok(Json(role))
}

async fn delete_role(
    _auth: AuthAdmin,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<()>, ApiError> {
    let staff_count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM staff WHERE role_id = $1")
        .bind(id)
        .fetch_one(&state.db)
        .await?;

    if staff_count.0 > 0 {
        return Err(ApiError::BadRequest(format!("Cannot delete role: {} staff members are assigned to it", staff_count.0)));
    }

    let result = sqlx::query("DELETE FROM roles WHERE id = $1")
        .bind(id)
        .execute(&state.db)
        .await?;

    if result.rows_affected() == 0 {
        return Err(ApiError::NotFound("role not found".into()));
    }

    Ok(Json(()))
}
