use crate::auth::AuthAdmin;
use crate::error::ApiError;
use crate::state::AppState;
use vaultscope_db::models::{Role, Staff};

/// Evaluates whether a set of held RBAC permissions satisfies `required_perm`.
///
/// Supports:
/// - Wildcard (`"*"`)
/// - Exact permission match
/// - Domain `.manage` inheritance (`<domain>.manage` grants `<domain>.view`, `<domain>.create`,
///   `<domain>.edit`, `<domain>.reply`, `<domain>.power`, `<domain>.reinstall`, `<domain>.suspend`,
///   `<domain>.impersonate`, and `<domain>.delete` for standard resource domains)
/// - Cross-domain operational aliases (e.g. `billing.manage` -> `invoices.*`, `tax.manage`, `coupons.manage`;
///   `tickets.manage` -> `departments.manage`; `products.manage` -> `forms.manage`)
pub fn permission_satisfies(held_permissions: &[String], required_perm: &str) -> bool {
    if held_permissions.iter().any(|p| p == "*" || p == required_perm) {
        return true;
    }

    let has = |p: &str| held_permissions.iter().any(|hp| hp == p);

    if let Some((domain, action)) = required_perm.split_once('.') {
        let domain_manage = format!("{domain}.manage");
        if has(&domain_manage) {
            match (domain, action) {
                // High-risk destructive / privilege actions require explicit permission or wildcard
                ("staff", "delete")
                | ("accounts", "delete")
                | ("services", "terminate")
                | ("rbac", "manage") => {}
                _ => return true,
            }
        }
    }

    match required_perm {
        "billing.view" => has("billing.manage") || has("invoices.manage") || has("reports.view"),
        "invoices.view" => has("billing.view") || has("billing.manage") || has("invoices.manage"),
        "invoices.create" | "invoices.edit" | "invoices.delete" | "invoices.manage" => {
            has("billing.manage") || has("invoices.manage")
        }
        "tax.manage" | "coupons.manage" => has("billing.manage"),
        "departments.manage" | "tickets.create" | "tickets.reply" | "tickets.view" => {
            has("tickets.manage")
        }
        "forms.manage" => has("products.manage"),
        "customers.impersonate" => has("customers.manage") || has("customers.edit"),
        "customers.delete" => has("accounts.delete"),
        "staff.delete" => has("accounts.delete"),
        "staff.view" => has("staff.manage") || has("rbac.manage"),
        "email_logs.view" | "mail.view" => {
            has("email_logs.manage") || has("mail.manage") || has("email_logs.view") || has("mail.view")
        }
        "blog.view" | "blog.create" | "blog.edit" | "blog.delete" => {
            has("blog.manage") || has("templates.manage")
        }
        _ => false,
    }
}

/// Check if the authenticated admin has the required permission.
pub async fn check_permission(
    auth: &AuthAdmin,
    state: &AppState,
    required_perm: &str,
) -> Result<(), ApiError> {
    let staff_id = auth.0.sub;

    let staff: Staff = sqlx::query_as("SELECT * FROM staff WHERE id = $1")
        .bind(staff_id)
        .fetch_optional(&state.db)
        .await?
        .ok_or(ApiError::Unauthorized)?;

    let role: Role = sqlx::query_as("SELECT * FROM roles WHERE id = $1")
        .bind(staff.role_id)
        .fetch_optional(&state.db)
        .await?
        .ok_or(ApiError::Forbidden)?;

    if role.name.eq_ignore_ascii_case("Owner")
        || role.name.eq_ignore_ascii_case("Super Admin")
        || permission_satisfies(&role.permissions, required_perm)
    {
        return Ok(());
    }

    Err(ApiError::Forbidden)
}

/// Convenience macro for permission checking in handlers.
///
/// Usage:
/// ```ignore
/// require_permission!(auth, state, "users.delete");
/// ```
#[macro_export]
macro_rules! require_permission {
    ($auth:expr, $state:expr, $perm:expr) => {
        $crate::middleware::rbac::check_permission(&$auth, &$state, $perm).await?
    };
}

