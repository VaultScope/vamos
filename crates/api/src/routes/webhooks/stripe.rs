use axum::{
    extract::{State, Request},
    http::StatusCode,
    response::IntoResponse,
};
use crate::state::AppState;
use tracing::{info, error, warn};
use hmac::{Hmac, Mac};
use sha2::Sha256;
use subtle::ConstantTimeEq;
use chrono::Utc;
use serde_json::Value;

type HmacSha256 = Hmac<Sha256>;

const SIGNATURE_TOLERANCE_SECS: i64 = 300;

fn verify_stripe_signature(payload: &str, sig_header: &str, secret: &str) -> Result<(), &'static str> {
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
    if (now - timestamp).abs() > SIGNATURE_TOLERANCE_SECS {
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

    let bytes = axum::body::to_bytes(request.into_body(), usize::MAX)
        .await
        .map_err(|_| StatusCode::BAD_REQUEST)?;

    let payload = std::str::from_utf8(&bytes).map_err(|_| StatusCode::BAD_REQUEST)?;

    let secret = state
        .config
        .stripe_webhook_secret
        .as_ref()
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

    let event_type = event.get("type").and_then(Value::as_str).unwrap_or("");
    info!("Received Stripe webhook event: {}", event_type);

    let data_object = event.get("data").and_then(|d| d.get("object"));

    match event_type {
        "payment_intent.succeeded" => {
            if let Some(obj) = data_object {
                if let Some(pi_id) = obj.get("id").and_then(Value::as_str) {
                    info!("PaymentIntent succeeded: {}", pi_id);
                    let _ = sqlx::query(
                        "UPDATE invoices SET status = 'paid', paid_at = now() WHERE stripe_payment_intent_id = $1 AND status != 'paid'"
                    )
                    .bind(pi_id)
                    .execute(&state.db)
                    .await;
                }
            }
        }
        "invoice.payment_succeeded" => {
            if let Some(obj) = data_object {
                let customer = obj.get("customer").and_then(Value::as_str);
                info!("Invoice payment succeeded for customer {:?}", customer);

                let pi_id = obj.get("payment_intent").and_then(|pi| {
                    pi.as_str().or_else(|| pi.get("id").and_then(Value::as_str))
                });

                if let Some(pi_id) = pi_id {
                    let _ = sqlx::query(
                        "UPDATE invoices SET status = 'paid', paid_at = now() WHERE stripe_payment_intent_id = $1 AND status != 'paid'"
                    )
                    .bind(pi_id)
                    .execute(&state.db)
                    .await;
                }
            }
        }
        "invoice.payment_failed" => {
            if let Some(obj) = data_object {
                let customer = obj.get("customer").and_then(Value::as_str);
                warn!("Invoice payment failed for customer {:?}", customer);

                let pi_id = obj.get("payment_intent").and_then(|pi| {
                    pi.as_str().or_else(|| pi.get("id").and_then(Value::as_str))
                });

                if let Some(pi_id) = pi_id {
                    let _ = sqlx::query(
                        "UPDATE invoices SET status = 'overdue' WHERE stripe_payment_intent_id = $1"
                    )
                    .bind(pi_id)
                    .execute(&state.db)
                    .await;
                }
            }
        }
        "checkout.session.completed" => {
            if let Some(obj) = data_object {
                let customer = obj.get("customer").and_then(Value::as_str);
                info!("Checkout session completed for customer {:?}", customer);
            }
        }
        _ => {}
    }

    Ok(StatusCode::OK)
}
