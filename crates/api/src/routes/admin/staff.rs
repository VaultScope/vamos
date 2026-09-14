use axum::extract::{Multipart, Path, State};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use std::path::Path as StdPath;
use tokio::fs;
use uuid::Uuid;

use crate::auth::AuthAdmin;
use crate::error::ApiError;
use crate::state::AppState;
use crate::validation::{validate_email, validate_required_string};
use vaultscope_db::models::{Role, Staff};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/", axum::routing::get(list).post(create_staff))
        .route(
            "/{id}",
            axum::routing::put(update_staff).delete(delete_staff),
        )
        .route("/{id}/avatar", axum::routing::post(upload_avatar))
        .route("/me", axum::routing::get(me).put(update_me))
        .route("/roles", axum::routing::get(list_roles).post(create_role))
        .route(
            "/roles/{id}",
            axum::routing::get(get_role)
                .put(update_role)
                .delete(delete_role),
        )
}

#[derive(Serialize)]
struct StaffWithRole {
    #[serde(flatten)]
    staff: Staff,
    role_name: String,
}

async fn list(
    auth: AuthAdmin,
    State(state): State<AppState>,
) -> Result<Json<Vec<StaffWithRole>>, ApiError> {
    crate::require_permission!(auth, state, "staff.view");
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
            StaffWithRole {
                staff: s,
                role_name,
            }
        })
        .collect();

    Ok(Json(result))
}

async fn me(
    auth: AuthAdmin,
    State(state): State<AppState>,
) -> Result<Json<StaffWithRole>, ApiError> {
    // No permission check needed — every staff member can read their own profile
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
    auth: AuthAdmin,
    State(state): State<AppState>,
) -> Result<Json<Vec<Role>>, ApiError> {
    crate::require_permission!(auth, state, "rbac.manage");
    let roles: Vec<Role> = sqlx::query_as("SELECT * FROM roles ORDER BY name")
        .fetch_all(&state.db)
        .await?;

    Ok(Json(roles))
}

async fn get_role(
    auth: AuthAdmin,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<Role>, ApiError> {
    crate::require_permission!(auth, state, "rbac.manage");
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
    auth: AuthAdmin,
    State(state): State<AppState>,
    Json(payload): Json<CreateRole>,
) -> Result<Json<Role>, ApiError> {
    crate::require_permission!(auth, state, "rbac.manage");
    let role: Role = sqlx::query_as(
        r#"INSERT INTO roles (name, permissions, mapped_group)
           VALUES ($1, $2, $3)
           RETURNING *"#,
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
    auth: AuthAdmin,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(payload): Json<UpdateRole>,
) -> Result<Json<Role>, ApiError> {
    crate::require_permission!(auth, state, "rbac.manage");
    let role: Role = sqlx::query_as(
        r#"UPDATE roles
           SET name = COALESCE($1, name),
               permissions = COALESCE($2, permissions),
               mapped_group = COALESCE($3, mapped_group),
               updated_at = now()
           WHERE id = $4
           RETURNING *"#,
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
    auth: AuthAdmin,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<()>, ApiError> {
    crate::require_permission!(auth, state, "rbac.manage");
    let staff_count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM staff WHERE role_id = $1")
        .bind(id)
        .fetch_one(&state.db)
        .await?;

    if staff_count.0 > 0 {
        return Err(ApiError::BadRequest(format!(
            "Cannot delete role: {} staff members are assigned to it",
            staff_count.0
        )));
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

#[derive(Deserialize)]
struct CreateStaff {
    external_id: String,
    first_name: String,
    last_name: String,
    username: String,
    email: String,
    role_id: Uuid,
}

async fn create_staff(
    auth: AuthAdmin,
    State(state): State<AppState>,
    Json(payload): Json<CreateStaff>,
) -> Result<Json<Staff>, ApiError> {
    crate::require_permission!(auth, state, "staff.manage");
    if payload.external_id.trim().is_empty() {
        return Err(ApiError::BadRequest("external_id is required".into()));
    }
    validate_required_string(&payload.username, "Username")?;
    validate_email(&payload.email)?;

    let staff: Staff = sqlx::query_as(
        r#"INSERT INTO staff (id, external_id, first_name, last_name, username, email, role_id)
           VALUES ($1, $2, $3, $4, $5, $6, $7)
           RETURNING *"#,
    )
    .bind(Uuid::now_v7())
    .bind(&payload.external_id)
    .bind(&payload.first_name)
    .bind(&payload.last_name)
    .bind(&payload.username)
    .bind(&payload.email)
    .bind(payload.role_id)
    .fetch_one(&state.db)
    .await?;

    Ok(Json(staff))
}

#[derive(Deserialize)]
struct UpdateStaff {
    first_name: Option<String>,
    last_name: Option<String>,
    username: Option<String>,
    email: Option<String>,
    role_id: Option<Uuid>,
}

async fn update_staff(
    auth: AuthAdmin,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(payload): Json<UpdateStaff>,
) -> Result<Json<Staff>, ApiError> {
    crate::require_permission!(auth, state, "staff.manage");
    let staff: Staff = sqlx::query_as(
        r#"UPDATE staff
           SET first_name = COALESCE($1, first_name),
               last_name = COALESCE($2, last_name),
               username = COALESCE($3, username),
               email = COALESCE($4, email),
               role_id = COALESCE($5, role_id),
               updated_at = now()
           WHERE id = $6
           RETURNING *"#,
    )
    .bind(payload.first_name)
    .bind(payload.last_name)
    .bind(payload.username)
    .bind(payload.email)
    .bind(payload.role_id)
    .bind(id)
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound("staff not found".into()))?;

    Ok(Json(staff))
}

async fn delete_staff(
    auth: AuthAdmin,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<()>, ApiError> {
    crate::require_permission!(auth, state, "staff.manage");
    let result = sqlx::query("DELETE FROM staff WHERE id = $1")
        .bind(id)
        .execute(&state.db)
        .await?;

    if result.rows_affected() == 0 {
        return Err(ApiError::NotFound("staff not found".into()));
    }
    Ok(Json(()))
}

#[derive(Deserialize)]
struct UpdateMe {
    first_name: Option<String>,
    last_name: Option<String>,
    username: Option<String>,
    email: Option<String>,
}

async fn update_me(
    auth: AuthAdmin,
    State(state): State<AppState>,
    Json(payload): Json<UpdateMe>,
) -> Result<Json<Staff>, ApiError> {
    let id = auth.0.sub;
    let staff: Staff = sqlx::query_as(
        r#"UPDATE staff
           SET first_name = COALESCE($1, first_name),
               last_name = COALESCE($2, last_name),
               username = COALESCE($3, username),
               email = COALESCE($4, email),
               updated_at = now()
           WHERE id = $5
           RETURNING *"#,
    )
    .bind(payload.first_name)
    .bind(payload.last_name)
    .bind(payload.username)
    .bind(payload.email)
    .bind(id)
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound("staff not found".into()))?;

    Ok(Json(staff))
}

async fn upload_avatar(
    auth: AuthAdmin,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    mut multipart: Multipart,
) -> Result<Json<Staff>, ApiError> {
    if auth.0.sub != id {
        crate::require_permission!(auth, state, "staff.manage");
    }

    let mut avatar_url = String::new();

    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| ApiError::BadRequest(e.to_string()))?
    {
        if field.name() == Some("avatar") {
            let file_name = field.file_name().unwrap_or("avatar.png").to_string();
            let ext = file_name
                .split('.')
                .next_back()
                .unwrap_or("")
                .to_lowercase();
            if !["png", "jpg", "jpeg", "webp"].contains(&ext.as_str()) {
                return Err(ApiError::BadRequest("Invalid file extension".to_string()));
            }

            let data = field
                .bytes()
                .await
                .map_err(|e| ApiError::BadRequest(e.to_string()))?;
            let dir = StdPath::new("uploads/avatars");
            fs::create_dir_all(dir)
                .await
                .map_err(|e| ApiError::Internal(e.to_string()))?;

            let new_filename = format!("{}.{}", id, ext);
            let path = dir.join(&new_filename);
            fs::write(&path, data)
                .await
                .map_err(|e| ApiError::Internal(e.to_string()))?;

            avatar_url = format!("/uploads/avatars/{}", new_filename);
            break;
        }
    }

    if avatar_url.is_empty() {
        return Err(ApiError::BadRequest("No avatar file provided".to_string()));
    }

    let staff: Staff = sqlx::query_as(
        r#"UPDATE staff SET avatar_url = $1, updated_at = now() WHERE id = $2 RETURNING *"#,
    )
    .bind(&avatar_url)
    .bind(id)
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound("staff not found".into()))?;

    Ok(Json(staff))
}
