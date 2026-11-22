use axum::{
    extract::{Request, State},
    http::{header, StatusCode},
    response::IntoResponse,
};
use chrono::Utc;
use hmac::{Hmac, Mac};
use rust_decimal::prelude::ToPrimitive;
use rust_decimal::Decimal;
use serde_json::Value;
use sha2::Sha256;
use subtle::ConstantTimeEq;
use tracing::{error, info, warn};
use uuid::Uuid;

use crate::state::AppState;
use vaultscope_db::models::InvoiceStatus;

type HmacSha256 = Hmac<Sha256>;

const SIGNATURE_TOLERANCE_SECS: i64 = 300;
pub const MAX_WEBHOOK_BODY_BYTES: usize = 1024 * 1024;

pub fn expected_invoice_cents(total: Decimal) -> i64 {
    (total * Decimal::from(100))
        .round()
        .to_i64()
        .unwrap_or(0)
        .max(50)
}

pub fn verify_stripe_signature(payload: &str, sig_header: &str, secret: &str) -> Result<(), &'static str> {
    if secret.is_empty() {
        return Err("Stripe webhook secret is empty");
    }

    let mut timestamp: Option<i64> = None;
    let mut signatures: Vec<&str> = Vec::new();

    for item in sig_header.split(',') {
        let mut parts = item.splitn(2, '=');
        let key = parts.next().map(str::trim);
        let val = parts.next().map(str::trim);
        match (key, val) {
            (Some("t"), Some(t_str)) => {
                if let Ok(t_val) = t_str.parse::<i64>() {
                    timestamp = Some(t_val);
                }
            }
            (Some("v1"), Some(sig)) => {
                signatures.push(sig);
            }
            _ => {}
        }
    }

    let timestamp = timestamp.ok_or("Missing timestamp in Stripe-Signature header")?;
    if signatures.is_empty() {
        return Err("Missing v1 signature in Stripe-Signature header");
    }

    let now = Utc::now().timestamp();
    if now.abs_diff(timestamp) > SIGNATURE_TOLERANCE_SECS as u64 {
        return Err("Stripe-Signature timestamp outside tolerance");
    }

    let signed_payload = format!("{}.{}", timestamp, payload);
    let mut mac = HmacSha256::new_from_slice(secret.as_bytes())
        .map_err(|_| "Failed to initialize HMAC with secret")?;
    mac.update(signed_payload.as_bytes());
    let expected_sig = hex::encode(mac.finalize().into_bytes());

    let is_valid = signatures.iter().any(|sig| {
        sig.as_bytes().ct_eq(expected_sig.as_bytes()).into()
    });

    if is_valid {
        Ok(())
    } else {
        Err("Stripe signature mismatch")
    }
}

fn extract_payment_intent_id<'a>(event_type: &str, data_object: Option<&'a Value>) -> Option<&'a str> {
    let obj = data_object?;
    if event_type.starts_with("payment_intent.") {
        obj.get("id")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
    } else {
        obj.get("payment_intent")
            .and_then(|pi| pi.as_str().or_else(|| pi.get("id").and_then(Value::as_str)))
            .map(str::trim)
            .filter(|s| !s.is_empty())
    }
}

fn db_err(context: &str, err: sqlx::Error) -> StatusCode {
    error!("{context}: {err}");
    StatusCode::INTERNAL_SERVER_ERROR
}

#[utoipa::path(
    post,
    path = "/stripe",
    tag = "Webhooks",
    params(
        ("Stripe-Signature" = String, Header, description = "Stripe webhook HMAC signature header")
    ),
    request_body(content = String, content_type = "application/json", description = "Raw Stripe webhook event JSON payload"),
    responses(
        (status = 200, description = "Webhook processed"),
        (status = 400, description = "Invalid signature or malformed JSON"),
        (status = 413, description = "Webhook payload exceeds size limit"),
        (status = 500, description = "Webhook secret not configured or database error")
    )
)]
pub async fn handle_stripe_webhook(
    State(state): State<AppState>,
    request: Request,
) -> Result<impl IntoResponse, StatusCode> {
    let signature = request
        .headers()
        .get("Stripe-Signature")
        .and_then(|val| val.to_str().ok())
        .ok_or(StatusCode::BAD_REQUEST)?
        .to_owned();

    if let Some(content_len) = request
        .headers()
        .get(header::CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.trim().parse::<usize>().ok())
    {
        if content_len > MAX_WEBHOOK_BODY_BYTES {
            return Err(StatusCode::PAYLOAD_TOO_LARGE);
        }
    }

    // Cap body read to 1 MiB BEFORE verifying signature or buffering unbounded payloads
    let bytes = axum::body::to_bytes(request.into_body(), MAX_WEBHOOK_BODY_BYTES)
        .await
        .map_err(|e| {
            if e.to_string().to_ascii_lowercase().contains("length limit") {
                StatusCode::PAYLOAD_TOO_LARGE
            } else {
                StatusCode::BAD_REQUEST
            }
        })?;

    let payload = std::str::from_utf8(&bytes).map_err(|_| StatusCode::BAD_REQUEST)?;

    let secret = state
        .config
        .stripe_webhook_secret
        .as_deref()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            error!("Stripe webhook secret not configured");
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    if let Err(err) = verify_stripe_signature(payload, &signature, secret) {
        error!("Stripe webhook signature validation failed: {}", err);
        return Err(StatusCode::BAD_REQUEST);
    }

    let event: Value = serde_json::from_str(payload).map_err(|e| {
        error!("Failed to parse Stripe webhook JSON: {}", e);
        StatusCode::BAD_REQUEST
    })?;

    let event_id = event
        .get("id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty() && s.len() <= 255)
        .ok_or(StatusCode::BAD_REQUEST)?;

    let event_type = event.get("type").and_then(Value::as_str).unwrap_or("");
    info!(event_id = %event_id, event_type = %event_type, "Received Stripe webhook event");

    let data_object = event.get("data").and_then(|d| d.get("object"));
    let event_pi_id = extract_payment_intent_id(event_type, data_object);

    let mut tx = state
        .db
        .begin()
        .await
        .map_err(|e| db_err("Failed to start Stripe webhook transaction", e))?;

    let inserted_event: Option<String> = sqlx::query_scalar(
        r#"INSERT INTO stripe_webhook_events (event_id, event_type, payment_intent_id, processing_status)
           VALUES ($1, $2, $3, 'received')
           ON CONFLICT (event_id) DO NOTHING
           RETURNING event_id"#,
    )
    .bind(event_id)
    .bind(event_type)
    .bind(event_pi_id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(|e| db_err("Failed to record Stripe webhook event idempotency row", e))?;

    if inserted_event.is_none() {
        info!(event_id = %event_id, event_type = %event_type, "Duplicate Stripe webhook event ignored");
        return Ok(StatusCode::OK);
    }

    let processing_status = match event_type {
        "payment_intent.succeeded" => {
            if let Some(obj) = data_object {
                if let Some(pi_id) = extract_payment_intent_id(event_type, Some(obj)) {
                    let currency_ok = obj
                        .get("currency")
                        .and_then(Value::as_str)
                        .is_some_and(|c| c.eq_ignore_ascii_case("eur"));
                    if !currency_ok {
                        error!(event_id = %event_id, pi_id = %pi_id, "Rejecting payment_intent.succeeded with missing or non-EUR currency");
                        "failed"
                    } else {
                        let amount_received = obj
                            .get("amount_received")
                            .and_then(Value::as_i64)
                            .unwrap_or(0);

                        let invoice_row: Option<(Uuid, Decimal, InvoiceStatus)> = sqlx::query_as(
                            "SELECT id, total, status FROM invoices WHERE stripe_payment_intent_id = $1 FOR UPDATE",
                        )
                        .bind(pi_id)
                        .fetch_optional(&mut *tx)
                        .await
                        .map_err(|e| db_err("Failed to query invoice for payment_intent.succeeded", e))?;

                        if let Some((inv_id, inv_total, inv_status)) = invoice_row {
                            let expected_cents = expected_invoice_cents(inv_total);
                            if amount_received < expected_cents {
                                error!(
                                    event_id = %event_id,
                                    pi_id = %pi_id,
                                    invoice_id = %inv_id,
                                    amount_received,
                                    expected_cents,
                                    "Insufficient amount_received on payment_intent.succeeded"
                                );
                                "failed"
                            } else if inv_status == InvoiceStatus::Pending
                                || inv_status == InvoiceStatus::Overdue
                            {
                                sqlx::query(
                                    "UPDATE invoices SET status = 'paid'::invoice_status, paid_at = now(), updated_at = now() WHERE id = $1 AND status IN ('pending'::invoice_status, 'overdue'::invoice_status)",
                                )
                                .bind(inv_id)
                                .execute(&mut *tx)
                                .await
                                .map_err(|e| db_err("Failed to mark invoice paid on payment_intent.succeeded", e))?;
                                info!(event_id = %event_id, pi_id = %pi_id, invoice_id = %inv_id, "Marked invoice paid via payment_intent.succeeded");
                                "processed"
                            } else {
                                info!(event_id = %event_id, pi_id = %pi_id, invoice_id = %inv_id, status = ?inv_status, "Skipping paid transition for finalized invoice");
                                "ignored"
                            }
                        } else {
                            info!(event_id = %event_id, pi_id = %pi_id, "No matching invoice for payment_intent.succeeded");
                            "ignored"
                        }
                    }
                } else {
                    "ignored"
                }
            } else {
                "ignored"
            }
        }
        "invoice.payment_succeeded" => {
            if let Some(obj) = data_object {
                let customer = obj.get("customer").and_then(Value::as_str);
                info!("Invoice payment succeeded for customer {:?}", customer);

                if let Some(pi_id) = extract_payment_intent_id(event_type, Some(obj)) {
                    let currency_ok = obj
                        .get("currency")
                        .and_then(Value::as_str)
                        .is_some_and(|c| c.eq_ignore_ascii_case("eur"));
                    if !currency_ok {
                        error!(event_id = %event_id, pi_id = %pi_id, "Rejecting invoice.payment_succeeded with non-EUR currency");
                        "failed"
                    } else {
                        let invoice_row: Option<(Uuid, Decimal, InvoiceStatus)> = sqlx::query_as(
                            "SELECT id, total, status FROM invoices WHERE stripe_payment_intent_id = $1 FOR UPDATE",
                        )
                        .bind(pi_id)
                        .fetch_optional(&mut *tx)
                        .await
                        .map_err(|e| db_err("Failed to query invoice for invoice.payment_succeeded", e))?;

                        if let Some((inv_id, inv_total, inv_status)) = invoice_row {
                            let expected_cents = expected_invoice_cents(inv_total);
                            let amount_ok = obj
                                .get("amount_paid")
                                .and_then(Value::as_i64)
                                .is_some_and(|amt| amt >= expected_cents);
                            if !amount_ok {
                                error!(event_id = %event_id, pi_id = %pi_id, invoice_id = %inv_id, "Insufficient amount_paid on invoice.payment_succeeded");
                                "failed"
                            } else if inv_status == InvoiceStatus::Pending
                                || inv_status == InvoiceStatus::Overdue
                            {
                                sqlx::query(
                                    "UPDATE invoices SET status = 'paid'::invoice_status, paid_at = now(), updated_at = now() WHERE id = $1 AND status IN ('pending'::invoice_status, 'overdue'::invoice_status)",
                                )
                                .bind(inv_id)
                                .execute(&mut *tx)
                                .await
                                .map_err(|e| db_err("Failed to mark invoice paid on invoice.payment_succeeded", e))?;
                                "processed"
                            } else {
                                "ignored"
                            }
                        } else {
                            "ignored"
                        }
                    }
                } else {
                    "ignored"
                }
            } else {
                "ignored"
            }
        }
        "invoice.payment_failed" => {
            if let Some(obj) = data_object {
                let customer = obj.get("customer").and_then(Value::as_str);
                warn!("Invoice payment failed for customer {:?}", customer);

                if let Some(pi_id) = extract_payment_intent_id(event_type, Some(obj)) {
                    sqlx::query(
                        r#"UPDATE invoices
                           SET status = 'overdue'::invoice_status, updated_at = now()
                           WHERE stripe_payment_intent_id = $1
                             AND status IN ('pending'::invoice_status, 'overdue'::invoice_status)"#,
                    )
                    .bind(pi_id)
                    .execute(&mut *tx)
                    .await
                    .map_err(|e| db_err("Failed to update invoice on invoice.payment_failed", e))?;
                    "processed"
                } else {
                    "ignored"
                }
            } else {
                "ignored"
            }
        }
        "checkout.session.completed" => {
            if let Some(obj) = data_object {
                let customer = obj.get("customer").and_then(Value::as_str);
                info!("Checkout session completed for customer {:?}", customer);
            }
            "processed"
        }
        _ => "ignored",
    };

    sqlx::query(
        "UPDATE stripe_webhook_events SET processing_status = $2, processed_at = now() WHERE event_id = $1",
    )
    .bind(event_id)
    .bind(processing_status)
    .execute(&mut *tx)
    .await
    .map_err(|e| db_err("Failed to finalize Stripe webhook event row", e))?;

    tx.commit()
        .await
        .map_err(|e| db_err("Failed to commit Stripe webhook transaction", e))?;

    Ok(StatusCode::OK)
}
