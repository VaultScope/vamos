use axum::extract::State;
use uuid::Uuid;

use crate::auth::AuthAdmin;
use crate::error::ApiError;
use crate::state::AppState;
use vaultscope_db::models::{Role, Staff};

/// Check if the authenticated admin has the required permission.
///
/// Permission checking logic:
/// 1. Fetch the staff member by ID from the JWT claims
/// 2. Fetch the role assigned to the staff member
/// 3. Check if the role has either:
///    - Wildcard permission ("*") - superadmin
///    - The specific required permission
///
/// Returns Ok(()) if authorized, Err(ApiError::Forbidden) otherwise.
pub async fn check_permission(
    auth: &AuthAdmin,
    state: &AppState,
    required_perm: &str,
) -> Result<(), ApiError> {
    // The subject is already a Uuid
    let staff_id = auth.0.sub;

    // Fetch the staff member
    let staff: Staff = sqlx::query_as("SELECT * FROM staff WHERE id = $1")
        .bind(staff_id)
        .fetch_optional(&state.db)
        .await?
        .ok_or(ApiError::Unauthorized)?;

    // Fetch the role
    let role: Role = sqlx::query_as("SELECT * FROM roles WHERE id = $1")
        .bind(staff.role_id)
        .fetch_optional(&state.db)
        .await?
        .ok_or_else(|| ApiError::Forbidden)?;

    // Check for wildcard permission (superadmin)
    if role.permissions.contains(&"*".to_string()) {
        return Ok(());
    }

    // Check for specific permission
    if role.permissions.contains(&required_perm.to_string()) {
        return Ok(());
    }

    // Permission denied
    Err(ApiError::Forbidden)
}

/// Convenience macro for permission checking in handlers.
///
/// Usage:
/// ```
/// require_permission!(auth, state, "users.delete");
/// ```
#[macro_export]
macro_rules! require_permission {
    ($auth:expr, $state:expr, $perm:expr) => {
        $crate::middleware::rbac::check_permission(&$auth, &$state, $perm).await?
    };
}
