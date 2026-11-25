use axum::extract::{Multipart, Path, State};
use axum::Json;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use utoipa_axum::{router::OpenApiRouter, routes};
use uuid::Uuid;

use crate::auth::AuthAdmin;
use crate::error::ApiError;
use crate::openapi::{BinaryFileUpload, ErrorResponse};
use crate::state::AppState;
use crate::validation::{validate_email, validate_required_string, validate_string_length};
use vaultscope_db::models::{Role, Staff};

/// Prevents RBAC privilege escalation when creating/updating staff or roles.
/// Non-wildcard callers may never grant `"*"` and may only assign roles whose
/// permissions are a subset of the caller's own permissions (including `"rbac.manage"`).
pub fn ensure_role_assignable(
    caller_perms: &[String],
    target_perms: &[String],
) -> Result<(), ApiError> {
    if caller_perms.iter().any(|p| p == "*") {
        return Ok(());
    }
    for perm in target_perms {
        if perm == "*" || !caller_perms.iter().any(|cp| cp == perm) {
            return Err(ApiError::Forbidden);
        }
    }
    Ok(())
}

async fn fetch_caller_permissions(
    db: &sqlx::PgPool,
    staff_id: Uuid,
) -> Result<Vec<String>, ApiError> {
    let caller: Staff = sqlx::query_as("SELECT * FROM staff WHERE id = $1")
        .bind(staff_id)
        .fetch_optional(db)
        .await?
        .ok_or(ApiError::Unauthorized)?;

    let role: Role = sqlx::query_as("SELECT * FROM roles WHERE id = $1")
        .bind(caller.role_id)
        .fetch_optional(db)
        .await?
        .ok_or(ApiError::Forbidden)?;

    Ok(role.permissions)
}

async fn hash_staff_password(pw: String) -> Result<String, ApiError> {
    if pw.trim().is_empty() || !(8..=72).contains(&pw.len()) {
        return Err(ApiError::BadRequest(
            "Password must be between 8 and 72 bytes".into(),
        ));
    }
    tokio::task::spawn_blocking(move || bcrypt::hash(pw, bcrypt::DEFAULT_COST))
        .await
        .map_err(|e| ApiError::Internal(format!("Password hashing task failed: {e}")))?
        .map_err(|e| ApiError::Internal(format!("Password hashing failed: {e}")))
}

pub fn routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(list, create_staff))
        .routes(routes!(update_staff, delete_staff))
        .routes(routes!(upload_avatar))
        .routes(routes!(me, update_me))
        .routes(routes!(list_roles, create_role))
        .routes(routes!(get_role, update_role, delete_role))
}

#[derive(Serialize, ToSchema)]
pub(crate) struct StaffWithRole {
    #[serde(flatten)]
    staff: Staff,
    role_name: String,
    permissions: Vec<String>,
}

#[utoipa::path(
    get,
    path = "/",
    tag = "Admin Staff",
    security(("bearer_auth" = [])),
    responses((status = 200, description = "List staff accounts", body = Vec<StaffWithRole>))
)]
pub(crate) async fn list(
    auth: AuthAdmin,
    State(state): State<AppState>,
) -> Result<Json<Vec<StaffWithRole>>, ApiError> {
    crate::require_permission!(auth, state, "staff.view");
    let staff: Vec<Staff> = sqlx::query_as(
        r#"SELECT s.id, s.external_id, s.username, s.first_name, s.last_name, s.email,
                  (s.email_verified OR COALESCE(c.email_verified, false)) AS email_verified,
                  s.avatar_url, s.role_id,
                  (s.mfa_enabled OR COALESCE(c.two_factor_enabled, false)) AS mfa_enabled,
                  s.last_login, s.last_login_ip, s.created_at, s.updated_at, s.password_hash
           FROM staff s
           LEFT JOIN customers c ON LOWER(c.email) = LOWER(s.email) OR s.external_id = ('customer_' || c.id::text)
           ORDER BY s.created_at"#,
    )
    .fetch_all(&state.db)
    .await?;

    let roles: Vec<Role> = sqlx::query_as("SELECT * FROM roles")
        .fetch_all(&state.db)
        .await?;

    let result = staff
        .into_iter()
        .map(|s| {
            let matched_role = roles.iter().find(|r| r.id == s.role_id);
            let role_name = matched_role.map(|r| r.name.clone()).unwrap_or_default();
            let permissions = matched_role.map(|r| r.permissions.clone()).unwrap_or_default();
            StaffWithRole {
                staff: s,
                role_name,
                permissions,
            }
        })
        .collect();

    Ok(Json(result))
}

#[utoipa::path(
    get,
    path = "/me",
    tag = "Admin Staff",
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "Current authenticated staff profile", body = StaffWithRole),
        (status = 404, description = "Staff or role not found", body = ErrorResponse)
    )
)]
pub(crate) async fn me(
    auth: AuthAdmin,
    State(state): State<AppState>,
) -> Result<Json<StaffWithRole>, ApiError> {
    // No permission check needed — every staff member can read their own profile
    let staff: Staff = sqlx::query_as(
        r#"SELECT s.id, s.external_id, s.username, s.first_name, s.last_name, s.email,
                  (s.email_verified OR COALESCE(c.email_verified, false)) AS email_verified,
                  s.avatar_url, s.role_id,
                  (s.mfa_enabled OR COALESCE(c.two_factor_enabled, false)) AS mfa_enabled,
                  s.last_login, s.last_login_ip, s.created_at, s.updated_at, s.password_hash
           FROM staff s
           LEFT JOIN customers c ON LOWER(c.email) = LOWER(s.email) OR s.external_id = ('customer_' || c.id::text)
           WHERE s.id = $1
           LIMIT 1"#,
    )
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
        permissions: role.permissions,
    }))
}

#[utoipa::path(
    get,
    path = "/roles",
    tag = "Admin Staff",
    security(("bearer_auth" = [])),
    responses((status = 200, description = "List RBAC roles", body = Vec<Role>))
)]
pub(crate) async fn list_roles(
    auth: AuthAdmin,
    State(state): State<AppState>,
) -> Result<Json<Vec<Role>>, ApiError> {
    if crate::middleware::rbac::check_permission(&auth, &state, "rbac.manage").await.is_err()
        && crate::middleware::rbac::check_permission(&auth, &state, "staff.view").await.is_err()
        && crate::middleware::rbac::check_permission(&auth, &state, "customers.edit").await.is_err()
    {
        return Err(ApiError::Forbidden);
    }
    let roles: Vec<Role> = sqlx::query_as("SELECT * FROM roles ORDER BY name")
        .fetch_all(&state.db)
        .await?;

    Ok(Json(roles))
}

#[utoipa::path(
    get,
    path = "/roles/{id}",
    tag = "Admin Staff",
    security(("bearer_auth" = [])),
    params(("id" = Uuid, Path, description = "Role ID")),
    responses(
        (status = 200, description = "Role details", body = Role),
        (status = 404, description = "Role not found", body = ErrorResponse)
    )
)]
pub(crate) async fn get_role(
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

#[derive(Deserialize, ToSchema)]
pub(crate) struct CreateRole {
    name: String,
    permissions: Vec<String>,
    #[serde(default)]
    mapped_group: Option<String>,
}

fn derive_role_slug(name: &str, provided: Option<&str>) -> String {
    if let Some(p) = provided {
        let trimmed = p.trim();
        if !trimmed.is_empty() {
            return trimmed.to_string();
        }
    }
    let slug: String = name
        .trim()
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    let cleaned = slug.trim_matches('_').to_string();
    if cleaned.is_empty() {
        format!("role_{}", &Uuid::now_v7().simple().to_string()[24..])
    } else {
        format!("vs_{cleaned}")
    }
}

#[utoipa::path(
    post,
    path = "/roles",
    tag = "Admin Staff",
    security(("bearer_auth" = []), ("csrf_token" = [])),
    request_body = CreateRole,
    responses((status = 200, description = "Role created", body = Role))
)]
pub(crate) async fn create_role(
    auth: AuthAdmin,
    State(state): State<AppState>,
    Json(payload): Json<CreateRole>,
) -> Result<Json<Role>, ApiError> {
    crate::require_permission!(auth, state, "rbac.manage");
    validate_required_string(&payload.name, "Role name")?;
    let caller_perms = fetch_caller_permissions(&state.db, auth.0.sub).await?;
    ensure_role_assignable(&caller_perms, &payload.permissions)?;
    let mapped_group = derive_role_slug(&payload.name, payload.mapped_group.as_deref());

    let role: Role = sqlx::query_as(
        r#"INSERT INTO roles (name, permissions, mapped_group)
           VALUES ($1, $2, $3)
           RETURNING *"#,
    )
    .bind(payload.name.trim())
    .bind(&payload.permissions)
    .bind(&mapped_group)
    .fetch_one(&state.db)
    .await?;

    Ok(Json(role))
}

#[derive(Deserialize, ToSchema)]
pub(crate) struct UpdateRole {
    name: Option<String>,
    permissions: Option<Vec<String>>,
    mapped_group: Option<String>,
}

#[utoipa::path(
    put,
    path = "/roles/{id}",
    tag = "Admin Staff",
    security(("bearer_auth" = []), ("csrf_token" = [])),
    params(("id" = Uuid, Path, description = "Role ID")),
    request_body = UpdateRole,
    responses(
        (status = 200, description = "Role updated", body = Role),
        (status = 404, description = "Role not found", body = ErrorResponse)
    )
)]
pub(crate) async fn update_role(
    auth: AuthAdmin,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(payload): Json<UpdateRole>,
) -> Result<Json<Role>, ApiError> {
    crate::require_permission!(auth, state, "rbac.manage");
    let caller_perms = fetch_caller_permissions(&state.db, auth.0.sub).await?;

    let existing_role: Role = sqlx::query_as("SELECT * FROM roles WHERE id = $1")
        .bind(id)
        .fetch_optional(&state.db)
        .await?
        .ok_or(ApiError::NotFound("role not found".into()))?;
    ensure_role_assignable(&caller_perms, &existing_role.permissions)?;

    if let Some(ref new_perms) = payload.permissions {
        ensure_role_assignable(&caller_perms, new_perms)?;
    }

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

#[utoipa::path(
    delete,
    path = "/roles/{id}",
    tag = "Admin Staff",
    security(("bearer_auth" = []), ("csrf_token" = [])),
    params(("id" = Uuid, Path, description = "Role ID")),
    responses(
        (status = 200, description = "Role deleted"),
        (status = 400, description = "Role has assigned staff", body = ErrorResponse),
        (status = 404, description = "Role not found", body = ErrorResponse)
    )
)]
pub(crate) async fn delete_role(
    auth: AuthAdmin,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<()>, ApiError> {
    crate::require_permission!(auth, state, "rbac.manage");
    let caller_perms = fetch_caller_permissions(&state.db, auth.0.sub).await?;
    let existing_role: Role = sqlx::query_as("SELECT * FROM roles WHERE id = $1")
        .bind(id)
        .fetch_optional(&state.db)
        .await?
        .ok_or(ApiError::NotFound("role not found".into()))?;
    ensure_role_assignable(&caller_perms, &existing_role.permissions)?;

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

#[derive(Deserialize, ToSchema)]
pub(crate) struct CreateStaff {
    external_id: Option<String>,
    first_name: String,
    last_name: String,
    username: String,
    email: String,
    role_id: Uuid,
    password: Option<String>,
}

#[utoipa::path(
    post,
    path = "/",
    tag = "Admin Staff",
    security(("bearer_auth" = []), ("csrf_token" = [])),
    request_body = CreateStaff,
    responses(
        (status = 200, description = "Staff account created", body = Staff),
        (status = 400, description = "Invalid input", body = ErrorResponse),
        (status = 403, description = "Forbidden role assignment", body = ErrorResponse)
    )
)]
#[allow(clippy::type_complexity)]
pub(crate) async fn create_staff(
    auth: AuthAdmin,
    State(state): State<AppState>,
    Json(payload): Json<CreateStaff>,
) -> Result<Json<Staff>, ApiError> {
    crate::require_permission!(auth, state, "staff.manage");
    let caller_perms = fetch_caller_permissions(&state.db, auth.0.sub).await?;

    let target_role: Role = sqlx::query_as("SELECT * FROM roles WHERE id = $1")
        .bind(payload.role_id)
        .fetch_optional(&state.db)
        .await?
        .ok_or_else(|| ApiError::BadRequest("Invalid role_id".into()))?;
    ensure_role_assignable(&caller_perms, &target_role.permissions)?;

    validate_required_string(&payload.username, "Username")?;
    let email = payload.email.trim().to_string();
    validate_email(&email)?;

    // If a VaultScope customer account with this email exists, share its credentials & 2FA
    let linked_customer: Option<(Uuid, Option<String>, bool, bool, Option<String>)> =
        sqlx::query_as(
            "SELECT id, password_hash, email_verified, two_factor_enabled, totp_secret
             FROM customers
             WHERE LOWER(email) = LOWER($1)
             LIMIT 1",
        )
        .bind(&email)
        .fetch_optional(&state.db)
        .await?;

    let ext_id = payload
        .external_id
        .filter(|s| !s.trim().is_empty())
        .or_else(|| linked_customer.as_ref().map(|(cid, _, _, _, _)| format!("customer_{cid}")))
        .unwrap_or_else(|| format!("local_{}", Uuid::now_v7()));

    let explicit_password_hash = match payload.password {
        Some(pw) if !pw.is_empty() => Some(hash_staff_password(pw).await?),
        _ => None,
    };

    let (password_hash, email_verified, mfa_enabled, totp_secret) =
        if let Some((cid, cust_pw, cust_verified, cust_2fa, cust_totp)) = linked_customer {
            if let Some(ref new_hash) = explicit_password_hash {
                let _ = sqlx::query("UPDATE customers SET password_hash = COALESCE(password_hash, $1), updated_at = now() WHERE id = $2")
                    .bind(new_hash)
                    .bind(cid)
                    .execute(&state.db)
                    .await;
            }
            (
                explicit_password_hash.or(cust_pw),
                cust_verified,
                cust_2fa,
                cust_totp,
            )
        } else {
            (explicit_password_hash, false, false, None)
        };

    let staff: Staff = sqlx::query_as(
        r#"INSERT INTO staff (id, external_id, first_name, last_name, username, email, email_verified, role_id, password_hash, mfa_enabled, totp_secret)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
           RETURNING *"#,
    )
    .bind(Uuid::now_v7())
    .bind(&ext_id)
    .bind(payload.first_name.trim())
    .bind(payload.last_name.trim())
    .bind(payload.username.trim())
    .bind(&email)
    .bind(email_verified)
    .bind(payload.role_id)
    .bind(password_hash)
    .bind(mfa_enabled)
    .bind(totp_secret)
    .fetch_one(&state.db)
    .await?;

    Ok(Json(staff))
}

#[derive(Deserialize, ToSchema)]
pub(crate) struct UpdateStaff {
    first_name: Option<String>,
    last_name: Option<String>,
    username: Option<String>,
    email: Option<String>,
    role_id: Option<Uuid>,
    password: Option<String>,
}

#[utoipa::path(
    put,
    path = "/{id}",
    tag = "Admin Staff",
    security(("bearer_auth" = []), ("csrf_token" = [])),
    params(("id" = Uuid, Path, description = "Staff ID")),
    request_body = UpdateStaff,
    responses(
        (status = 200, description = "Staff account updated", body = Staff),
        (status = 403, description = "Forbidden role assignment or self role change", body = ErrorResponse),
        (status = 404, description = "Staff not found", body = ErrorResponse)
    )
)]
pub(crate) async fn update_staff(
    auth: AuthAdmin,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(payload): Json<UpdateStaff>,
) -> Result<Json<Staff>, ApiError> {
    crate::require_permission!(auth, state, "staff.manage");
    let caller_perms = fetch_caller_permissions(&state.db, auth.0.sub).await?;

    let target_staff: Staff = sqlx::query_as("SELECT * FROM staff WHERE id = $1")
        .bind(id)
        .fetch_optional(&state.db)
        .await?
        .ok_or(ApiError::NotFound("staff not found".into()))?;

    let existing_role: Role = sqlx::query_as("SELECT * FROM roles WHERE id = $1")
        .bind(target_staff.role_id)
        .fetch_optional(&state.db)
        .await?
        .ok_or(ApiError::Forbidden)?;
    ensure_role_assignable(&caller_perms, &existing_role.permissions)?;

    if let Some(new_role_id) = payload.role_id {
        if id == auth.0.sub && new_role_id != target_staff.role_id {
            return Err(ApiError::Forbidden);
        }
        let target_role: Role = sqlx::query_as("SELECT * FROM roles WHERE id = $1")
            .bind(new_role_id)
            .fetch_optional(&state.db)
            .await?
            .ok_or_else(|| ApiError::BadRequest("Invalid role_id".into()))?;
        ensure_role_assignable(&caller_perms, &target_role.permissions)?;
    }

    let trimmed_username = match payload.username {
        Some(ref u) => {
            validate_required_string(u, "Username")?;
            Some(u.trim().to_string())
        }
        None => None,
    };

    let trimmed_email = match payload.email {
        Some(ref e) => {
            let trimmed = e.trim().to_string();
            validate_email(&trimmed)?;
            Some(trimmed)
        }
        None => None,
    };

    let password_hash = match payload.password {
        Some(pw) if !pw.is_empty() => Some(hash_staff_password(pw).await?),
        _ => None,
    };

    let role_changed = payload.role_id.is_some_and(|r| r != target_staff.role_id);
    let bump_auth = password_hash.is_some() || role_changed;

    let staff: Staff = sqlx::query_as(
        r#"UPDATE staff
           SET first_name = COALESCE($1, first_name),
               last_name = COALESCE($2, last_name),
               username = COALESCE($3, username),
               email = COALESCE($4, email),
               email_verified = CASE
                   WHEN $4 IS NOT NULL AND LOWER($4) IS DISTINCT FROM LOWER(email) THEN false
                   ELSE email_verified
               END,
               role_id = COALESCE($5, role_id),
               password_hash = COALESCE($6, password_hash),
               auth_version = CASE
                   WHEN $7 THEN auth_version + 1
                   ELSE auth_version
               END,
               updated_at = now()
           WHERE id = $8
           RETURNING *"#,
    )
    .bind(payload.first_name.map(|s| s.trim().to_string()))
    .bind(payload.last_name.map(|s| s.trim().to_string()))
    .bind(trimmed_username)
    .bind(trimmed_email)
    .bind(payload.role_id)
    .bind(&password_hash)
    .bind(bump_auth)
    .bind(id)
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound("staff not found".into()))?;

    if let Some(ref new_hash) = password_hash {
        let _ = sqlx::query(
            "UPDATE customers SET password_hash = $1, auth_version = auth_version + 1, updated_at = now() WHERE LOWER(email) = LOWER($2) OR ('customer_' || id::text) = $3",
        )
        .bind(new_hash)
        .bind(&staff.email)
        .bind(&staff.external_id)
        .execute(&state.db)
        .await;
    }

    Ok(Json(staff))
}

#[utoipa::path(
    delete,
    path = "/{id}",
    tag = "Admin Staff",
    security(("bearer_auth" = []), ("csrf_token" = [])),
    params(("id" = Uuid, Path, description = "Staff ID")),
    responses(
        (status = 200, description = "Staff account deleted"),
        (status = 403, description = "Insufficient permissions", body = ErrorResponse),
        (status = 404, description = "Staff not found", body = ErrorResponse)
    )
)]
pub(crate) async fn delete_staff(
    auth: AuthAdmin,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<()>, ApiError> {
    if crate::middleware::rbac::check_permission(&auth, &state, "staff.delete").await.is_err()
        && crate::middleware::rbac::check_permission(&auth, &state, "accounts.delete").await.is_err()
    {
        crate::require_permission!(auth, state, "staff.manage");
    }

    if id == auth.0.sub {
        return Err(ApiError::Forbidden);
    }

    let caller_perms = fetch_caller_permissions(&state.db, auth.0.sub).await?;
    let target_staff: Staff = sqlx::query_as("SELECT * FROM staff WHERE id = $1")
        .bind(id)
        .fetch_optional(&state.db)
        .await?
        .ok_or(ApiError::NotFound("staff not found".into()))?;

    let target_role: Role = sqlx::query_as("SELECT * FROM roles WHERE id = $1")
        .bind(target_staff.role_id)
        .fetch_optional(&state.db)
        .await?
        .ok_or(ApiError::Forbidden)?;
    ensure_role_assignable(&caller_perms, &target_role.permissions)?;

    let result = sqlx::query("DELETE FROM staff WHERE id = $1")
        .bind(id)
        .execute(&state.db)
        .await?;

    if result.rows_affected() == 0 {
        return Err(ApiError::NotFound("staff not found".into()));
    }

    crate::activity::spawn_record(
        state.db.clone(),
        Some(auth.0.sub),
        vaultscope_db::models::ActorType::Staff,
        auth.0.email.clone(),
        "staff".to_string(),
        "deleted_staff_account".to_string(),
        target_staff.email.clone(),
        format!("Deleted staff account @{} ({})", target_staff.username, target_staff.email),
    );

    Ok(Json(()))
}

#[derive(Deserialize, ToSchema)]
pub(crate) struct UpdateMe {
    first_name: Option<String>,
    last_name: Option<String>,
    username: Option<String>,
    email: Option<String>,
}

#[utoipa::path(
    put,
    path = "/me",
    tag = "Admin Staff",
    security(("bearer_auth" = []), ("csrf_token" = [])),
    request_body = UpdateMe,
    responses(
        (status = 200, description = "Current staff profile updated", body = Staff),
        (status = 400, description = "Invalid input", body = ErrorResponse),
        (status = 404, description = "Staff not found", body = ErrorResponse)
    )
)]
pub(crate) async fn update_me(
    auth: AuthAdmin,
    State(state): State<AppState>,
    Json(payload): Json<UpdateMe>,
) -> Result<Json<Staff>, ApiError> {
    let id = auth.0.sub;

    if let Some(ref f) = payload.first_name {
        validate_string_length(f, "First name", 255)?;
    }
    if let Some(ref l) = payload.last_name {
        validate_string_length(l, "Last name", 255)?;
    }
    let trimmed_username = match payload.username {
        Some(ref u) => {
            validate_required_string(u, "Username")?;
            validate_string_length(u.trim(), "Username", 150)?;
            Some(u.trim().to_string())
        }
        None => None,
    };
    let trimmed_email = match payload.email {
        Some(ref e) => {
            let trimmed = e.trim().to_string();
            validate_email(&trimmed)?;
            Some(trimmed)
        }
        None => None,
    };

    let staff: Staff = sqlx::query_as(
        r#"UPDATE staff
           SET first_name = COALESCE($1, first_name),
               last_name = COALESCE($2, last_name),
               username = COALESCE($3, username),
               email = COALESCE($4, email),
               email_verified = CASE
                   WHEN $4 IS NOT NULL AND LOWER($4) IS DISTINCT FROM LOWER(email) THEN false
                   ELSE email_verified
               END,
               updated_at = now()
           WHERE id = $5
           RETURNING *"#,
    )
    .bind(payload.first_name.map(|s| s.trim().to_string()))
    .bind(payload.last_name.map(|s| s.trim().to_string()))
    .bind(trimmed_username)
    .bind(trimmed_email)
    .bind(id)
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound("staff not found".into()))?;

    Ok(Json(staff))
}

#[utoipa::path(
    post,
    path = "/{id}/avatar",
    tag = "Admin Staff",
    security(("bearer_auth" = []), ("csrf_token" = [])),
    params(("id" = Uuid, Path, description = "Staff ID")),
    request_body(content = BinaryFileUpload, content_type = "multipart/form-data"),
    responses(
        (status = 200, description = "Staff avatar uploaded", body = Staff),
        (status = 400, description = "Invalid avatar upload", body = ErrorResponse),
        (status = 404, description = "Staff not found", body = ErrorResponse)
    )
)]
pub(crate) async fn upload_avatar(
    auth: AuthAdmin,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    mut multipart: Multipart,
) -> Result<Json<Staff>, ApiError> {
    let target_staff: Staff = sqlx::query_as("SELECT * FROM staff WHERE id = $1")
        .bind(id)
        .fetch_optional(&state.db)
        .await?
        .ok_or(ApiError::NotFound("staff not found".into()))?;

    if auth.0.sub != id {
        crate::require_permission!(auth, state, "staff.manage");
        let caller_perms = fetch_caller_permissions(&state.db, auth.0.sub).await?;
        let target_role: Role = sqlx::query_as("SELECT * FROM roles WHERE id = $1")
            .bind(target_staff.role_id)
            .fetch_optional(&state.db)
            .await?
            .ok_or(ApiError::Forbidden)?;
        ensure_role_assignable(&caller_perms, &target_role.permissions)?;
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
