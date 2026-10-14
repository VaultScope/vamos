use axum::extract::{Multipart, Path, State};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
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
    external_id: Option<String>,
    first_name: String,
    last_name: String,
    username: String,
    email: String,
    role_id: Uuid,
    password: Option<String>,
}

async fn create_staff(
    auth: AuthAdmin,
    State(state): State<AppState>,
    Json(payload): Json<CreateStaff>,
) -> Result<Json<Staff>, ApiError> {
    crate::require_permission!(auth, state, "staff.manage");
    let ext_id = payload
        .external_id
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| format!("local_{}", Uuid::now_v7()));
    validate_required_string(&payload.username, "Username")?;
    validate_email(&payload.email)?;

    let password_hash = match payload.password {
        Some(pw) if !pw.trim().is_empty() => {
            Some(bcrypt::hash(pw, bcrypt::DEFAULT_COST).map_err(|e| ApiError::Internal(e.to_string()))?)
        }
        _ => None,
    };

    let staff: Staff = sqlx::query_as(
        r#"INSERT INTO staff (id, external_id, first_name, last_name, username, email, role_id, password_hash)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
           RETURNING *"#,
    )
    .bind(Uuid::now_v7())
    .bind(&ext_id)
    .bind(&payload.first_name)
    .bind(&payload.last_name)
    .bind(&payload.username)
    .bind(&payload.email)
    .bind(payload.role_id)
    .bind(password_hash)
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
    password: Option<String>,
}

async fn update_staff(
    auth: AuthAdmin,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(payload): Json<UpdateStaff>,
) -> Result<Json<Staff>, ApiError> {
    crate::require_permission!(auth, state, "staff.manage");
    let password_hash = match payload.password {
        Some(pw) if !pw.trim().is_empty() => {
            Some(bcrypt::hash(pw, bcrypt::DEFAULT_COST).map_err(|e| ApiError::Internal(e.to_string()))?)
        }
        _ => None,
    };

    let staff: Staff = sqlx::query_as(
        r#"UPDATE staff
           SET first_name = COALESCE($1, first_name),
               last_name = COALESCE($2, last_name),
               username = COALESCE($3, username),
               email = COALESCE($4, email),
               role_id = COALESCE($5, role_id),
               password_hash = COALESCE($6, password_hash),
               updated_at = now()
           WHERE id = $7
           RETURNING *"#,
    )
    .bind(payload.first_name)
    .bind(payload.last_name)
    .bind(payload.username)
    .bind(payload.email)
    .bind(payload.role_id)
    .bind(password_hash)
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
    if crate::middleware::rbac::check_permission(&auth, &state, "staff.delete").await.is_err()
        && crate::middleware::rbac::check_permission(&auth, &state, "accounts.delete").await.is_err()
    {
        crate::require_permission!(auth, state, "staff.manage");
    }

    let target_staff: Option<Staff> = sqlx::query_as("SELECT * FROM staff WHERE id = $1")
        .bind(id)
        .fetch_optional(&state.db)
        .await?;

    let result = sqlx::query("DELETE FROM staff WHERE id = $1")
        .bind(id)
        .execute(&state.db)
        .await?;

    if result.rows_affected() == 0 {
        return Err(ApiError::NotFound("staff not found".into()));
    }

    if let Some(target) = target_staff {
        crate::activity::spawn_record(
            state.db.clone(),
            Some(auth.0.sub),
            vaultscope_db::models::ActorType::Staff,
            auth.0.email.clone(),
            "staff".to_string(),
            "deleted_staff_account".to_string(),
            target.email.clone(),
            format!("Deleted staff account @{} ({})", target.username, target.email),
        );
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
        if field.name() == Some("avatar") || field.name() == Some("file") {
            let data = field
                .bytes()
                .await
                .map_err(|e| ApiError::BadRequest(e.to_string()))?;
            let processed = crate::media::save_optimized_image(&data, "avatars", 512).await?;
            avatar_url = processed.url;
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
