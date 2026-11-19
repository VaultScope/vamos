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
        .routes(routes!(list))
        .routes(routes!(get_one, update, delete_customer))
        .routes(routes!(promote_to_staff))
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
pub(crate) struct UpdateCustomer {
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
    let customer: Customer = sqlx::query_as(
        r#"UPDATE customers
           SET status = COALESCE($1::customer_status, status),
               notes = COALESCE($2, notes),
               email_verified = COALESCE($3, email_verified),
               updated_at = now()
           WHERE id = $4
           RETURNING *"#,
    )
    .bind(payload.status)
    .bind(payload.notes)
    .bind(payload.email_verified)
    .bind(id)
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound("customer not found".into()))?;

    Ok(Json(customer))
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

    sqlx::query("UPDATE customers SET verification_token = $1, updated_at = now() WHERE id = $2")
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
               updated_at = now()
           WHERE id = $1
           RETURNING *"#
    )
    .bind(id)
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| ApiError::NotFound("Customer not found".into()))?;

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
    let caller_has_rbac_manage = caller_role.permissions.iter().any(|p| p == "rbac.manage");
    let target_is_owner = target_role.permissions.iter().any(|p| p == "*")
        || target_role.name.eq_ignore_ascii_case("Owner")
        || target_role.name.eq_ignore_ascii_case("Super Admin");

    if target_is_owner && !caller_is_owner && !caller_has_rbac_manage {
        return Err(ApiError::Forbidden);
    }
    if !caller_is_owner && !caller_has_rbac_manage {
        let has_all_target_perms = target_role
            .permissions
            .iter()
            .all(|perm| caller_role.permissions.contains(perm));
        if !has_all_target_perms {
            return Err(ApiError::Forbidden);
        }
    }

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

    // 3. Check if staff already exists with this email
    let existing_staff: Option<Staff> = sqlx::query_as("SELECT * FROM staff WHERE email = $1")
        .bind(&customer.email)
        .fetch_optional(&state.db)
        .await?;

    if let Some(existing) = existing_staff {
        if !caller_is_owner && !caller_has_rbac_manage {
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
            }
        }
        let updated: Staff = sqlx::query_as(
            "UPDATE staff SET role_id = $1, updated_at = now() WHERE id = $2 RETURNING *"
        )
        .bind(payload.role_id)
        .bind(existing.id)
        .fetch_one(&state.db)
        .await?;
        return Ok(Json(updated));
    }

    // 4. Generate clean username
    let raw_username = payload.username.unwrap_or_else(|| {
        customer.email.split('@').next().unwrap_or("staff").to_string()
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

    // 6. Handle password hash on blocking threadpool
    let password_hash = match validated_password {
        Some(pw) => {
            let hashed =
                tokio::task::spawn_blocking(move || bcrypt::hash(pw, bcrypt::DEFAULT_COST))
                    .await
                    .map_err(|e| ApiError::Internal(e.to_string()))?
                    .map_err(|e| ApiError::Internal(e.to_string()))?;
            Some(hashed)
        }
        None => customer.password_hash,
    };

    // 7. Insert new staff row
    let staff: Staff = sqlx::query_as(
        r#"INSERT INTO staff (
            id, external_id, first_name, last_name, username, email, email_verified, role_id, password_hash
        ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
        RETURNING *"#
    )
    .bind(Uuid::now_v7())
    .bind(format!("customer_{}", customer.id))
    .bind(first_name)
    .bind(last_name)
    .bind(username)
    .bind(customer.email)
    .bind(customer.email_verified)
    .bind(payload.role_id)
    .bind(password_hash)
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

