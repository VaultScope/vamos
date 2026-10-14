use axum::extract::{Path, Query, State};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use uuid::Uuid;

use crate::auth::AuthAdmin;
use crate::error::ApiError;
use crate::state::AppState;
use vaultscope_db::models::{Customer, CustomerStatus, Staff};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/", get(list))
        .route("/{id}", get(get_one).put(update).delete(delete_customer))
        .route("/{id}/promote", post(promote_to_staff))
        .route("/{id}/resend-verification", post(resend_verification_email))
        .route("/{id}/verify", post(mark_email_verified))
}

#[derive(Deserialize)]
struct ListParams {
    status: Option<String>,
    search: Option<String>,
    limit: Option<i64>,
    offset: Option<i64>,
}

async fn list(
    auth: AuthAdmin,
    State(state): State<AppState>,
    Query(params): Query<ListParams>,
) -> Result<Json<Vec<CustomerWithStats>>, ApiError> {
    crate::require_permission!(auth, state, "customers.view");
    let limit = params.limit.unwrap_or(50).min(200);
    let offset = params.offset.unwrap_or(0);

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

async fn get_one(
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

#[derive(Deserialize)]
struct UpdateCustomer {
    status: Option<CustomerStatus>,
    notes: Option<String>,
    email_verified: Option<bool>,
}

async fn update(
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

async fn resend_verification_email(
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

    let base = state.config.storefront_url.trim_end_matches('/');
    let verification_link = format!("{}/verify-email?token={}", base, verification_token);

    crate::routes::auth::send_verification_email(
        &state.config,
        Some(state.db.clone()),
        &customer.email,
        &customer.name,
        &verification_token,
    );

    Ok(Json(serde_json::json!({
        "ok": true,
        "message": format!("Verification email sent to {}", customer.email),
        "verification_token": verification_token,
        "verification_link": verification_link
    })))
}

async fn mark_email_verified(
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

#[derive(serde::Serialize)]
pub struct CustomerWithStats {
    #[serde(flatten)]
    pub customer: Customer,
    pub active_services: i64,
}

#[derive(Deserialize)]
pub struct PromoteCustomerPayload {
    pub role_id: Uuid,
    pub username: Option<String>,
    pub password: Option<String>,
}

async fn promote_to_staff(
    auth: AuthAdmin,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(payload): Json<PromoteCustomerPayload>,
) -> Result<Json<Staff>, ApiError> {
    crate::require_permission!(auth, state, "staff.manage");

    // 1. Fetch customer
    let customer: Customer = sqlx::query_as("SELECT * FROM customers WHERE id = $1")
        .bind(id)
        .fetch_optional(&state.db)
        .await?
        .ok_or_else(|| ApiError::NotFound("Customer not found".into()))?;

    // 2. Validate role
    let role_exists: Option<(Uuid,)> = sqlx::query_as("SELECT id FROM roles WHERE id = $1")
        .bind(payload.role_id)
        .fetch_optional(&state.db)
        .await?;
    if role_exists.is_none() {
        return Err(ApiError::BadRequest("Invalid role_id".into()));
    }

    // 3. Check if staff already exists with this email
    let existing_staff: Option<Staff> = sqlx::query_as("SELECT * FROM staff WHERE email = $1")
        .bind(&customer.email)
        .fetch_optional(&state.db)
        .await?;

    if let Some(existing) = existing_staff {
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

    // 6. Handle password hash
    let password_hash = match payload.password {
        Some(pw) if !pw.trim().is_empty() => {
            Some(bcrypt::hash(pw, bcrypt::DEFAULT_COST).map_err(|e| ApiError::Internal(e.to_string()))?)
        }
        _ => customer.password_hash,
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

async fn delete_customer(
    auth: AuthAdmin,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, ApiError> {
    if crate::middleware::rbac::check_permission(&auth, &state, "customers.delete").await.is_err()
        && crate::middleware::rbac::check_permission(&auth, &state, "accounts.delete").await.is_err()
    {
        crate::require_permission!(auth, state, "customers.manage");
    }

    let customer: Customer = sqlx::query_as("SELECT * FROM customers WHERE id = $1")
        .bind(id)
        .fetch_optional(&state.db)
        .await?
        .ok_or_else(|| ApiError::NotFound("Customer not found".into()))?;

    // Delete secondary billing emails
    let _ = sqlx::query("DELETE FROM customer_billing_emails WHERE customer_id = $1")
        .bind(id)
        .execute(&state.db)
        .await;

    // Delete tickets and ticket messages
    let _ = sqlx::query(
        "DELETE FROM ticket_messages WHERE ticket_id IN (SELECT id FROM tickets WHERE customer_id = $1)"
    )
    .bind(id)
    .execute(&state.db)
    .await;
    let _ = sqlx::query("DELETE FROM tickets WHERE customer_id = $1")
        .bind(id)
        .execute(&state.db)
        .await;

    // Delete invoices and invoice line items
    let _ = sqlx::query(
        "DELETE FROM invoice_line_items WHERE invoice_id IN (SELECT id FROM invoices WHERE customer_id = $1)"
    )
    .bind(id)
    .execute(&state.db)
    .await;
    let _ = sqlx::query("DELETE FROM invoices WHERE customer_id = $1")
        .bind(id)
        .execute(&state.db)
        .await;

    // Delete services
    let _ = sqlx::query("DELETE FROM services WHERE customer_id = $1")
        .bind(id)
        .execute(&state.db)
        .await;

    // Unlink jobs
    let _ = sqlx::query("UPDATE jobs SET customer_id = NULL WHERE customer_id = $1")
        .bind(id)
        .execute(&state.db)
        .await;

    // Delete customer
    let res = sqlx::query("DELETE FROM customers WHERE id = $1")
        .bind(id)
        .execute(&state.db)
        .await?;

    if res.rows_affected() == 0 {
        return Err(ApiError::NotFound("Customer not found".into()));
    }

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

