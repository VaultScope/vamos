use axum::extract::State;
use axum::routing::get;
use axum::{Json, Router};
use serde::Serialize;

use crate::auth::AuthAdmin;
use crate::error::ApiError;
use crate::state::AppState;
use vaultscope_db::models::{Role, Staff};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/", get(list))
        .route("/me", get(me))
        .route("/roles", get(list_roles))
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
