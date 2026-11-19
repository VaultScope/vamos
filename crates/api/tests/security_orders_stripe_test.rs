mod common;

use std::str::FromStr;
use std::sync::Arc;

use axum::body::{to_bytes, Body};
use axum::http::{header, Request, StatusCode};
use axum::routing::{get, post};
use axum::{Json, Router};
use chrono::Utc;
use hmac::{Hmac, Mac};
use rust_decimal::Decimal;
use serde_json::{json, Value};
use sha2::Sha256;
use sqlx::postgres::PgPoolOptions;
use tokio::net::TcpListener;
use tower::ServiceExt;
use uuid::Uuid;

use vaultscope_api::auth::{jwt, Audience, Claims};
use vaultscope_api::error::ApiError;
use vaultscope_api::routes::storefront::invoices::verify_invoice_stripe_pi_response;
use vaultscope_api::routes::storefront::orders::{
    compute_order_totals, sanitize_reliablesite_config, validate_stripe_payment_intent_id,
    verify_order_stripe_pi_response,
};
use vaultscope_api::routes::webhooks::stripe::{
    expected_invoice_cents, verify_stripe_signature, MAX_WEBHOOK_BODY_BYTES,
};
use vaultscope_api::state::AppState;

type HmacSha256 = Hmac<Sha256>;

fn make_token(sub: Uuid, email: &str, role: &str, audience: Audience, secret: &str) -> String {
    let claims = Claims {
        sub,
        email: email.to_string(),
        role: role.to_string(),
        audience,
        iat: Utc::now().timestamp(),
        exp: jwt::new_expiry(),
        auth_version: Some(1),
        jti: None,
    };
    jwt::sign(&claims, secret).unwrap()
}

fn sign_stripe_webhook(payload: &str, secret: &str, timestamp: i64) -> String {
    let signed_payload = format!("{timestamp}.{payload}");
    let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).unwrap();
    mac.update(signed_payload.as_bytes());
    let sig = hex::encode(mac.finalize().into_bytes());
    format!("t={timestamp},v1={sig}")
}

fn lazy_app_state(stripe_webhook_secret: Option<&str>) -> AppState {
    let pool = PgPoolOptions::new()
        .max_connections(1)
        .connect_lazy("postgres://postgres:postgres@127.0.0.1:5432/vaultscope_unused")
        .expect("lazy pool creation should not connect");
    let mut cfg = common::test_config();
    cfg.stripe_webhook_secret = stripe_webhook_secret.map(str::to_string);
    cfg.stripe_secret_key = Some("sk_test_dummy".to_string());
    let (ticket_bus, _) = tokio::sync::broadcast::channel(16);
    AppState {
        db: pool,
        config: Arc::new(cfg),
        csrf_config: axum_csrf::CsrfConfig::default(),
        redis: None,
        ticket_bus,
        sse_tickets: vaultscope_api::state::new_sse_tickets(),
    }
}

async fn get_csrf_cookie_and_token(app: &Router) -> (String, String) {
    let resp = app
        .clone()
        .oneshot(Request::builder().uri("/api/csrf").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let set_cookie = resp
        .headers()
        .get_all(header::SET_COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .map(|s| s.split(';').next().unwrap_or("").trim().to_string())
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("; ");

    let bytes = to_bytes(resp.into_body(), 1024 * 1024).await.unwrap();
    let json_body: Value = serde_json::from_slice(&bytes).unwrap();
    let token = json_body["token"].as_str().unwrap().to_string();
    (set_cookie, token)
}

#[test]
fn unit_pi_id_format_validation_rejects_traversal_and_invalid_inputs() {
    assert_eq!(
        validate_stripe_payment_intent_id("pi_3QxYz2abcDEF123").unwrap(),
        "pi_3QxYz2abcDEF123"
    );
    assert_eq!(
        validate_stripe_payment_intent_id("pi_1").unwrap(),
        "pi_1"
    );

    let long_pi = format!("pi_{}", "a".repeat(256));
    let invalid_inputs = [
        "",
        "   ",
        "pi_",
        " pi_abc123",
        "pi_abc123 ",
        "pi_abc 123",
        "pi_abc\n123",
        "pi_abc/../../customers",
        "pi_abc?expand=latest_charge",
        "pi_abc#fragment",
        "pi_abc_123",
        "cs_test_123",
        "../pi_abc123",
        long_pi.as_str(),
    ];

    for bad in invalid_inputs {
        let err = validate_stripe_payment_intent_id(bad).unwrap_err();
        assert!(
            matches!(err, ApiError::BadRequest(_)),
            "Expected BadRequest for invalid pi_id '{bad}'"
        );
    }
}

#[test]
fn unit_order_vat_decimal_rounding_consistency() {
    // 8.50 EUR * 19% = 1.615 -> round_dp(2) = 1.62 -> total = 10.12 -> 1012 cents
    // (Integer truncation would previously produce 1011 cents vs invoice 10.12 EUR.)
    let (sub, tax, total, cents) = compute_order_totals(Decimal::from_str("8.50").unwrap());
    assert_eq!(sub, Decimal::from_str("8.50").unwrap());
    assert_eq!(tax, Decimal::from_str("1.62").unwrap());
    assert_eq!(total, Decimal::from_str("10.12").unwrap());
    assert_eq!(cents, 1012);

    // SFTP Storage Pro: 12.00 EUR * 19% = 2.28 -> total = 14.28 -> 1428 cents
    let (sub, tax, total, cents) = compute_order_totals(Decimal::from_str("12.00").unwrap());
    assert_eq!(sub, Decimal::from_str("12.00").unwrap());
    assert_eq!(tax, Decimal::from_str("2.28").unwrap());
    assert_eq!(total, Decimal::from_str("14.28").unwrap());
    assert_eq!(cents, 1428);

    // ReliableSite quote subtotal: 467.00 EUR -> 88.73 VAT -> 555.73 total -> 55573 cents
    let (sub, tax, total, cents) = compute_order_totals(Decimal::from_str("467.00").unwrap());
    assert_eq!(sub, Decimal::from_str("467.00").unwrap());
    assert_eq!(tax, Decimal::from_str("88.73").unwrap());
    assert_eq!(total, Decimal::from_str("555.73").unwrap());
    assert_eq!(cents, 55573);

    assert_eq!(expected_invoice_cents(Decimal::from_str("14.28").unwrap()), 1428);
    assert_eq!(expected_invoice_cents(Decimal::from_str("0.10").unwrap()), 50);
}

#[test]
fn unit_order_stripe_pi_verification_enforces_status_currency_metadata_and_amount() {
    let customer_id = Uuid::now_v7();
    let other_customer_id = Uuid::now_v7();
    let product_id = Uuid::now_v7();
    let other_product_id = Uuid::now_v7();

    let valid_pi = json!({
        "id": "pi_validOrder123",
        "status": "succeeded",
        "currency": "eur",
        "amount_received": 1428,
        "metadata": {
            "purpose": "order",
            "customer_id": customer_id.to_string(),
            "product_id": product_id.to_string()
        }
    });
    assert!(verify_order_stripe_pi_response(&valid_pi, customer_id, product_id, 1428).is_ok());

    let wrong_status = json!({
        "status": "requires_payment_method",
        "currency": "eur",
        "amount_received": 1428,
        "metadata": { "purpose": "order", "customer_id": customer_id.to_string() }
    });
    assert!(verify_order_stripe_pi_response(&wrong_status, customer_id, product_id, 1428).is_err());

    let wrong_currency = json!({
        "status": "succeeded",
        "currency": "usd",
        "amount_received": 1428,
        "metadata": { "purpose": "order", "customer_id": customer_id.to_string() }
    });
    assert!(verify_order_stripe_pi_response(&wrong_currency, customer_id, product_id, 1428).is_err());

    let missing_currency = json!({
        "status": "succeeded",
        "amount_received": 1428,
        "metadata": { "purpose": "order", "customer_id": customer_id.to_string() }
    });
    assert!(verify_order_stripe_pi_response(&missing_currency, customer_id, product_id, 1428).is_err());

    let invoice_purpose = json!({
        "status": "succeeded",
        "currency": "eur",
        "amount_received": 1428,
        "metadata": { "purpose": "invoice", "customer_id": customer_id.to_string() }
    });
    assert!(verify_order_stripe_pi_response(&invoice_purpose, customer_id, product_id, 1428).is_err());

    let missing_purpose = json!({
        "status": "succeeded",
        "currency": "eur",
        "amount_received": 1428,
        "metadata": { "customer_id": customer_id.to_string() }
    });
    assert!(verify_order_stripe_pi_response(&missing_purpose, customer_id, product_id, 1428).is_err());

    let wrong_customer = json!({
        "status": "succeeded",
        "currency": "eur",
        "amount_received": 1428,
        "metadata": { "purpose": "order", "customer_id": other_customer_id.to_string() }
    });
    assert!(verify_order_stripe_pi_response(&wrong_customer, customer_id, product_id, 1428).is_err());

    let wrong_product = json!({
        "status": "succeeded",
        "currency": "eur",
        "amount_received": 1428,
        "metadata": {
            "purpose": "order",
            "customer_id": customer_id.to_string(),
            "product_id": other_product_id.to_string()
        }
    });
    assert!(verify_order_stripe_pi_response(&wrong_product, customer_id, product_id, 1428).is_err());

    let underpaid = json!({
        "status": "succeeded",
        "currency": "eur",
        "amount_received": 1427,
        "metadata": { "purpose": "order", "customer_id": customer_id.to_string() }
    });
    assert!(verify_order_stripe_pi_response(&underpaid, customer_id, product_id, 1428).is_err());
}

#[test]
fn unit_invoice_stripe_pi_verification_enforces_currency_metadata_and_amount() {
    let invoice_id = Uuid::now_v7();
    let other_invoice_id = Uuid::now_v7();
    let customer_id = Uuid::now_v7();
    let other_customer_id = Uuid::now_v7();

    let valid_pi = json!({
        "id": "pi_validInv123",
        "status": "succeeded",
        "currency": "eur",
        "amount_received": 2500,
        "metadata": {
            "purpose": "invoice",
            "invoice_id": invoice_id.to_string(),
            "customer_id": customer_id.to_string()
        }
    });
    assert!(verify_invoice_stripe_pi_response(&valid_pi, invoice_id, customer_id, 2500).is_ok());

    // Missing metadata.invoice_id MUST fail closed (regression for old `if let Some` bypass)
    let missing_invoice_meta = json!({
        "status": "succeeded",
        "currency": "eur",
        "amount_received": 2500,
        "metadata": { "purpose": "invoice", "customer_id": customer_id.to_string() }
    });
    assert!(verify_invoice_stripe_pi_response(&missing_invoice_meta, invoice_id, customer_id, 2500).is_err());

    // Missing metadata.customer_id MUST fail closed
    let missing_customer_meta = json!({
        "status": "succeeded",
        "currency": "eur",
        "amount_received": 2500,
        "metadata": { "purpose": "invoice", "invoice_id": invoice_id.to_string() }
    });
    assert!(verify_invoice_stripe_pi_response(&missing_customer_meta, invoice_id, customer_id, 2500).is_err());

    // Mismatched invoice_id or customer_id
    let wrong_invoice_meta = json!({
        "status": "succeeded",
        "currency": "eur",
        "amount_received": 2500,
        "metadata": {
            "purpose": "invoice",
            "invoice_id": other_invoice_id.to_string(),
            "customer_id": customer_id.to_string()
        }
    });
    assert!(verify_invoice_stripe_pi_response(&wrong_invoice_meta, invoice_id, customer_id, 2500).is_err());

    let wrong_customer_meta = json!({
        "status": "succeeded",
        "currency": "eur",
        "amount_received": 2500,
        "metadata": {
            "purpose": "invoice",
            "invoice_id": invoice_id.to_string(),
            "customer_id": other_customer_id.to_string()
        }
    });
    assert!(verify_invoice_stripe_pi_response(&wrong_customer_meta, invoice_id, customer_id, 2500).is_err());

    // Non-EUR currency or wrong purpose or underpaid
    let usd_pi = json!({
        "status": "succeeded",
        "currency": "usd",
        "amount_received": 2500,
        "metadata": {
            "purpose": "invoice",
            "invoice_id": invoice_id.to_string(),
            "customer_id": customer_id.to_string()
        }
    });
    assert!(verify_invoice_stripe_pi_response(&usd_pi, invoice_id, customer_id, 2500).is_err());

    let order_purpose = json!({
        "status": "succeeded",
        "currency": "eur",
        "amount_received": 2500,
        "metadata": {
            "purpose": "order",
            "invoice_id": invoice_id.to_string(),
            "customer_id": customer_id.to_string()
        }
    });
    assert!(verify_invoice_stripe_pi_response(&order_purpose, invoice_id, customer_id, 2500).is_err());

    let underpaid = json!({
        "status": "succeeded",
        "currency": "eur",
        "amount_received": 2499,
        "metadata": {
            "purpose": "invoice",
            "invoice_id": invoice_id.to_string(),
            "customer_id": customer_id.to_string()
        }
    });
    assert!(verify_invoice_stripe_pi_response(&underpaid, invoice_id, customer_id, 2500).is_err());
}

#[test]
fn unit_reliablesite_config_sanitization_strips_customer_promo_keys() {
    let raw = json!({
        "billing_cycle": "monthly",
        "promo_code": "ATTACKER100",
        "promoCode": "FREE",
        "os_image": "ubuntu-24.04"
    });
    let sanitized = sanitize_reliablesite_config(Some(&raw)).unwrap();
    assert!(sanitized.get("promo_code").is_none());
    assert!(sanitized.get("promoCode").is_none());
    assert_eq!(sanitized["billing_cycle"], "monthly");
    assert_eq!(sanitized["os_image"], "ubuntu-24.04");
}

#[test]
fn unit_webhook_signature_helper_validates_hmac_and_tolerance() {
    let secret = "whsec_unit_test_secret";
    let payload = r#"{"id":"evt_1","type":"payment_intent.succeeded"}"#;
    let now = Utc::now().timestamp();
    let valid_sig = sign_stripe_webhook(payload, secret, now);
    assert!(verify_stripe_signature(payload, &valid_sig, secret).is_ok());

    let stale_sig = sign_stripe_webhook(payload, secret, now - 600);
    assert!(verify_stripe_signature(payload, &stale_sig, secret).is_err());

    assert!(verify_stripe_signature(payload, "t=123,v1=deadbeef", secret).is_err());
    assert!(verify_stripe_signature(payload, &valid_sig, "").is_err());
}

#[tokio::test]
async fn unit_webhook_body_size_cap_and_signature_guards_without_db() {
    let secret = "whsec_test_cap";
    let state = lazy_app_state(Some(secret));
    let app = vaultscope_api::routes::build_router(state);

    // 1. Oversized body (> 1 MiB) MUST be rejected BEFORE signature verification or DB access
    let oversized = vec![b'a'; MAX_WEBHOOK_BODY_BYTES + 1];
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/webhooks/stripe")
                .header("Stripe-Signature", "t=1,v1=invalid")
                .body(Body::from(oversized))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::PAYLOAD_TOO_LARGE);

    // 2. Bad HMAC signature rejected with 400
    let body = json!({ "id": "evt_bad_sig", "type": "payment_intent.succeeded" }).to_string();
    let resp_bad_sig = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/webhooks/stripe")
                .header("Stripe-Signature", format!("t={},v1=0000", Utc::now().timestamp()))
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp_bad_sig.status(), StatusCode::BAD_REQUEST);

    // 3. Valid signature but missing event id rejected with 400 before DB
    let missing_id = json!({ "type": "payment_intent.succeeded" }).to_string();
    let sig = sign_stripe_webhook(&missing_id, secret, Utc::now().timestamp());
    let resp_missing_id = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/webhooks/stripe")
                .header("Stripe-Signature", sig)
                .body(Body::from(missing_id))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp_missing_id.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn unit_pay_invoice_rejects_path_injection_pi_id_before_db() {
    let mut state = common::test_app_state().await;
    let mut cfg = (*state.config).clone();
    cfg.stripe_secret_key = Some("sk_test_dummy".to_string());
    state.config = Arc::new(cfg);

    let cust_id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO customers (id, external_id, username, name, email, status, email_verified) VALUES ($1, $2, $3, 'PI Path Injection User', $4, 'active', true)",
    )
    .bind(cust_id)
    .bind(format!("cust_pi_{}", cust_id.simple()))
    .bind(format!("pi_user_{}", &cust_id.simple().to_string()[20..]))
    .bind(format!("pi_user_{}@test.local", &cust_id.simple().to_string()[20..]))
    .execute(&state.db)
    .await
    .unwrap();

    let jwt_token = make_token(
        cust_id,
        "unit@test.local",
        "customer",
        Audience::Storefront,
        &state.config.jwt_secret,
    );
    let app = vaultscope_api::routes::build_router(state.clone());
    let (cookie, csrf) = get_csrf_cookie_and_token(&app).await;
    let fake_inv_id = Uuid::now_v7();

    for bad_pi in [
        "pi_abc/../../customers",
        "pi_abc?expand=charges",
        "pi_abc#frag",
        "pi_abc def",
        "   ",
        "not_a_pi",
    ] {
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/api/storefront/invoices/{fake_inv_id}/pay"))
                    .header(header::AUTHORIZATION, format!("Bearer {jwt_token}"))
                    .header(header::COOKIE, &cookie)
                    .header("x-csrf-token", &csrf)
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(json!({ "payment_intent_id": bad_pi }).to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::BAD_REQUEST,
            "Expected 400 BadRequest for pi_id '{bad_pi}'"
        );
    }

    let _ = sqlx::query("DELETE FROM customers WHERE id = $1")
        .bind(cust_id)
        .execute(&state.db)
        .await;
}

#[tokio::test]
async fn db_sftp_paid_tier_requires_payment_and_rejects_traversal_pi_id() {
    let mut state = common::test_app_state().await;
    let mut cfg = (*state.config).clone();
    cfg.stripe_secret_key = Some("sk_test_dummy".to_string());
    state.config = Arc::new(cfg);

    let cust_id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO customers (id, external_id, username, name, email, status, email_verified) VALUES ($1, $2, $3, 'SFTP Buyer', $4, 'active', true)",
    )
    .bind(cust_id)
    .bind(format!("cust_sftp_{}", cust_id.simple()))
    .bind(format!("sftpbuyer_{}", &cust_id.simple().to_string()[20..]))
    .bind(format!("sftpbuyer_{}@test.local", &cust_id.simple().to_string()[20..]))
    .execute(&state.db)
    .await
    .unwrap();

    let jwt_token = make_token(
        cust_id,
        "sftpbuyer@test.local",
        "customer",
        Audience::Storefront,
        &state.config.jwt_secret,
    );
    let app = vaultscope_api::routes::build_router(state.clone());
    let (cookie, csrf) = get_csrf_cookie_and_token(&app).await;

    let sftp_pro_id = "e9e9e9e9-e9e9-e9e9-e9e9-e9e9e9e9e9e9";

    // 1. Paid SFTP Pro (12.00 EUR) without payment_intent_id MUST fail with 400 (no longer free!)
    let unpaid_resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/storefront/orders")
                .header(header::AUTHORIZATION, format!("Bearer {jwt_token}"))
                .header(header::COOKIE, &cookie)
                .header("x-csrf-token", &csrf)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(json!({ "product_id": sftp_pro_id }).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(unpaid_resp.status(), StatusCode::BAD_REQUEST);

    // 2. Paid SFTP Pro with path-traversal payment_intent_id MUST fail with 400 before Stripe call
    let traversal_resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/storefront/orders")
                .header(header::AUTHORIZATION, format!("Bearer {jwt_token}"))
                .header(header::COOKIE, &cookie)
                .header("x-csrf-token", &csrf)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    json!({
                        "product_id": sftp_pro_id,
                        "payment_intent_id": "pi_test/../../customers"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(traversal_resp.status(), StatusCode::BAD_REQUEST);

    let service_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM services WHERE customer_id = $1")
        .bind(cust_id)
        .fetch_one(&state.db)
        .await
        .unwrap();
    assert_eq!(service_count, 0);

    let _ = sqlx::query("DELETE FROM customers WHERE id = $1")
        .bind(cust_id)
        .execute(&state.db)
        .await;
}

#[tokio::test]
async fn db_mismatched_and_inactive_partner_slug_ignored_and_conversions_atomic() {
    let state = common::test_app_state().await;

    let cust_id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO customers (id, external_id, username, name, email, status, email_verified) VALUES ($1, $2, $3, 'Partner Buyer', $4, 'active', true)",
    )
    .bind(cust_id)
    .bind(format!("cust_part_{}", cust_id.simple()))
    .bind(format!("partbuyer_{}", &cust_id.simple().to_string()[20..]))
    .bind(format!("partbuyer_{}@test.local", &cust_id.simple().to_string()[20..]))
    .execute(&state.db)
    .await
    .unwrap();

    let sftp_trial_id = Uuid::from_str("d9d9d9d9-d9d9-d9d9-d9d9-d9d9d9d9d9d9").unwrap();
    let sftp_pro_id = Uuid::from_str("e9e9e9e9-e9e9-e9e9-e9e9-e9e9e9e9e9e9").unwrap();

    let partner_trial_only_id = Uuid::now_v7();
    let partner_trial_slug = format!("trial-only-{}", &partner_trial_only_id.simple().to_string()[20..]);
    sqlx::query(
        r#"INSERT INTO partners (id, slug, name, product_id, trial_days_override, trial_quota_gb_override, quota_gb_override, limit_one_per_user, active)
           VALUES ($1, $2, 'Trial Partner', $3, 60, 500, 1000, true, true)"#,
    )
    .bind(partner_trial_only_id)
    .bind(&partner_trial_slug)
    .bind(sftp_trial_id)
    .execute(&state.db)
    .await
    .unwrap();

    let partner_inactive_id = Uuid::now_v7();
    let partner_inactive_slug = format!("inactive-{}", &partner_inactive_id.simple().to_string()[20..]);
    sqlx::query(
        r#"INSERT INTO partners (id, slug, name, product_id, trial_days_override, limit_one_per_user, active)
           VALUES ($1, $2, 'Inactive Partner', $3, 30, true, false)"#,
    )
    .bind(partner_inactive_id)
    .bind(&partner_inactive_slug)
    .bind(sftp_pro_id)
    .execute(&state.db)
    .await
    .unwrap();

    let jwt_token = make_token(
        cust_id,
        "partbuyer@test.local",
        "customer",
        Audience::Storefront,
        &state.config.jwt_secret,
    );
    let app = vaultscope_api::routes::build_router(state.clone());
    let (cookie, csrf) = get_csrf_cookie_and_token(&app).await;

    // 1. Supplying trial-only partner_slug on SFTP Pro (mismatched product_id) MUST NOT grant trial or increment conversions
    let mismatch_resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/storefront/orders")
                .header(header::AUTHORIZATION, format!("Bearer {jwt_token}"))
                .header(header::COOKIE, &cookie)
                .header("x-csrf-token", &csrf)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    json!({
                        "product_id": sftp_pro_id,
                        "partner_slug": partner_trial_slug
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(mismatch_resp.status(), StatusCode::BAD_REQUEST);

    let conv_after_mismatch: i32 =
        sqlx::query_scalar("SELECT conversions FROM partners WHERE id = $1")
            .bind(partner_trial_only_id)
            .fetch_one(&state.db)
            .await
            .unwrap();
    assert_eq!(conv_after_mismatch, 0);

    // 2. Supplying inactive partner_slug on SFTP Pro MUST NOT grant trial or increment conversions
    let inactive_resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/storefront/orders")
                .header(header::AUTHORIZATION, format!("Bearer {jwt_token}"))
                .header(header::COOKIE, &cookie)
                .header("x-csrf-token", &csrf)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    json!({
                        "product_id": sftp_pro_id,
                        "partner_slug": partner_inactive_slug
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(inactive_resp.status(), StatusCode::BAD_REQUEST);

    let conv_after_inactive: i32 =
        sqlx::query_scalar("SELECT conversions FROM partners WHERE id = $1")
            .bind(partner_inactive_id)
            .fetch_one(&state.db)
            .await
            .unwrap();
    assert_eq!(conv_after_inactive, 0);

    // 3. Matching active partner on its own product succeeds and increments conversions once
    let ok_resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/storefront/orders")
                .header(header::AUTHORIZATION, format!("Bearer {jwt_token}"))
                .header(header::COOKIE, &cookie)
                .header("x-csrf-token", &csrf)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    json!({
                        "product_id": sftp_trial_id,
                        "partner_slug": partner_trial_slug
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(ok_resp.status(), StatusCode::OK);

    let conv_after_ok: i32 =
        sqlx::query_scalar("SELECT conversions FROM partners WHERE id = $1")
            .bind(partner_trial_only_id)
            .fetch_one(&state.db)
            .await
            .unwrap();
    assert_eq!(conv_after_ok, 1);

    // 4. Second trial claim by same customer is blocked and does NOT increment conversions
    let second_claim = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/storefront/orders")
                .header(header::AUTHORIZATION, format!("Bearer {jwt_token}"))
                .header(header::COOKIE, &cookie)
                .header("x-csrf-token", &csrf)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    json!({
                        "product_id": sftp_trial_id,
                        "partner_slug": partner_trial_slug
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(second_claim.status(), StatusCode::BAD_REQUEST);

    let conv_after_second: i32 =
        sqlx::query_scalar("SELECT conversions FROM partners WHERE id = $1")
            .bind(partner_trial_only_id)
            .fetch_one(&state.db)
            .await
            .unwrap();
    assert_eq!(conv_after_second, 1);

    let _ = sqlx::query("DELETE FROM jobs WHERE customer_id = $1")
        .bind(cust_id)
        .execute(&state.db)
        .await;
    let _ = sqlx::query("DELETE FROM services WHERE customer_id = $1")
        .bind(cust_id)
        .execute(&state.db)
        .await;
    let _ = sqlx::query("DELETE FROM partners WHERE id IN ($1, $2)")
        .bind(partner_trial_only_id)
        .bind(partner_inactive_id)
        .execute(&state.db)
        .await;
    let _ = sqlx::query("DELETE FROM customers WHERE id = $1")
        .bind(cust_id)
        .execute(&state.db)
        .await;
}

#[tokio::test]
async fn db_limited_stock_atomic_decrement_prevents_oversell() {
    let state = common::test_app_state().await;

    let cust_id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO customers (id, external_id, username, name, email, status, email_verified) VALUES ($1, $2, $3, 'Stock Buyer', $4, 'active', true)",
    )
    .bind(cust_id)
    .bind(format!("cust_stk_{}", cust_id.simple()))
    .bind(format!("stkbuyer_{}", &cust_id.simple().to_string()[20..]))
    .bind(format!("stkbuyer_{}@test.local", &cust_id.simple().to_string()[20..]))
    .execute(&state.db)
    .await
    .unwrap();

    let prod_id = Uuid::now_v7();
    sqlx::query(
        r#"INSERT INTO products (id, name, category, provider, target, specs, cost, price, setup_fee, stock, user_limit, billing_cycle, hidden)
           VALUES ($1, 'Limited Stock VPS', 'vps', 'hetzner_cloud', 'vps', '{"server_type":"cx22","location":"fsn1"}'::jsonb, 0.00, 0.00, 0.00, 1, 0, 'monthly', false)"#,
    )
    .bind(prod_id)
    .execute(&state.db)
    .await
    .unwrap();

    let jwt_token = make_token(
        cust_id,
        "stkbuyer@test.local",
        "customer",
        Audience::Storefront,
        &state.config.jwt_secret,
    );
    let app = vaultscope_api::routes::build_router(state.clone());
    let (cookie, csrf) = get_csrf_cookie_and_token(&app).await;

    let first = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/storefront/orders")
                .header(header::AUTHORIZATION, format!("Bearer {jwt_token}"))
                .header(header::COOKIE, &cookie)
                .header("x-csrf-token", &csrf)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(json!({ "product_id": prod_id }).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(first.status(), StatusCode::OK);

    let remaining: i32 = sqlx::query_scalar("SELECT stock FROM products WHERE id = $1")
        .bind(prod_id)
        .fetch_one(&state.db)
        .await
        .unwrap();
    assert_eq!(remaining, 0);

    let second = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/storefront/orders")
                .header(header::AUTHORIZATION, format!("Bearer {jwt_token}"))
                .header(header::COOKIE, &cookie)
                .header("x-csrf-token", &csrf)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(json!({ "product_id": prod_id }).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(second.status(), StatusCode::BAD_REQUEST);

    let _ = sqlx::query("DELETE FROM jobs WHERE customer_id = $1")
        .bind(cust_id)
        .execute(&state.db)
        .await;
    let _ = sqlx::query("DELETE FROM services WHERE customer_id = $1")
        .bind(cust_id)
        .execute(&state.db)
        .await;
    let _ = sqlx::query("DELETE FROM products WHERE id = $1")
        .bind(prod_id)
        .execute(&state.db)
        .await;
    let _ = sqlx::query("DELETE FROM customers WHERE id = $1")
        .bind(cust_id)
        .execute(&state.db)
        .await;
}

#[tokio::test]
async fn db_reliablesite_live_stock_check_fails_closed_on_upstream_error() {
    let mut state = common::test_app_state().await;

    let failing_rs_api = Router::new()
        .route(
            "/v1/Login/Token",
            post(|| async { Json(json!({ "status": true, "message": "tok_fail" })) }),
        )
        .route(
            "/v1/Products/{product_id}/Stock",
            get(|| async { (StatusCode::INTERNAL_SERVER_ERROR, "upstream error") }),
        );
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, failing_rs_api).await.unwrap();
    });

    let mut cfg = (*state.config).clone();
    cfg.reliablesite_api_key = Some("rs_live_key".to_string());
    cfg.reliablesite_order_api_url = Some(format!("http://{addr}"));
    cfg.stripe_secret_key = Some("sk_test_dummy".to_string());
    state.config = Arc::new(cfg);

    let cust_id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO customers (id, external_id, username, name, email, status, email_verified) VALUES ($1, $2, $3, 'RS FailClosed', $4, 'active', true)",
    )
    .bind(cust_id)
    .bind(format!("cust_rsfc_{}", cust_id.simple()))
    .bind(format!("rsfc_{}", &cust_id.simple().to_string()[20..]))
    .bind(format!("rsfc_{}@test.local", &cust_id.simple().to_string()[20..]))
    .execute(&state.db)
    .await
    .unwrap();

    let jwt_token = make_token(
        cust_id,
        "rsfc@test.local",
        "customer",
        Audience::Storefront,
        &state.config.jwt_secret,
    );
    let app = vaultscope_api::routes::build_router(state.clone());
    let (cookie, csrf) = get_csrf_cookie_and_token(&app).await;

    // Product 428 has DB stock = 7, but live Order API fails with 500 -> MUST fail closed (500), never fall back to DB stock
    let prod_428_id = "10000000-0000-4000-8000-000000000428";
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/storefront/orders/create-payment-intent")
                .header(header::AUTHORIZATION, format!("Bearer {jwt_token}"))
                .header(header::COOKIE, &cookie)
                .header("x-csrf-token", &csrf)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(json!({ "product_id": prod_428_id }).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);

    let _ = sqlx::query("DELETE FROM customers WHERE id = $1")
        .bind(cust_id)
        .execute(&state.db)
        .await;
}

#[tokio::test]
async fn db_reliablesite_non_dev_mock_mode_fails_closed() {
    let mut state = common::test_app_state().await;
    let mut cfg = (*state.config).clone();
    cfg.reliablesite_api_key = None;
    cfg.dev_mode = false;
    cfg.stripe_secret_key = Some("sk_test_dummy".to_string());
    state.config = Arc::new(cfg);

    let cust_id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO customers (id, external_id, username, name, email, status, email_verified) VALUES ($1, $2, $3, 'RS NonDev', $4, 'active', true)",
    )
    .bind(cust_id)
    .bind(format!("cust_rsnd_{}", cust_id.simple()))
    .bind(format!("rsnd_{}", &cust_id.simple().to_string()[20..]))
    .bind(format!("rsnd_{}@test.local", &cust_id.simple().to_string()[20..]))
    .execute(&state.db)
    .await
    .unwrap();

    let jwt_token = make_token(
        cust_id,
        "rsnd@test.local",
        "customer",
        Audience::Storefront,
        &state.config.jwt_secret,
    );
    let app = vaultscope_api::routes::build_router(state.clone());
    let (cookie, csrf) = get_csrf_cookie_and_token(&app).await;

    let prod_428_id = "10000000-0000-4000-8000-000000000428";
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/storefront/orders/create-payment-intent")
                .header(header::AUTHORIZATION, format!("Bearer {jwt_token}"))
                .header(header::COOKIE, &cookie)
                .header("x-csrf-token", &csrf)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(json!({ "product_id": prod_428_id }).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);

    let _ = sqlx::query("DELETE FROM customers WHERE id = $1")
        .bind(cust_id)
        .execute(&state.db)
        .await;
}

#[tokio::test]
async fn db_webhook_dedup_amount_currency_and_state_transition_guards() {
    let mut state = common::test_app_state().await;
    let secret = "whsec_integration_guard";
    let mut cfg = (*state.config).clone();
    cfg.stripe_webhook_secret = Some(secret.to_string());
    state.config = Arc::new(cfg);

    let cust_id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO customers (id, external_id, username, name, email, status, email_verified) VALUES ($1, $2, $3, 'Webhook User', $4, 'active', true)",
    )
    .bind(cust_id)
    .bind(format!("cust_wh_{}", cust_id.simple()))
    .bind(format!("whuser_{}", &cust_id.simple().to_string()[20..]))
    .bind(format!("whuser_{}@test.local", &cust_id.simple().to_string()[20..]))
    .execute(&state.db)
    .await
    .unwrap();

    let inv1_id = Uuid::now_v7();
    let pi1 = format!("pi_wh1_{}", inv1_id.simple());
    let inv1_num = format!("INV-TEST-{}", &inv1_id.simple().to_string()[20..]);
    sqlx::query(
        r#"INSERT INTO invoices (id, invoice_number, customer_id, status, subtotal, tax_rate, tax_amount, total, issue_date, due_date, stripe_payment_intent_id)
           VALUES ($1, $2, $3, 'pending', 12.00, 19.00, 2.28, 14.28, CURRENT_DATE, CURRENT_DATE + 14, $4)"#,
    )
    .bind(inv1_id)
    .bind(&inv1_num)
    .bind(cust_id)
    .bind(&pi1)
    .execute(&state.db)
    .await
    .unwrap();

    // Verify 0038 unique partial index blocks duplicate non-null stripe_payment_intent_id on invoices
    let dup_inv_id = Uuid::now_v7();
    let dup_res = sqlx::query(
        r#"INSERT INTO invoices (id, invoice_number, customer_id, status, subtotal, tax_rate, tax_amount, total, issue_date, due_date, stripe_payment_intent_id)
           VALUES ($1, $2, $3, 'pending', 12.00, 19.00, 2.28, 14.28, CURRENT_DATE, CURRENT_DATE + 14, $4)"#,
    )
    .bind(dup_inv_id)
    .bind(format!("INV-DUP-{}", &dup_inv_id.simple().to_string()[20..]))
    .bind(cust_id)
    .bind(&pi1)
    .execute(&state.db)
    .await;
    assert!(dup_inv_id != inv1_id && dup_res.is_err());

    // Verify 0038 invoice_number_seq works
    let seq_val: i64 = sqlx::query_scalar("SELECT nextval('invoice_number_seq')")
        .fetch_one(&state.db)
        .await
        .unwrap();
    assert!(seq_val >= 1000);

    let app = vaultscope_api::routes::build_router(state.clone());
    let now = Utc::now().timestamp();

    // 1. Non-EUR payment_intent.succeeded MUST NOT mark invoice paid
    let evt_usd = format!("evt_usd_{}", Uuid::now_v7().simple());
    let body_usd = json!({
        "id": evt_usd,
        "type": "payment_intent.succeeded",
        "data": { "object": { "id": pi1, "currency": "usd", "amount_received": 1428 } }
    })
    .to_string();
    let resp_usd = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/webhooks/stripe")
                .header("Stripe-Signature", sign_stripe_webhook(&body_usd, secret, now))
                .body(Body::from(body_usd))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp_usd.status(), StatusCode::OK);

    let st_after_usd: String =
        sqlx::query_scalar("SELECT status::text FROM invoices WHERE id = $1")
            .bind(inv1_id)
            .fetch_one(&state.db)
            .await
            .unwrap();
    assert_eq!(st_after_usd, "pending");

    // 2. Underpaid payment_intent.succeeded MUST NOT mark invoice paid
    let evt_underpaid = format!("evt_underpaid_{}", Uuid::now_v7().simple());
    let body_underpaid = json!({
        "id": evt_underpaid,
        "type": "payment_intent.succeeded",
        "data": { "object": { "id": pi1, "currency": "eur", "amount_received": 500 } }
    })
    .to_string();
    let resp_underpaid = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/webhooks/stripe")
                .header("Stripe-Signature", sign_stripe_webhook(&body_underpaid, secret, now))
                .body(Body::from(body_underpaid))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp_underpaid.status(), StatusCode::OK);

    let st_after_underpaid: String =
        sqlx::query_scalar("SELECT status::text FROM invoices WHERE id = $1")
            .bind(inv1_id)
            .fetch_one(&state.db)
            .await
            .unwrap();
    assert_eq!(st_after_underpaid, "pending");

    // 3. Valid EUR full-amount payment_intent.succeeded marks invoice paid & records dedup row
    let evt_paid = format!("evt_paid_{}", Uuid::now_v7().simple());
    let body_paid = json!({
        "id": evt_paid,
        "type": "payment_intent.succeeded",
        "data": { "object": { "id": pi1, "currency": "eur", "amount_received": 1428 } }
    })
    .to_string();
    let resp_paid = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/webhooks/stripe")
                .header("Stripe-Signature", sign_stripe_webhook(&body_paid, secret, now))
                .body(Body::from(body_paid.clone()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp_paid.status(), StatusCode::OK);

    let st_after_paid: String =
        sqlx::query_scalar("SELECT status::text FROM invoices WHERE id = $1")
            .bind(inv1_id)
            .fetch_one(&state.db)
            .await
            .unwrap();
    assert_eq!(st_after_paid, "paid");

    // 4. Duplicate event_id is deduplicated cleanly
    let resp_dup = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/webhooks/stripe")
                .header("Stripe-Signature", sign_stripe_webhook(&body_paid, secret, now))
                .body(Body::from(body_paid))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp_dup.status(), StatusCode::OK);

    // 5. Out-of-order invoice.payment_failed MUST NOT downgrade paid invoice to overdue
    let evt_fail_after_paid = format!("evt_fail_paid_{}", Uuid::now_v7().simple());
    let body_fail_after_paid = json!({
        "id": evt_fail_after_paid,
        "type": "invoice.payment_failed",
        "data": { "object": { "customer": "cus_123", "payment_intent": pi1 } }
    })
    .to_string();
    let resp_fail = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/webhooks/stripe")
                .header("Stripe-Signature", sign_stripe_webhook(&body_fail_after_paid, secret, now))
                .body(Body::from(body_fail_after_paid))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp_fail.status(), StatusCode::OK);

    let st_after_fail: String =
        sqlx::query_scalar("SELECT status::text FROM invoices WHERE id = $1")
            .bind(inv1_id)
            .fetch_one(&state.db)
            .await
            .unwrap();
    assert_eq!(st_after_fail, "paid");

    // 6. invoice.payment_failed DOES transition a pending invoice to overdue
    let inv2_id = Uuid::now_v7();
    let pi2 = format!("pi_wh2_{}", inv2_id.simple());
    let inv2_num = format!("INV-TEST-{}", &inv2_id.simple().to_string()[20..]);
    sqlx::query(
        r#"INSERT INTO invoices (id, invoice_number, customer_id, status, subtotal, tax_rate, tax_amount, total, issue_date, due_date, stripe_payment_intent_id)
           VALUES ($1, $2, $3, 'pending', 10.00, 19.00, 1.90, 11.90, CURRENT_DATE, CURRENT_DATE + 14, $4)"#,
    )
    .bind(inv2_id)
    .bind(&inv2_num)
    .bind(cust_id)
    .bind(&pi2)
    .execute(&state.db)
    .await
    .unwrap();

    let evt_fail_pending = format!("evt_fail_pend_{}", Uuid::now_v7().simple());
    let body_fail_pending = json!({
        "id": evt_fail_pending,
        "type": "invoice.payment_failed",
        "data": { "object": { "customer": "cus_123", "payment_intent": pi2 } }
    })
    .to_string();
    let resp_fail_pend = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/webhooks/stripe")
                .header("Stripe-Signature", sign_stripe_webhook(&body_fail_pending, secret, now))
                .body(Body::from(body_fail_pending))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp_fail_pend.status(), StatusCode::OK);

    let st_inv2: String =
        sqlx::query_scalar("SELECT status::text FROM invoices WHERE id = $1")
            .bind(inv2_id)
            .fetch_one(&state.db)
            .await
            .unwrap();
    assert_eq!(st_inv2, "overdue");

    // Cleanup synthetic webhook events
    let _ = sqlx::query("DELETE FROM stripe_webhook_events WHERE event_id IN ($1, $2, $3, $4, $5)")
        .bind(&evt_usd)
        .bind(&evt_underpaid)
        .bind(&evt_paid)
        .bind(&evt_fail_after_paid)
        .bind(&evt_fail_pending)
        .execute(&state.db)
        .await;
}
