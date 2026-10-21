use axum::extract::State;
use axum::routing::post;
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::auth::AuthCustomer;
use crate::error::ApiError;
use crate::state::AppState;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/", post(place_order))
        .route("/create-payment-intent", post(create_payment_intent))
}

#[derive(Deserialize)]
struct CreatePaymentIntentPayload {
    product_id: Uuid,
}

async fn create_payment_intent(
    _auth: AuthCustomer,
    State(state): State<AppState>,
    Json(payload): Json<CreatePaymentIntentPayload>,
) -> Result<Json<Value>, ApiError> {
    let product = sqlx::query_as::<_, vaultscope_db::models::Product>(
        "SELECT * FROM products WHERE id = $1 AND hidden = false",
    )
    .bind(payload.product_id)
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound("product not found".into()))?;

    let stripe_secret = state
        .config
        .stripe_secret_key
        .as_deref()
        .ok_or(ApiError::Internal("Stripe not configured".into()))?;

    use rust_decimal::prelude::ToPrimitive;
    let price_cents = (product.price * rust_decimal::Decimal::from(100))
        .to_i64()
        .unwrap_or(0);

    let vat_cents = price_cents * 19 / 100;
    let total_cents = price_cents + vat_cents;
    let amount = total_cents.max(50);

    let client = reqwest::Client::new();
    let resp = client
        .post("https://api.stripe.com/v1/payment_intents")
        .basic_auth(stripe_secret, Option::<&str>::None)
        .form(&[
            ("amount", amount.to_string()),
            ("currency", "eur".to_string()),
            ("automatic_payment_methods[enabled]", "true".to_string()),
        ])
        .send()
        .await
        .map_err(|e| ApiError::Internal(format!("Stripe request failed: {e}")))?;

    if !resp.status().is_success() {
        let body = resp.text().await.unwrap_or_default();
        return Err(ApiError::Internal(format!("Stripe error: {body}")));
    }

    let body: Value = resp
        .json()
        .await
        .map_err(|e| ApiError::Internal(format!("Stripe parse error: {e}")))?;

    let client_secret = body["client_secret"]
        .as_str()
        .ok_or(ApiError::Internal("Missing client_secret from Stripe".into()))?;
    let payment_intent_id = body["id"].as_str().unwrap_or("");

    Ok(Json(json!({
        "client_secret": client_secret,
        "payment_intent_id": payment_intent_id
    })))
}

#[derive(Deserialize)]
struct PlaceOrder {
    product_id: Uuid,
    hostname: Option<String>,
    payment_intent_id: Option<String>,
    partner_slug: Option<String>,
    config: Option<Value>,
}

async fn place_order(
    auth: AuthCustomer,
    State(state): State<AppState>,
    Json(payload): Json<PlaceOrder>,
) -> Result<Json<Value>, ApiError> {
    let product = sqlx::query_as::<_, vaultscope_db::models::Product>(
        "SELECT * FROM products WHERE id = $1 AND hidden = false",
    )
    .bind(payload.product_id)
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound("product not found".into()))?;

    let (partner_id, partner_trial_override, partner_trial_quota_override, partner_quota_override, partner_limit_one_per_user): 
        (Option<Uuid>, Option<i32>, Option<i32>, Option<i32>, Option<bool>) = if let Some(ref slug) = payload.partner_slug {
        let _ = sqlx::query("UPDATE partners SET conversions = conversions + 1 WHERE LOWER(slug) = LOWER($1)")
            .bind(slug.trim())
            .execute(&state.db)
            .await;

        let row: Option<(Uuid, Option<i32>, Option<i32>, Option<i32>, bool)> = sqlx::query_as(
            "SELECT id, trial_days_override, trial_quota_gb_override, quota_gb_override, limit_one_per_user FROM partners WHERE LOWER(slug) = LOWER($1) AND active = true"
        )
        .bind(slug.trim())
        .fetch_optional(&state.db)
        .await
        .ok()
        .flatten();

        match row {
            Some((pid, trial, t_quota, quota, limit)) => (Some(pid), trial, t_quota, quota, Some(limit)),
            None => (None, None, None, None, None),
        }
    } else {
        (None, None, None, None, None)
    };

    let effective_trial_days = partner_trial_override.or(product.trial_days);

    use rust_decimal::prelude::ToPrimitive;
    let price_cents = (product.price * rust_decimal::Decimal::from(100))
        .to_i64()
        .unwrap_or(0);
    let vat_cents = price_cents * 19 / 100;
    let total_cents = price_cents + vat_cents;

    let has_trial = effective_trial_days.unwrap_or(0) > 0;
    let is_storage = product.category.to_lowercase().contains("sftp")
        || product.category.to_lowercase().contains("storage")
        || product.name.to_lowercase().contains("sftp")
        || product.provider == "sftpgo";

    // Enforce limit: check if campaign / trial can only be claimed once per customer
    let is_single_claim_limited = partner_limit_one_per_user.unwrap_or(true);
    if has_trial && is_single_claim_limited {
        let existing_trials: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM services WHERE customer_id = $1 AND is_trial = true"
        )
        .bind(auth.0.sub)
        .fetch_one(&state.db)
        .await?;

        if existing_trials >= 1 {
            return Err(ApiError::BadRequest(
                "This partner campaign can only be claimed once per user. You have already claimed a promotional trial.".into(),
            ));
        }
    }

    let is_free = has_trial || product.category == "sftp" || product.price <= rust_decimal::Decimal::ZERO;

    let verified_payment_intent_id: Option<String> = if is_free {
        None
    } else {
        let pi_id = payload.payment_intent_id.ok_or_else(|| {
            ApiError::BadRequest("payment_intent_id required for paid services".into())
        })?;

        // 1. Ensure the payment intent has not been redeemed already
        let existing: Option<(Uuid,)> = sqlx::query_as(
            "SELECT id FROM services WHERE stripe_payment_intent_id = $1"
        )
        .bind(&pi_id)
        .fetch_optional(&state.db)
        .await?;

        if existing.is_some() {
            return Err(ApiError::BadRequest("This payment intent has already been used".into()));
        }

        // 2. Verify with Stripe API (fail-closed for paid services)
        let stripe_secret = state.config.stripe_secret_key.as_deref().ok_or_else(|| {
            ApiError::Internal("Payment processing is not configured for paid services.".into())
        })?;

        let client = reqwest::Client::new();
        let resp = client
            .get(format!("https://api.stripe.com/v1/payment_intents/{}", pi_id))
            .basic_auth(stripe_secret, Option::<&str>::None)
            .send()
            .await
            .map_err(|e| ApiError::Internal(format!("Stripe verification failed: {e}")))?;

        if !resp.status().is_success() {
            let err_text = resp.text().await.unwrap_or_default();
            return Err(ApiError::BadRequest(format!("Invalid payment intent: {err_text}")));
        }

        let pi_data: Value = resp
            .json()
            .await
            .map_err(|e| ApiError::Internal(format!("Failed to parse Stripe response: {e}")))?;

        let status = pi_data["status"].as_str().unwrap_or("");
        if status != "succeeded" {
            return Err(ApiError::BadRequest(format!("Payment has not succeeded yet (status: {status})")));
        }

        let amount_received = pi_data["amount_received"].as_i64().unwrap_or(0);
        if amount_received < total_cents.max(50) {
            return Err(ApiError::BadRequest(format!(
                "Payment amount insufficient (received {amount_received}, expected at least {})",
                total_cents.max(50)
            )));
        }

        Some(pi_id)
    };

    let service_id = Uuid::now_v7();
    let hostname = payload
        .hostname
        .filter(|h| !h.trim().is_empty())
        .unwrap_or_else(|| {
            if is_storage {
                "sftp.vaultscope.de".to_string()
            } else {
                format!("srv-{}", &service_id.to_string()[..8])
            }
        });

    let is_trial = has_trial || (is_storage && product.price <= rust_decimal::Decimal::ZERO);
    let trial_days_count = if has_trial {
        effective_trial_days.unwrap() as i64
    } else {
        30
    };

    let (initial_price, next_due, trial_ends_at) = if is_trial {
        let expiry = chrono::Utc::now() + chrono::Duration::days(trial_days_count);
        (rust_decimal::Decimal::ZERO, Some(expiry), Some(expiry))
    } else {
        (product.price, Some(chrono::Utc::now() + chrono::Duration::days(30)), None)
    };

    let effective_trial_quota_gb = partner_trial_quota_override
        .or(product.trial_quota_gb)
        .unwrap_or(25);

    let effective_post_trial_quota_gb = product.quota_gb
        .or(partner_quota_override)
        .or_else(|| product.specs.get("storage_gb").and_then(|v| v.as_i64()).map(|v| v as i32))
        .unwrap_or(150);

    let current_quota_gb = if is_trial {
        effective_trial_quota_gb
    } else {
        effective_post_trial_quota_gb
    };

    let quota_size: u64 = if is_storage {
        (current_quota_gb as u64) * 1024 * 1024 * 1024
    } else {
        0_u64
    };

    sqlx::query(
        r#"INSERT INTO services (id, customer_id, product_id, name, hostname, price, status, next_due, stripe_payment_intent_id, is_trial, trial_ends_at, partner_id, quota_gb)
        VALUES ($1, $2, $3, $4, $5, $6, 'pending', $7, $8, $9, $10, $11, $12)"#,
    )
    .bind(service_id)
    .bind(auth.0.sub)
    .bind(product.id)
    .bind(&product.name)
    .bind(&hostname)
    .bind(initial_price)
    .bind(next_due)
    .bind(&verified_payment_intent_id)
    .bind(is_trial)
    .bind(trial_ends_at)
    .bind(partner_id)
    .bind(current_quota_gb)
    .execute(&state.db)
    .await?;

    // Create paid invoice record if payment occurred
    if let Some(ref pi_id) = verified_payment_intent_id {
        let invoice_id = Uuid::now_v7();
        let inv_num = format!("INV-{}-{}", chrono::Utc::now().format("%Y%m"), &invoice_id.to_string()[..6].to_uppercase());
        let subtotal = product.price;
        let tax_amount = product.price * rust_decimal::Decimal::new(19, 2);
        let total = subtotal + tax_amount;
        let today = chrono::Utc::now().date_naive();

        let _ = sqlx::query(
            r#"INSERT INTO invoices (id, invoice_number, customer_id, status, subtotal, tax_rate, tax_amount, total, issue_date, due_date, paid_at, stripe_payment_intent_id, notes)
            VALUES ($1, $2, $3, 'paid', $4, 19.00, $5, $6, $7, $7, now(), $8, $9)"#
        )
        .bind(invoice_id)
        .bind(inv_num)
        .bind(auth.0.sub)
        .bind(subtotal)
        .bind(tax_amount)
        .bind(total)
        .bind(today)
        .bind(pi_id)
        .bind(format!("Order for {}", product.name))
        .execute(&state.db)
        .await;
    }

    let job_id = Uuid::now_v7();
    let custom_image = payload
        .config
        .as_ref()
        .and_then(|c| c.get("image").or_else(|| c.get("os_image")))
        .and_then(|v| v.as_str())
        .or_else(|| product.specs.get("image").and_then(|v| v.as_str()))
        .unwrap_or("ubuntu-22.04");

    let provision_payload = if is_storage {
        serde_json::json!({
            "name": hostname,
            "quota_size": quota_size,
            "extra": {
                "customer_id": auth.0.sub.to_string(),
                "is_trial": is_trial,
                "quota_size": quota_size,
                "specs": product.specs,
                "config": payload.config
            }
        })
    } else {
        serde_json::json!({
            "server_type": product.specs.get("server_type").and_then(|v| v.as_str()).unwrap_or("cx22"),
            "location": product.specs.get("location").and_then(|v| v.as_str()).unwrap_or("fsn1"),
            "image": custom_image,
            "ssh_keys": [],
            "name": hostname,
            "extra": {
                "customer_id": auth.0.sub.to_string(),
                "specs": product.specs,
                "config": payload.config
            }
        })
    };

    sqlx::query(
        r#"INSERT INTO jobs (id, task, target_api, customer_id, service_id, request_payload)
        VALUES ($1, 'provision', $2, $3, $4, $5)"#
    )
    .bind(job_id)
    .bind(&product.provider)
    .bind(auth.0.sub)
    .bind(service_id)
    .bind(&provision_payload)
    .execute(&state.db)
    .await?;

    Ok(Json(
        json!({ "service_id": service_id, "job_id": job_id, "status": "pending" }),
    ))
}
