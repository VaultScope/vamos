use axum::extract::{Path, Query, State};
use axum::Json;
use serde::Deserialize;
use utoipa::{IntoParams, ToSchema};
use utoipa_axum::{router::OpenApiRouter, routes};
use uuid::Uuid;

use crate::auth::AuthAdmin;
use crate::error::ApiError;
use crate::openapi::{ErrorResponse, OkMessageResponse};
use crate::state::AppState;
use vaultscope_db::models::{Customer, CustomerStatus, Role, Staff};

pub fn routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(list, create_customer))
        .routes(routes!(get_one, update, delete_customer))
        .routes(routes!(promote_to_staff))
        .routes(routes!(impersonate_customer))
        .routes(routes!(resend_verification_email))
        .routes(routes!(mark_email_verified))
}

#[derive(Deserialize, IntoParams)]
pub(crate) struct ListParams {
    status: Option<String>,
    search: Option<String>,
    limit: Option<i64>,
    offset: Option<i64>,
}

#[utoipa::path(
    get,
    path = "/",
    tag = "Admin Customers",
    security(("bearer_auth" = [])),
    params(ListParams),
    responses((status = 200, description = "List customers with active service counts", body = Vec<CustomerWithStats>))
)]
pub(crate) async fn list(
    auth: AuthAdmin,
    State(state): State<AppState>,
    Query(params): Query<ListParams>,
) -> Result<Json<Vec<CustomerWithStats>>, ApiError> {
    crate::require_permission!(auth, state, "customers.view");
    let limit = params.limit.unwrap_or(50).clamp(1, 500);
    let offset = params.offset.unwrap_or(0).max(0);

    let customers: Vec<Customer> = match (&params.status, &params.search) {
        (Some(status), Some(search)) => {
            let pattern = format!("%{}%", search);
            sqlx::query_as(
                "SELECT * FROM customers WHERE status = $1::customer_status AND (name ILIKE $2 OR email ILIKE $2) ORDER BY created_at DESC LIMIT $3 OFFSET $4"
            )
            .bind(status)
            .bind(&pattern)
            .bind(limit)
            .bind(offset)
            .fetch_all(&state.db)
            .await?
        }
        (Some(status), None) => {
            sqlx::query_as(
                "SELECT * FROM customers WHERE status = $1::customer_status ORDER BY created_at DESC LIMIT $2 OFFSET $3"
            )
            .bind(status)
            .bind(limit)
            .bind(offset)
            .fetch_all(&state.db)
            .await?
        }
        (None, Some(search)) => {
            let pattern = format!("%{}%", search);
            sqlx::query_as(
                "SELECT * FROM customers WHERE name ILIKE $1 OR email ILIKE $1 ORDER BY created_at DESC LIMIT $2 OFFSET $3"
            )
            .bind(&pattern)
            .bind(limit)
            .bind(offset)
            .fetch_all(&state.db)
            .await?
        }
        (None, None) => {
            sqlx::query_as("SELECT * FROM customers ORDER BY created_at DESC LIMIT $1 OFFSET $2")
                .bind(limit)
                .bind(offset)
                .fetch_all(&state.db)
                .await?
        }
    };

    let mut with_stats = Vec::new();
    for c in customers {
        let count: (i64,) = sqlx::query_as("SELECT count(*) FROM services WHERE customer_id = $1 AND status IN ('running', 'pending')")
            .bind(c.id)
            .fetch_one(&state.db).await.unwrap_or((0,));
        with_stats.push(CustomerWithStats {
            customer: c,
            active_services: count.0,
        });
    }

    Ok(Json(with_stats))
}

#[utoipa::path(
    get,
    path = "/{id}",
    tag = "Admin Customers",
    security(("bearer_auth" = [])),
    params(("id" = Uuid, Path, description = "Customer ID")),
    responses(
        (status = 200, description = "Customer details", body = Customer),
        (status = 404, description = "Customer not found", body = ErrorResponse)
    )
)]
pub(crate) async fn get_one(
    auth: AuthAdmin,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<Customer>, ApiError> {
    crate::require_permission!(auth, state, "customers.view");
    let customer: Customer = sqlx::query_as("SELECT * FROM customers WHERE id = $1")
        .bind(id)
        .fetch_optional(&state.db)
        .await?
        .ok_or(ApiError::NotFound("customer not found".into()))?;
    Ok(Json(customer))
}

#[derive(Deserialize, ToSchema)]
pub(crate) struct CreateCustomerPayload {
    name: String,
    email: String,
    username: Option<String>,
    phone: Option<String>,
    company: Option<String>,
    address: Option<String>,
    postal_code: Option<String>,
    city: Option<String>,
    country: Option<String>,
    vat_id: Option<String>,
    email_verified: Option<bool>,
}

#[utoipa::path(
    post,
    path = "/",
    tag = "Admin Customers",
    security(("bearer_auth" = []), ("csrf_token" = [])),
    request_body = CreateCustomerPayload,
    responses(
        (status = 200, description = "Customer created", body = Customer),
        (status = 400, description = "Validation error", body = ErrorResponse)
    )
)]
pub(crate) async fn create_customer(
    auth: AuthAdmin,
    State(state): State<AppState>,
    Json(payload): Json<CreateCustomerPayload>,
) -> Result<Json<Customer>, ApiError> {
    crate::require_permission!(auth, state, "customers.create");
    crate::validation::validate_required_string(&payload.name, "Name")?;
    crate::validation::validate_required_string(&payload.email, "Email")?;
    crate::validation::validate_email(payload.email.trim())?;

    let email = payload.email.trim().to_string();
    let raw_username = payload
        .username
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| {
            email
                .split('@')
                .next()
                .unwrap_or("customer")
                .chars()
                .filter(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-')
                .collect()
        });

    let existing: Option<(Uuid,)> = sqlx::query_as(
        "SELECT id FROM customers WHERE LOWER(email) = LOWER($1) OR (username <> '' AND LOWER(username) = LOWER($2)) LIMIT 1",
    )
    .bind(&email)
    .bind(&raw_username)
    .fetch_optional(&state.db)
    .await?;

    if existing.is_some() {
        return Err(ApiError::BadRequest(
            "A customer with this email or username already exists".into(),
        ));
    }

    let customer_id = Uuid::now_v7();
    let external_id = format!("local_{customer_id}");
    let country = payload
        .country
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("DE")
        .to_uppercase();

    let customer: Customer = sqlx::query_as(
        r#"INSERT INTO customers (
               id, external_id, username, name, email, phone, company, address, postal_code, city, country, vat_id, status, email_verified
           ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, 'active'::customer_status, $13)
           RETURNING *"#,
    )
    .bind(customer_id)
    .bind(external_id)
    .bind(raw_username)
    .bind(payload.name.trim())
    .bind(&email)
    .bind(payload.phone.as_deref().unwrap_or("").trim())
    .bind(payload.company.as_deref().unwrap_or("").trim())
    .bind(payload.address.as_deref().unwrap_or("").trim())
    .bind(payload.postal_code.as_deref().unwrap_or("").trim())
    .bind(payload.city.as_deref().unwrap_or("").trim())
    .bind(country)
    .bind(payload.vat_id.as_deref().unwrap_or("").trim().to_uppercase())
    .bind(payload.email_verified.unwrap_or(true))
    .fetch_one(&state.db)
    .await?;

    crate::activity::spawn_record(
        state.db.clone(),
        Some(auth.0.sub),
        vaultscope_db::models::ActorType::Staff,
        auth.0.email.clone(),
        "customer".to_string(),
        "created".to_string(),
        customer.email.clone(),
        format!("Admin created customer account {}", customer.name),
    );

    Ok(Json(customer))
}

#[derive(Deserialize, ToSchema)]
pub(crate) struct UpdateCustomer {
    name: Option<String>,
    email: Option<String>,
    username: Option<String>,
    phone: Option<String>,
    company: Option<String>,
    address: Option<String>,
    postal_code: Option<String>,
    city: Option<String>,
    country: Option<String>,
    vat_id: Option<String>,
    vat_verified: Option<bool>,
    status: Option<CustomerStatus>,
    notes: Option<String>,
    email_verified: Option<bool>,
}

#[utoipa::path(
    put,
    path = "/{id}",
    tag = "Admin Customers",
    security(("bearer_auth" = []), ("csrf_token" = [])),
    params(("id" = Uuid, Path, description = "Customer ID")),
    request_body = UpdateCustomer,
    responses(
        (status = 200, description = "Customer updated", body = Customer),
        (status = 404, description = "Customer not found", body = ErrorResponse)
    )
)]
pub(crate) async fn update(
    auth: AuthAdmin,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(payload): Json<UpdateCustomer>,
) -> Result<Json<Customer>, ApiError> {
    crate::require_permission!(auth, state, "customers.edit");
    if let Some(ref email) = payload.email {
        crate::validation::validate_email(email.trim())?;
    }
    let customer: Customer = sqlx::query_as(
        r#"UPDATE customers
           SET name = COALESCE(NULLIF(btrim($1), ''), name),
               email = COALESCE(NULLIF(btrim($2), ''), email),
               username = COALESCE($3, username),
               phone = COALESCE($4, phone),
               company = COALESCE($5, company),
               address = COALESCE($6, address),
               postal_code = COALESCE($7, postal_code),
               city = COALESCE($8, city),
               country = COALESCE($9, country),
               vat_id = COALESCE($10, vat_id),
               vat_verified = COALESCE($11, vat_verified),
               status = COALESCE($12::customer_status, status),
               notes = COALESCE($13, notes),
               email_verified = COALESCE($14, email_verified),
               updated_at = now()
           WHERE id = $15
           RETURNING *"#,
    )
    .bind(payload.name)
    .bind(payload.email)
    .bind(payload.username)
    .bind(payload.phone)
    .bind(payload.company)
    .bind(payload.address)
    .bind(payload.postal_code)
    .bind(payload.city)
    .bind(payload.country)
    .bind(payload.vat_id)
    .bind(payload.vat_verified)
    .bind(payload.status)
    .bind(payload.notes)
    .bind(payload.email_verified)
    .bind(id)
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound("customer not found".into()))?;

    crate::activity::spawn_record(
        state.db.clone(),
        Some(auth.0.sub),
        vaultscope_db::models::ActorType::Staff,
        auth.0.email.clone(),
        "customer".to_string(),
        "updated".to_string(),
        customer.email.clone(),
        format!("Updated customer profile for {}", customer.name),
    );

    Ok(Json(customer))
}

#[utoipa::path(
    post,
    path = "/{id}/impersonate",
    tag = "Admin Customers",
    security(("bearer_auth" = []), ("csrf_token" = [])),
    params(("id" = Uuid, Path, description = "Customer ID")),
    responses(
        (status = 200, description = "Short-lived Storefront impersonation session token and URL", body = serde_json::Value),
        (status = 403, description = "Forbidden", body = ErrorResponse),
        (status = 404, description = "Customer not found", body = ErrorResponse)
    )
)]
pub(crate) async fn impersonate_customer(
    auth: AuthAdmin,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, ApiError> {
    crate::require_permission!(auth, state, "customers.impersonate");

    let row: Option<(Customer, i64)> = sqlx::query_as::<_, Customer>("SELECT * FROM customers WHERE id = $1")
        .bind(id)
        .fetch_optional(&state.db)
        .await?
        .map(|c| (c, 1));

    let (customer, _) = row.ok_or_else(|| ApiError::NotFound("Customer not found".into()))?;
    let auth_version: i64 = sqlx::query_scalar("SELECT auth_version FROM customers WHERE id = $1")
        .bind(customer.id)
        .fetch_one(&state.db)
        .await
        .unwrap_or(1);

    let now = chrono::Utc::now().timestamp();
    let claims = crate::auth::Claims {
        sub: customer.id,
        email: customer.email.clone(),
        role: "customer".to_string(),
        audience: crate::auth::Audience::Storefront,
        iat: now,
        exp: now + 3600,
        auth_version: Some(auth_version),
        jti: Some(Uuid::now_v7().to_string()),
    };

    let token = crate::auth::jwt::sign(&claims, &state.config.jwt_secret)?;
    let storefront_base = state.config.storefront_url.trim_end_matches('/');
    let base_url = if storefront_base.is_empty() || storefront_base.contains("localhost") {
        "https://vaultscope.de"
    } else {
        storefront_base
    };
    let username = if !customer.username.trim().is_empty() {
        customer.username.clone()
    } else {
        customer.email.split('@').next().unwrap_or("customer").to_string()
    };
    fn pct_encode(s: &str) -> String {
        let mut out = String::with_capacity(s.len());
        for b in s.bytes() {
            if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
                out.push(b as char);
            } else {
                out.push_str(&format!("%{:02X}", b));
            }
        }
        out
    }
    let impersonation_url = format!(
        "{}/dashboard#impersonate_token={}&username={}&name={}",
        base_url,
        pct_encode(&token),
        pct_encode(&username),
        pct_encode(&customer.name),
    );

    crate::activity::spawn_record(
        state.db.clone(),
        Some(auth.0.sub),
        vaultscope_db::models::ActorType::Staff,
        auth.0.email.clone(),
        "security".to_string(),
        "impersonated".to_string(),
        customer.email.clone(),
        format!(
            "Staff {} initiated storefront impersonation session for customer {} ({})",
            auth.0.email, customer.name, customer.email
        ),
    );

    Ok(Json(serde_json::json!({
        "token": token,
        "claims": claims,
        "impersonation_url": impersonation_url,
        "customer": {
            "id": customer.id,
            "name": customer.name,
            "email": customer.email,
            "username": username,
        }
    })))
}

#[utoipa::path(
    post,
    path = "/{id}/resend-verification",
    tag = "Admin Customers",
    security(("bearer_auth" = []), ("csrf_token" = [])),
    params(("id" = Uuid, Path, description = "Customer ID")),
    responses(
        (status = 200, description = "Verification email resent", body = OkMessageResponse),
        (status = 404, description = "Customer not found", body = ErrorResponse)
    )
)]
pub(crate) async fn resend_verification_email(
    auth: AuthAdmin,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, ApiError> {
    crate::require_permission!(auth, state, "customers.edit");

    let customer: Customer = sqlx::query_as("SELECT * FROM customers WHERE id = $1")
        .bind(id)
        .fetch_optional(&state.db)
        .await?
        .ok_or_else(|| ApiError::NotFound("Customer not found".into()))?;

    let token_bytes: [u8; 32] = rand::random();
    let verification_token = hex::encode(token_bytes);

    sqlx::query(
        "UPDATE customers SET verification_token = $1, verification_token_expires_at = now() + interval '24 hours', updated_at = now() WHERE id = $2",
    )
    .bind(&verification_token)
    .bind(id)
    .execute(&state.db)
    .await?;

    crate::routes::auth::send_verification_email(
        &state.config,
        Some(state.db.clone()),
        &customer.email,
        &customer.name,
        &verification_token,
    );

    Ok(Json(serde_json::json!({
        "ok": true,
        "message": format!("Verification email sent to {}", customer.email)
    })))
}

#[utoipa::path(
    post,
    path = "/{id}/verify",
    tag = "Admin Customers",
    security(("bearer_auth" = []), ("csrf_token" = [])),
    params(("id" = Uuid, Path, description = "Customer ID")),
    responses(
        (status = 200, description = "Customer email marked verified", body = Customer),
        (status = 404, description = "Customer not found", body = ErrorResponse)
    )
)]
pub(crate) async fn mark_email_verified(
    auth: AuthAdmin,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<Customer>, ApiError> {
    crate::require_permission!(auth, state, "customers.edit");

    let customer: Customer = sqlx::query_as(
        r#"UPDATE customers
           SET email_verified = true,
               verification_token = NULL,
               verification_token_expires_at = NULL,
               updated_at = now()
           WHERE id = $1
           RETURNING *"#
    )
    .bind(id)
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| ApiError::NotFound("Customer not found".into()))?;

    let _ = sqlx::query(
        "UPDATE staff SET email_verified = true, updated_at = now() WHERE LOWER(email) = LOWER($1) OR external_id = $2",
    )
    .bind(&customer.email)
    .bind(format!("customer_{id}"))
    .execute(&state.db)
    .await;

    Ok(Json(customer))
}

#[derive(serde::Serialize, ToSchema)]
pub struct CustomerWithStats {
    #[serde(flatten)]
    pub customer: Customer,
    pub active_services: i64,
}

#[derive(Deserialize, ToSchema)]
pub struct PromoteCustomerPayload {
    pub role_id: Uuid,
    pub username: Option<String>,
    pub password: Option<String>,
}

#[utoipa::path(
    post,
    path = "/{id}/promote",
    tag = "Admin Customers",
    security(("bearer_auth" = []), ("csrf_token" = [])),
    params(("id" = Uuid, Path, description = "Customer ID")),
    request_body = PromoteCustomerPayload,
    responses(
        (status = 200, description = "Customer promoted to staff", body = Staff),
        (status = 400, description = "Invalid role ID or password", body = ErrorResponse),
        (status = 403, description = "Insufficient permissions to assign target role", body = ErrorResponse),
        (status = 404, description = "Customer not found", body = ErrorResponse)
    )
)]
pub(crate) async fn promote_to_staff(
    auth: AuthAdmin,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(payload): Json<PromoteCustomerPayload>,
) -> Result<Json<Staff>, ApiError> {
    crate::require_permission!(auth, state, "staff.manage");

    // 1. Validate caller's RBAC privilege to assign the target role
    let caller_staff: Staff = sqlx::query_as("SELECT * FROM staff WHERE id = $1")
        .bind(auth.0.sub)
        .fetch_optional(&state.db)
        .await?
        .ok_or(ApiError::Unauthorized)?;
    let caller_role: Role = sqlx::query_as("SELECT * FROM roles WHERE id = $1")
        .bind(caller_staff.role_id)
        .fetch_optional(&state.db)
        .await?
        .ok_or(ApiError::Forbidden)?;

    let target_role: Role = sqlx::query_as("SELECT * FROM roles WHERE id = $1")
        .bind(payload.role_id)
        .fetch_optional(&state.db)
        .await?
        .ok_or_else(|| ApiError::BadRequest("Invalid role_id".into()))?;

    let caller_is_owner = caller_role.permissions.iter().any(|p| p == "*");
    let target_is_owner = target_role.permissions.iter().any(|p| p == "*")
        || target_role.name.eq_ignore_ascii_case("Owner")
        || target_role.name.eq_ignore_ascii_case("Super Admin");

    if target_is_owner && !caller_is_owner {
        return Err(ApiError::Forbidden);
    }
    crate::routes::admin::staff::ensure_role_assignable(
        &caller_role.permissions,
        &target_role.permissions,
    )?;

    // Validate optional password up front before touching DB state
    let validated_password = match payload.password {
        Some(pw) if !pw.is_empty() => {
            if pw.trim().is_empty() || pw.chars().count() < 8 || pw.len() > 72 {
                return Err(ApiError::BadRequest(
                    "Password must be between 8 and 72 characters".into(),
                ));
            }
            Some(pw)
        }
        _ => None,
    };

    // 2. Fetch customer
    let customer: Customer = sqlx::query_as("SELECT * FROM customers WHERE id = $1")
        .bind(id)
        .fetch_optional(&state.db)
        .await?
        .ok_or_else(|| ApiError::NotFound("Customer not found".into()))?;

    // Hash password if explicitly supplied, and keep customer + staff credentials shared
    let explicit_password_hash = match validated_password {
        Some(pw) => {
            let hashed =
                tokio::task::spawn_blocking(move || bcrypt::hash(pw, bcrypt::DEFAULT_COST))
                    .await
                    .map_err(|e| ApiError::Internal(e.to_string()))?
                    .map_err(|e| ApiError::Internal(e.to_string()))?;
            sqlx::query("UPDATE customers SET password_hash = COALESCE(password_hash, $1), updated_at = now() WHERE id = $2")
                .bind(&hashed)
                .bind(customer.id)
                .execute(&state.db)
                .await?;
            Some(hashed)
        }
        None => None,
    };

    let shared_password_hash = explicit_password_hash.or_else(|| customer.password_hash.clone());
    let customer_ext_link = format!("customer_{}", customer.id);

    // 3. Check if staff already exists with this email or customer external_id
    let existing_staff: Option<Staff> = sqlx::query_as(
        "SELECT * FROM staff WHERE LOWER(email) = LOWER($1) OR external_id = $2 LIMIT 1",
    )
    .bind(&customer.email)
    .bind(&customer_ext_link)
    .fetch_optional(&state.db)
    .await?;

    if let Some(existing) = existing_staff {
        if existing.id == auth.0.sub && existing.role_id != payload.role_id {
            return Err(ApiError::Forbidden);
        }
        if !caller_is_owner {
            let existing_role: Option<Role> =
                sqlx::query_as("SELECT * FROM roles WHERE id = $1")
                    .bind(existing.role_id)
                    .fetch_optional(&state.db)
                    .await?;
            if let Some(ref er) = existing_role {
                let existing_is_owner = er.permissions.iter().any(|p| p == "*")
                    || er.name.eq_ignore_ascii_case("Owner")
                    || er.name.eq_ignore_ascii_case("Super Admin");
                if existing_is_owner {
                    return Err(ApiError::Forbidden);
                }
                crate::routes::admin::staff::ensure_role_assignable(
                    &caller_role.permissions,
                    &er.permissions,
                )?;
            }
        }
        let updated: Staff = sqlx::query_as(
            r#"UPDATE staff
               SET role_id = $1,
                   password_hash = COALESCE($2, password_hash),
                   email_verified = email_verified OR $3,
                   mfa_enabled = mfa_enabled OR $4,
                   totp_secret = CASE
                       WHEN $4 AND $5 IS NOT NULL AND length(trim($5)) > 0 THEN $5
                       ELSE COALESCE(NULLIF(trim(totp_secret), ''), $5)
                   END,
                   auth_version = auth_version + 1,
                   updated_at = now()
               WHERE id = $6
               RETURNING *"#,
        )
        .bind(payload.role_id)
        .bind(&shared_password_hash)
        .bind(customer.email_verified)
        .bind(customer.two_factor_enabled)
        .bind(&customer.totp_secret)
        .bind(existing.id)
        .fetch_one(&state.db)
        .await?;
        return Ok(Json(updated));
    }

    // 4. Generate clean username
    let raw_username = payload.username.unwrap_or_else(|| {
        if !customer.username.trim().is_empty() {
            customer.username.clone()
        } else {
            customer.email.split('@').next().unwrap_or("staff").to_string()
        }
    });
    let sanitized: String = raw_username
        .chars()
        .filter(|c| c.is_alphanumeric() || *c == '_' || *c == '-')
        .collect();
    let username = if sanitized.is_empty() {
        format!("staff_{}", &id.to_string()[..8])
    } else {
        sanitized
    };

    // 5. Split customer full name into first and last name
    let mut parts = customer.name.split_whitespace();
    let first_name = parts.next().unwrap_or("").to_string();
    let last_name = parts.collect::<Vec<_>>().join(" ");

    // 6. Insert new staff row sharing credentials & 2FA from the customer account
    let staff: Staff = sqlx::query_as(
        r#"INSERT INTO staff (
            id, external_id, first_name, last_name, username, email, email_verified, role_id, password_hash, mfa_enabled, totp_secret
        ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
        RETURNING *"#,
    )
    .bind(Uuid::now_v7())
    .bind(customer_ext_link)
    .bind(first_name)
    .bind(last_name)
    .bind(username)
    .bind(customer.email)
    .bind(customer.email_verified)
    .bind(payload.role_id)
    .bind(shared_password_hash)
    .bind(customer.two_factor_enabled)
    .bind(customer.totp_secret)
    .fetch_one(&state.db)
    .await?;

    Ok(Json(staff))
}

#[utoipa::path(
    delete,
    path = "/{id}",
    tag = "Admin Customers",
    security(("bearer_auth" = []), ("csrf_token" = [])),
    params(("id" = Uuid, Path, description = "Customer ID")),
    responses(
        (status = 200, description = "Customer account deleted", body = OkMessageResponse),
        (status = 404, description = "Customer not found", body = ErrorResponse)
    )
)]
pub(crate) async fn delete_customer(
    auth: AuthAdmin,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, ApiError> {
    if crate::middleware::rbac::check_permission(&auth, &state, "customers.delete").await.is_err()
        && crate::middleware::rbac::check_permission(&auth, &state, "accounts.delete").await.is_err()
    {
        crate::require_permission!(auth, state, "customers.manage");
    }

    let mut tx = state.db.begin().await?;

    let customer: Customer = sqlx::query_as("SELECT * FROM customers WHERE id = $1")
        .bind(id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(|| ApiError::NotFound("Customer not found".into()))?;

    // Queue 'delete' provisioning jobs for any provisioned services before removing service rows
    #[derive(sqlx::FromRow)]
    struct ProvisionedServiceRow {
        id: Uuid,
        provider_resource_id: String,
        provider: Option<String>,
    }

    let provisioned_services: Vec<ProvisionedServiceRow> = sqlx::query_as(
        r#"SELECT s.id, s.provider_resource_id, p.provider
           FROM services s
           LEFT JOIN products p ON p.id = s.product_id
           WHERE s.customer_id = $1 AND btrim(s.provider_resource_id) <> ''"#,
    )
    .bind(id)
    .fetch_all(&mut *tx)
    .await?;

    for svc in provisioned_services {
        let target_api = svc
            .provider
            .filter(|p| !p.trim().is_empty())
            .unwrap_or_else(|| "sftpgo".to_string());
        let delete_payload = serde_json::json!({
            "resource_id": svc.provider_resource_id
        });
        sqlx::query(
            r#"INSERT INTO jobs (id, task, target_api, customer_id, service_id, request_payload)
               VALUES ($1, 'delete', $2, $3, $4, $5)"#,
        )
        .bind(Uuid::now_v7())
        .bind(target_api)
        .bind(id)
        .bind(svc.id)
        .bind(&delete_payload)
        .execute(&mut *tx)
        .await?;
    }

    // Delete secondary billing emails
    sqlx::query("DELETE FROM customer_billing_emails WHERE customer_id = $1")
        .bind(id)
        .execute(&mut *tx)
        .await?;

    // Delete tickets and ticket messages
    sqlx::query(
        "DELETE FROM ticket_messages WHERE ticket_id IN (SELECT id FROM tickets WHERE customer_id = $1)"
    )
    .bind(id)
    .execute(&mut *tx)
    .await?;
    sqlx::query("DELETE FROM tickets WHERE customer_id = $1")
        .bind(id)
        .execute(&mut *tx)
        .await?;

    // Preserve statutory non-draft invoices and their line items; delete only drafts
    sqlx::query(
        "DELETE FROM invoice_line_items WHERE invoice_id IN (SELECT id FROM invoices WHERE customer_id = $1 AND status = 'draft'::invoice_status)"
    )
    .bind(id)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "DELETE FROM invoices WHERE customer_id = $1 AND status = 'draft'::invoice_status"
    )
    .bind(id)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "UPDATE invoices SET customer_id = NULL, updated_at = now() WHERE customer_id = $1 AND status <> 'draft'::invoice_status"
    )
    .bind(id)
    .execute(&mut *tx)
    .await?;

    // Delete services (ON DELETE SET NULL keeps queued deletion jobs and retained line items valid)
    sqlx::query("DELETE FROM services WHERE customer_id = $1")
        .bind(id)
        .execute(&mut *tx)
        .await?;

    // Unlink jobs from deleted customer while preserving job history and queued delete tasks
    sqlx::query("UPDATE jobs SET customer_id = NULL WHERE customer_id = $1")
        .bind(id)
        .execute(&mut *tx)
        .await?;

    // Delete customer
    let res = sqlx::query("DELETE FROM customers WHERE id = $1")
        .bind(id)
        .execute(&mut *tx)
        .await?;

    if res.rows_affected() == 0 {
        return Err(ApiError::NotFound("Customer not found".into()));
    }

    tx.commit().await?;

    crate::activity::spawn_record(
        state.db.clone(),
        Some(auth.0.sub),
        vaultscope_db::models::ActorType::Staff,
        auth.0.email.clone(),
        "customer".to_string(),
        "deleted_customer_account".to_string(),
        customer.email.clone(),
        format!("Deleted customer account {} ({})", customer.name, customer.email),
    );

    Ok(Json(serde_json::json!({
        "ok": true,
        "message": format!("Customer account for {} ({}) deleted successfully", customer.name, customer.email)
    })))
}

