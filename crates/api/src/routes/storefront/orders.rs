use std::collections::BTreeMap;
use std::time::Duration;

use axum::extract::State;
use axum::Json;
use rust_decimal::prelude::ToPrimitive;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use utoipa::ToSchema;
use utoipa_axum::{router::OpenApiRouter, routes};
use uuid::Uuid;

use crate::auth::AuthCustomer;
use crate::error::ApiError;
use crate::openapi::ErrorResponse;
use crate::reliablesite_catalog;
use crate::state::AppState;

pub fn routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(place_order))
        .routes(routes!(create_payment_intent))
}

/// Validates a Stripe PaymentIntent ID strictly against `^pi_[A-Za-z0-9]+$`.
/// Rejects path traversal (`/`), query/fragment delimiters (`?`, `#`), whitespace,
/// underscores, and empty values before issuing any Stripe API request.
pub fn validate_stripe_payment_intent_id(raw: &str) -> Result<&str, ApiError> {
    let Some(suffix) = raw.strip_prefix("pi_") else {
        return Err(ApiError::BadRequest(
            "Invalid payment_intent_id format".into(),
        ));
    };
    if suffix.is_empty()
        || suffix.len() > 255
        || !suffix.bytes().all(|b| b.is_ascii_alphanumeric())
    {
        return Err(ApiError::BadRequest(
            "Invalid payment_intent_id format".into(),
        ));
    }
    Ok(raw)
}

/// Computes net subtotal, tax for an explicit percentage rate (rounded to 2 decimal places via `rust_decimal`),
/// gross total, and gross total in integer euro cents consistently across
/// Stripe PaymentIntent creation and order/invoice verification.
pub fn compute_order_totals_with_rate(
    subtotal_net: Decimal,
    tax_rate_percent: Decimal,
) -> (Decimal, Decimal, Decimal, i64) {
    let subtotal = subtotal_net.max(Decimal::ZERO).round_dp(2);
    let clamped_rate = tax_rate_percent.max(Decimal::ZERO).min(Decimal::from(100));
    let tax_amount = (subtotal * (clamped_rate / Decimal::from(100))).round_dp(2);
    let total = (subtotal + tax_amount).round_dp(2);
    let total_cents = (total * Decimal::from(100))
        .round()
        .to_i64()
        .unwrap_or(0);
    (subtotal, tax_amount, total, total_cents)
}

/// Computes net subtotal, 19% VAT (rounded to 2 decimal places via `rust_decimal`),
/// gross total, and gross total in integer euro cents consistently for both
/// Stripe PaymentIntent creation and order/invoice verification.
pub fn compute_order_totals(subtotal_net: Decimal) -> (Decimal, Decimal, Decimal, i64) {
    compute_order_totals_with_rate(subtotal_net, Decimal::new(19, 0))
}

/// Verifies a Stripe PaymentIntent JSON object for a storefront service order.
pub fn verify_order_stripe_pi_response(
    pi_data: &Value,
    expected_customer_id: Uuid,
    expected_product_id: Uuid,
    expected_min_cents: i64,
) -> Result<(), ApiError> {
    let status = pi_data["status"].as_str().unwrap_or("");
    if status != "succeeded" {
        return Err(ApiError::BadRequest(format!(
            "Payment has not succeeded yet (status: {status})"
        )));
    }

    let currency_ok = pi_data["currency"]
        .as_str()
        .is_some_and(|c| c.eq_ignore_ascii_case("eur"));
    if !currency_ok {
        return Err(ApiError::BadRequest(
            "Payment currency must be EUR".into(),
        ));
    }

    let purpose = pi_data["metadata"]["purpose"].as_str().unwrap_or("");
    if purpose != "order" {
        return Err(ApiError::BadRequest(
            "Payment intent is not valid for service orders".into(),
        ));
    }

    let expected_customer = expected_customer_id.to_string();
    if pi_data["metadata"]["customer_id"].as_str() != Some(expected_customer.as_str()) {
        return Err(ApiError::BadRequest(
            "Payment intent does not belong to this customer".into(),
        ));
    }

    if let Some(meta_product_id) = pi_data["metadata"]["product_id"].as_str() {
        if meta_product_id != expected_product_id.to_string() {
            return Err(ApiError::BadRequest(
                "Payment intent does not match selected product".into(),
            ));
        }
    }

    let amount_received = pi_data["amount_received"].as_i64().unwrap_or(0);
    if amount_received < expected_min_cents {
        return Err(ApiError::BadRequest(format!(
            "Payment amount insufficient (received {amount_received}, expected at least {expected_min_cents})"
        )));
    }

    Ok(())
}

pub(crate) fn stripe_http_client() -> Result<reqwest::Client, ApiError> {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .build()
        .map_err(|e| {
            tracing::error!("Failed to initialize Stripe HTTP client: {e}");
            ApiError::Internal("Payment provider unavailable".into())
        })
}

pub(crate) fn is_unique_violation(err: &sqlx::Error) -> bool {
    matches!(
        err,
        sqlx::Error::Database(db_err) if db_err.code().as_deref() == Some("23505")
    )
}

/// Removes any customer-supplied promo keys from a dedicated server config JSON object.
pub fn sanitize_reliablesite_config(config: Option<&Value>) -> Option<Value> {
    config.cloned().map(|mut v| {
        if let Some(obj) = v.as_object_mut() {
            obj.remove("promo_code");
            obj.remove("promoCode");
        }
        v
    })
}

async fn resolve_reliablesite_stock(
    state: &AppState,
    q: &reliablesite_catalog::ReliableSiteQuote,
) -> Result<i32, ApiError> {
    let rs_conn = state.config.reliablesite_connector();
    if !rs_conn.is_mock() {
        match rs_conn
            .order_api()
            .get_product_stock(q.reliablesite_product_id)
            .await
        {
            Ok(s) if s.stock_controlled => Ok(s.stock),
            Ok(_) => Ok(1),
            Err(e) => {
                tracing::error!(
                    reliablesite_product_id = q.reliablesite_product_id,
                    "Upstream dedicated stock check failed: {e}"
                );
                Err(ApiError::Internal(
                    "Unable to verify server availability. Please try again shortly.".into(),
                ))
            }
        }
    } else if state.config.dev_mode {
        Ok(q.stock)
    } else {
        tracing::error!(
            reliablesite_product_id = q.reliablesite_product_id,
            "Dedicated server connector is unconfigured in non-dev mode"
        );
        Err(ApiError::Internal(
            "Dedicated server provisioning is temporarily unavailable.".into(),
        ))
    }
}

#[derive(Deserialize, ToSchema)]
pub(crate) struct CreatePaymentIntentPayload {
    product_id: Uuid,
    #[serde(default)]
    billing_cycle: Option<String>,
    #[serde(default)]
    config_options: Option<BTreeMap<String, u32>>,
    #[serde(default)]
    config: Option<Value>,
}

fn extract_config_options(
    explicit: Option<&BTreeMap<String, u32>>,
    config_json: Option<&Value>,
) -> Option<BTreeMap<String, u32>> {
    if let Some(m) = explicit {
        if !m.is_empty() {
            return Some(m.clone());
        }
    }
    let obj = config_json
        .and_then(|c| c.get("config_options").or_else(|| c.get("configOptions")))
        .and_then(|v| v.as_object())?;

    let mut out = BTreeMap::new();
    for (k, v) in obj {
        if let Some(val) = v
            .as_u64()
            .and_then(|n| u32::try_from(n).ok())
            .or_else(|| v.as_str().and_then(|s| s.trim().parse::<u32>().ok()))
        {
            out.insert(k.trim().to_string(), val);
        }
    }
    if out.is_empty() {
        None
    } else {
        Some(out)
    }
}

fn extract_optional_string(
    explicit: Option<&str>,
    config_json: Option<&Value>,
    keys: &[&str],
) -> Option<String> {
    if let Some(s) = explicit.map(str::trim).filter(|s| !s.is_empty()) {
        return Some(s.to_string());
    }
    let cfg = config_json?;
    for key in keys {
        if let Some(s) = cfg.get(*key).and_then(|v| v.as_str()).map(str::trim).filter(|s| !s.is_empty()) {
            return Some(s.to_string());
        }
    }
    None
}

#[allow(dead_code)]
#[derive(Serialize, ToSchema)]
pub struct OrderPaymentIntentResponse {
    pub client_secret: String,
    pub payment_intent_id: String,
}

#[utoipa::path(
    post,
    path = "/create-payment-intent",
    tag = "Storefront Orders",
    security(("bearer_auth" = []), ("csrf_token" = [])),
    request_body = CreatePaymentIntentPayload,
    responses(
        (status = 200, description = "Stripe PaymentIntent client secret for checkout", body = OrderPaymentIntentResponse),
        (status = 400, description = "Out of stock or invalid configuration", body = ErrorResponse),
        (status = 404, description = "Product not found", body = ErrorResponse)
    )
)]
pub(crate) async fn create_payment_intent(
    auth: AuthCustomer,
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

    if product.stock == 0 {
        return Err(ApiError::BadRequest(
            "Selected product is currently out of stock".into(),
        ));
    }

    let pricing_policy = reliablesite_catalog::load_pricing_policy(&state.db, &state.config).await;
    let tax_policy = reliablesite_catalog::load_tax_policy(&state.db, &state.config).await;

    let subtotal_net = if product.provider == "reliablesite" {
        let cycle = extract_optional_string(
            payload.billing_cycle.as_deref(),
            payload.config.as_ref(),
            &["billing_cycle", "billingCycle"],
        );
        let opts = extract_config_options(payload.config_options.as_ref(), payload.config.as_ref());
        let quote = reliablesite_catalog::compute_quote_with_policy(
            &state.db,
            &product,
            cycle.as_deref(),
            opts.as_ref(),
            pricing_policy,
            &tax_policy,
        )
        .await?;

        let current_stock = resolve_reliablesite_stock(&state, &quote).await?;
        if current_stock < 1 {
            return Err(ApiError::BadRequest(
                "Selected dedicated server is currently out of stock".into(),
            ));
        }

        quote.subtotal_net
    } else {
        product.price + product.setup_fee
    };

    let stripe_secret = state
        .config
        .stripe_secret_key
        .as_deref()
        .ok_or(ApiError::Internal("Stripe not configured".into()))?;

    let (_, _, _, total_cents) =
        compute_order_totals_with_rate(subtotal_net, tax_policy.rate_percent);
    let amount = total_cents.max(50);

    let customer_id_str = auth.0.sub.to_string();
    let product_id_str = product.id.to_string();

    let client = stripe_http_client()?;
    let resp = client
        .post("https://api.stripe.com/v1/payment_intents")
        .basic_auth(stripe_secret, Option::<&str>::None)
        .form(&[
            ("amount", amount.to_string()),
            ("currency", "eur".to_string()),
            ("automatic_payment_methods[enabled]", "true".to_string()),
            ("metadata[purpose]", "order".to_string()),
            ("metadata[customer_id]", customer_id_str),
            ("metadata[product_id]", product_id_str),
        ])
        .send()
        .await
        .map_err(|e| {
            tracing::error!("Stripe create payment intent request failed: {e}");
            ApiError::Internal("Payment provider unavailable".into())
        })?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        tracing::error!(%status, stripe_body = %body, "Stripe create payment intent error");
        return Err(ApiError::Internal("Failed to initialize payment".into()));
    }

    let body: Value = resp.json().await.map_err(|e| {
        tracing::error!("Failed to parse Stripe create payment intent response: {e}");
        ApiError::Internal("Invalid response from payment provider".into())
    })?;

    let client_secret = body["client_secret"].as_str().ok_or_else(|| {
        tracing::error!("Missing client_secret in Stripe create payment intent response");
        ApiError::Internal("Invalid response from payment provider".into())
    })?;
    let payment_intent_id = body["id"].as_str().ok_or_else(|| {
        tracing::error!("Missing id in Stripe create payment intent response");
        ApiError::Internal("Invalid response from payment provider".into())
    })?;

    Ok(Json(json!({
        "client_secret": client_secret,
        "payment_intent_id": payment_intent_id
    })))
}

#[derive(Deserialize, ToSchema)]
#[schema(as = StorefrontPlaceOrder)]
pub(crate) struct PlaceOrder {
    product_id: Uuid,
    hostname: Option<String>,
    payment_intent_id: Option<String>,
    partner_slug: Option<String>,
    config: Option<Value>,
    #[serde(default)]
    billing_cycle: Option<String>,
    #[serde(default)]
    config_options: Option<BTreeMap<String, u32>>,
    #[serde(default)]
    promo_code: Option<String>,
    #[serde(default)]
    order_notes: Option<String>,
}

#[allow(dead_code)]
#[derive(Serialize, ToSchema)]
pub struct PlaceOrderResponse {
    pub service_id: Uuid,
    pub job_id: Uuid,
    pub status: String,
}

#[utoipa::path(
    post,
    path = "/",
    tag = "Storefront Orders",
    security(("bearer_auth" = []), ("csrf_token" = [])),
    request_body = PlaceOrder,
    responses(
        (status = 200, description = "Service order created and provisioning job queued", body = PlaceOrderResponse),
        (status = 400, description = "Invalid order, partner limit, or payment verification failed", body = ErrorResponse),
        (status = 404, description = "Product not found", body = ErrorResponse)
    )
)]
pub(crate) async fn place_order(
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

    if product.stock == 0 {
        return Err(ApiError::BadRequest(
            "Selected product is currently out of stock".into(),
        ));
    }

    // Bind partner_slug strictly to payload.product_id AND active = true;
    // never increment conversions before full validation & final transaction commit.
    let (
        partner_id,
        partner_trial_override,
        partner_trial_quota_override,
        partner_quota_override,
        partner_limit_one_per_user,
    ): (Option<Uuid>, Option<i32>, Option<i32>, Option<i32>, Option<bool>) =
        if let Some(slug) = payload
            .partner_slug
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            let row: Option<(Uuid, Option<i32>, Option<i32>, Option<i32>, bool)> = sqlx::query_as(
                "SELECT id, trial_days_override, trial_quota_gb_override, quota_gb_override, limit_one_per_user FROM partners WHERE LOWER(slug) = LOWER($1) AND product_id = $2 AND active = true",
            )
            .bind(slug)
            .bind(product.id)
            .fetch_optional(&state.db)
            .await?;

            match row {
                Some((pid, trial, t_quota, quota, limit)) => {
                    (Some(pid), trial, t_quota, quota, Some(limit))
                }
                None => (None, None, None, None, None),
            }
        } else {
            (None, None, None, None, None)
        };

    let effective_trial_days = partner_trial_override.or(product.trial_days);

    let promo_code = extract_optional_string(
        payload.promo_code.as_deref(),
        payload.config.as_ref(),
        &["promo_code", "promoCode"],
    );
    if let Some(ref pc) = promo_code {
        if pc.len() > 64 {
            return Err(ApiError::BadRequest(
                "promo_code must be at most 64 characters".into(),
            ));
        }
    }
    let order_notes = extract_optional_string(
        payload.order_notes.as_deref(),
        payload.config.as_ref(),
        &["order_notes", "orderNotes"],
    );
    if let Some(ref notes) = order_notes {
        if notes.len() > 2000 {
            return Err(ApiError::BadRequest(
                "order_notes must be at most 2,000 characters".into(),
            ));
        }
    }

    let pricing_policy = reliablesite_catalog::load_pricing_policy(&state.db, &state.config).await;
    let tax_policy = reliablesite_catalog::load_tax_policy(&state.db, &state.config).await;

    let is_reliablesite = product.provider == "reliablesite";
    let rs_quote = if is_reliablesite {
        let cycle = extract_optional_string(
            payload.billing_cycle.as_deref(),
            payload.config.as_ref(),
            &["billing_cycle", "billingCycle"],
        );
        let opts = extract_config_options(payload.config_options.as_ref(), payload.config.as_ref());
        let q = reliablesite_catalog::compute_quote_with_policy(
            &state.db,
            &product,
            cycle.as_deref(),
            opts.as_ref(),
            pricing_policy,
            &tax_policy,
        )
        .await?;

        let current_stock = resolve_reliablesite_stock(&state, &q).await?;
        if current_stock < 1 {
            return Err(ApiError::BadRequest(
                "Selected dedicated server is currently out of stock".into(),
            ));
        }
        Some(q)
    } else {
        None
    };

    let (recurring_net, setup_fee_net, subtotal_net, period_days) = if let Some(ref q) = rs_quote {
        (q.recurring_net, q.setup_fee_net, q.subtotal_net, q.period_days)
    } else {
        (
            product.price,
            product.setup_fee,
            product.price + product.setup_fee,
            30i64,
        )
    };

    let (invoice_subtotal, invoice_tax_amount, invoice_total, total_cents) =
        compute_order_totals_with_rate(subtotal_net, tax_policy.rate_percent);

    let has_trial = !is_reliablesite && effective_trial_days.unwrap_or(0) > 0;
    let is_storage = product.category.to_lowercase().contains("sftp")
        || product.category.to_lowercase().contains("storage")
        || product.name.to_lowercase().contains("sftp")
        || product.provider == "sftpgo";

    let is_free = has_trial || subtotal_net <= Decimal::ZERO;
    let is_trial = has_trial || (is_storage && subtotal_net <= Decimal::ZERO);

    // Fast-fail limit check before calling Stripe (re-verified atomically inside transaction below)
    let is_single_claim_limited = partner_limit_one_per_user.unwrap_or(true);
    if is_trial && is_single_claim_limited {
        let existing_trials: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM services WHERE customer_id = $1 AND is_trial = true",
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

    let verified_payment_intent_id: Option<String> = if is_free {
        None
    } else {
        let raw_pi = payload.payment_intent_id.as_deref().ok_or_else(|| {
            ApiError::BadRequest("payment_intent_id required for paid services".into())
        })?;
        let pi_id = validate_stripe_payment_intent_id(raw_pi)?.to_string();

        // 1. Fast-fail check that the payment intent has not been redeemed in services or invoices
        let existing_service: Option<(Uuid,)> =
            sqlx::query_as("SELECT id FROM services WHERE stripe_payment_intent_id = $1")
                .bind(&pi_id)
                .fetch_optional(&state.db)
                .await?;

        if existing_service.is_some() {
            return Err(ApiError::BadRequest(
                "This payment intent has already been used".into(),
            ));
        }

        let existing_invoice: Option<(Uuid,)> =
            sqlx::query_as("SELECT id FROM invoices WHERE stripe_payment_intent_id = $1")
                .bind(&pi_id)
                .fetch_optional(&state.db)
                .await?;

        if existing_invoice.is_some() {
            return Err(ApiError::BadRequest(
                "This payment intent has already been used".into(),
            ));
        }

        // 2. Verify with Stripe API (fail-closed for paid services)
        let stripe_secret = state.config.stripe_secret_key.as_deref().ok_or_else(|| {
            ApiError::Internal("Payment processing is not configured for paid services.".into())
        })?;

        let client = stripe_http_client()?;
        let resp = client
            .get(format!("https://api.stripe.com/v1/payment_intents/{}", pi_id))
            .basic_auth(stripe_secret, Option::<&str>::None)
            .send()
            .await
            .map_err(|e| {
                tracing::error!(pi_id = %pi_id, "Stripe verification request failed: {e}");
                ApiError::Internal("Payment verification failed".into())
            })?;

        if !resp.status().is_success() {
            let status = resp.status();
            let err_text = resp.text().await.unwrap_or_default();
            tracing::error!(pi_id = %pi_id, %status, stripe_body = %err_text, "Stripe verification error");
            return Err(ApiError::BadRequest(
                "Invalid or unverified payment intent".into(),
            ));
        }

        let pi_data: Value = resp.json().await.map_err(|e| {
            tracing::error!(pi_id = %pi_id, "Failed to parse Stripe verification response: {e}");
            ApiError::Internal("Invalid response from payment provider".into())
        })?;

        verify_order_stripe_pi_response(&pi_data, auth.0.sub, product.id, total_cents.max(50))?;

        Some(pi_id)
    };

    // Never accept customer-supplied promo_code for ReliableSite reseller orders;
    // always resolve reseller_promo_code server-side from reliablesite_servers.
    let reseller_promo: Option<String> = if let Some(ref q) = rs_quote {
        sqlx::query_scalar::<_, Option<String>>(
            "SELECT reseller_promo_code FROM reliablesite_servers WHERE product_id = $1",
        )
        .bind(q.reliablesite_product_id)
        .fetch_optional(&state.db)
        .await?
        .flatten()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
    } else {
        None
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

    let trial_days_count = if has_trial {
        effective_trial_days.unwrap() as i64
    } else {
        30
    };

    let (initial_price, next_due, trial_ends_at) = if is_trial {
        let expiry = chrono::Utc::now() + chrono::Duration::days(trial_days_count);
        (Decimal::ZERO, Some(expiry), Some(expiry))
    } else {
        (
            recurring_net,
            Some(chrono::Utc::now() + chrono::Duration::days(period_days)),
            None,
        )
    };

    let effective_trial_quota_gb = partner_trial_quota_override
        .or(product.trial_quota_gb)
        .unwrap_or(25);

    let effective_post_trial_quota_gb = partner_quota_override
        .or(product.quota_gb)
        .or_else(|| {
            product
                .specs
                .get("storage_gb")
                .and_then(|v| v.as_i64())
                .map(|v| v as i32)
        })
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

    let sanitized_rs_config = sanitize_reliablesite_config(payload.config.as_ref());

    let initial_service_config = if let Some(ref q) = rs_quote {
        json!({
            "billing_cycle": q.billing_cycle,
            "config_options": q.validated_config_options,
            "setup_fee": setup_fee_net,
            "quote_line_items": q.line_items,
            "promo_code": reseller_promo,
            "order_notes": order_notes,
        })
    } else {
        payload.config.clone().unwrap_or_else(|| json!({}))
    };

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
    } else if let Some(ref q) = rs_quote {
        serde_json::json!({
            "server_type": q.reliablesite_product_id.to_string(),
            "location": product.specs.get("location").and_then(|v| v.as_str()).unwrap_or("NL"),
            "image": custom_image,
            "ssh_keys": [],
            "name": hostname,
            "extra": {
                "customer_id": auth.0.sub.to_string(),
                "reliablesite_product_id": q.reliablesite_product_id,
                "billing_cycle": q.billing_cycle,
                "config_options": q.validated_config_options,
                "promo_code": reseller_promo,
                "order_notes": order_notes,
                "idempotency_key": service_id.to_string(),
                "cpu": product.specs.get("cpu"),
                "ram": product.specs.get("ram"),
                "disk": product.specs.get("disk"),
                "specs": product.specs,
                "config": sanitized_rs_config
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

    // Execute all order state mutations in one atomic SQL transaction
    let mut tx = state.db.begin().await?;

    // Lock customer row to serialize concurrent trial/order claims by this customer
    let customer_exists: Option<Uuid> =
        sqlx::query_scalar("SELECT id FROM customers WHERE id = $1 FOR UPDATE")
            .bind(auth.0.sub)
            .fetch_optional(&mut *tx)
            .await?;
    if customer_exists.is_none() {
        return Err(ApiError::Unauthorized);
    }

    // Lock product row and decrement stock atomically for limited-stock products
    let locked_stock: i32 = sqlx::query_scalar(
        "SELECT stock FROM products WHERE id = $1 AND hidden = false FOR UPDATE",
    )
    .bind(product.id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(ApiError::NotFound("product not found".into()))?;

    if locked_stock == 0 {
        return Err(ApiError::BadRequest(
            "Selected product is currently out of stock".into(),
        ));
    }
    if locked_stock > 0 {
        let updated_stock: Option<i32> = sqlx::query_scalar(
            "UPDATE products SET stock = stock - 1, updated_at = now() WHERE id = $1 AND stock >= 1 RETURNING stock",
        )
        .bind(product.id)
        .fetch_optional(&mut *tx)
        .await?;

        if updated_stock.is_none() {
            return Err(ApiError::BadRequest(
                "Selected product is currently out of stock".into(),
            ));
        }
    }

    // Re-check single-trial limit atomically inside transaction
    if is_trial && is_single_claim_limited {
        let existing_trials: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM services WHERE customer_id = $1 AND is_trial = true",
        )
        .bind(auth.0.sub)
        .fetch_one(&mut *tx)
        .await?;

        if existing_trials >= 1 {
            return Err(ApiError::BadRequest(
                "This partner campaign can only be claimed once per user. You have already claimed a promotional trial.".into(),
            ));
        }
    }

    if let Some(pid) = partner_id {
        if is_single_claim_limited {
            let partner_claims: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM services WHERE customer_id = $1 AND partner_id = $2",
            )
            .bind(auth.0.sub)
            .bind(pid)
            .fetch_one(&mut *tx)
            .await?;

            if partner_claims >= 1 {
                return Err(ApiError::BadRequest(
                    "This partner campaign can only be claimed once per user.".into(),
                ));
            }
        }
    }

    if product.user_limit > 0 {
        let active_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM services WHERE customer_id = $1 AND product_id = $2 AND status != 'terminated'",
        )
        .bind(auth.0.sub)
        .bind(product.id)
        .fetch_one(&mut *tx)
        .await?;

        if active_count >= product.user_limit as i64 {
            return Err(ApiError::BadRequest(
                "Product purchase limit reached for this account".into(),
            ));
        }
    }

    if let Some(ref pi_id) = verified_payment_intent_id {
        let used_in_services: Option<Uuid> =
            sqlx::query_scalar("SELECT id FROM services WHERE stripe_payment_intent_id = $1")
                .bind(pi_id)
                .fetch_optional(&mut *tx)
                .await?;
        if used_in_services.is_some() {
            return Err(ApiError::BadRequest(
                "This payment intent has already been used".into(),
            ));
        }

        let used_in_invoices: Option<Uuid> =
            sqlx::query_scalar("SELECT id FROM invoices WHERE stripe_payment_intent_id = $1")
                .bind(pi_id)
                .fetch_optional(&mut *tx)
                .await?;
        if used_in_invoices.is_some() {
            return Err(ApiError::BadRequest(
                "This payment intent has already been used".into(),
            ));
        }
    }

    let initial_order_state = if verified_payment_intent_id.is_some() {
        "payment_received"
    } else {
        "provisioning"
    };
    let initial_order_state_note: Option<&str> = if product.provider == "reliablesite" {
        Some("Queued for dedicated hardware provisioning (regularly up to 7 business days after payment).")
    } else {
        None
    };

    sqlx::query(
        r#"INSERT INTO services (id, customer_id, product_id, name, hostname, price, status, next_due, stripe_payment_intent_id, is_trial, trial_ends_at, partner_id, quota_gb, config, order_state, order_state_note, order_state_updated_at)
        VALUES ($1, $2, $3, $4, $5, $6, 'pending', $7, $8, $9, $10, $11, $12, $13, $14, $15, now())"#,
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
    .bind(&initial_service_config)
    .bind(initial_order_state)
    .bind(initial_order_state_note)
    .execute(&mut *tx)
    .await
    .map_err(|e| {
        if is_unique_violation(&e) {
            ApiError::BadRequest("This payment intent has already been used".into())
        } else {
            ApiError::from(e)
        }
    })?;

    // Create paid invoice and line items atomically if payment occurred
    if let Some(ref pi_id) = verified_payment_intent_id {
        let invoice_id = Uuid::now_v7();
        let seq_num: i64 = sqlx::query_scalar("SELECT nextval('invoice_number_seq')")
            .fetch_one(&mut *tx)
            .await?;
        let inv_num = format!(
            "INV-{}-{:06}",
            chrono::Utc::now().format("%Y%m"),
            seq_num
        );
        let today = chrono::Utc::now().date_naive();

        sqlx::query(
            r#"INSERT INTO invoices (id, invoice_number, customer_id, status, subtotal, tax_rate, tax_amount, total, issue_date, due_date, paid_at, stripe_payment_intent_id, notes)
            VALUES ($1, $2, $3, 'paid', $4, $5, $6, $7, $8, $8, now(), $9, $10)"#,
        )
        .bind(invoice_id)
        .bind(&inv_num)
        .bind(auth.0.sub)
        .bind(invoice_subtotal)
        .bind(tax_policy.rate_percent)
        .bind(invoice_tax_amount)
        .bind(invoice_total)
        .bind(today)
        .bind(pi_id)
        .bind(format!("Order for {}", product.name))
        .execute(&mut *tx)
        .await
        .map_err(|e| {
            if is_unique_violation(&e) {
                ApiError::BadRequest("This payment intent has already been used".into())
            } else {
                ApiError::from(e)
            }
        })?;

        if let Some(ref q) = rs_quote {
            let mut sort_idx = 0i32;
            for (idx, item) in q.line_items.iter().enumerate() {
                let item_total = (item.recurring + item.setup_fee).round_dp(2);
                if idx > 0 && item_total <= Decimal::ZERO {
                    continue;
                }
                let desc = if item.group_name == "Base Server" {
                    item.label.clone()
                } else {
                    format!("{}: {}", item.group_name, item.label)
                };
                sqlx::query(
                    r#"INSERT INTO invoice_line_items (id, invoice_id, description, quantity, unit_price, total, service_id, sort_order)
                    VALUES ($1, $2, $3, 1, $4, $5, $6, $7)"#,
                )
                .bind(Uuid::now_v7())
                .bind(invoice_id)
                .bind(desc)
                .bind(item_total)
                .bind(item_total)
                .bind(service_id)
                .bind(sort_idx)
                .execute(&mut *tx)
                .await?;
                sort_idx += 1;
            }
        } else if setup_fee_net > Decimal::ZERO {
            let rec = recurring_net.max(Decimal::ZERO).round_dp(2);
            let setup = setup_fee_net.max(Decimal::ZERO).round_dp(2);
            sqlx::query(
                r#"INSERT INTO invoice_line_items (id, invoice_id, description, quantity, unit_price, total, service_id, sort_order)
                VALUES ($1, $2, $3, 1, $4, $5, $6, 0)"#,
            )
            .bind(Uuid::now_v7())
            .bind(invoice_id)
            .bind(&product.name)
            .bind(rec)
            .bind(rec)
            .bind(service_id)
            .execute(&mut *tx)
            .await?;

            sqlx::query(
                r#"INSERT INTO invoice_line_items (id, invoice_id, description, quantity, unit_price, total, service_id, sort_order)
                VALUES ($1, $2, $3, 1, $4, $5, $6, 1)"#,
            )
            .bind(Uuid::now_v7())
            .bind(invoice_id)
            .bind(format!("{} (Setup Fee)", product.name))
            .bind(setup)
            .bind(setup)
            .bind(service_id)
            .execute(&mut *tx)
            .await?;
        } else {
            sqlx::query(
                r#"INSERT INTO invoice_line_items (id, invoice_id, description, quantity, unit_price, total, service_id, sort_order)
                VALUES ($1, $2, $3, 1, $4, $5, $6, 0)"#,
            )
            .bind(Uuid::now_v7())
            .bind(invoice_id)
            .bind(&product.name)
            .bind(invoice_subtotal)
            .bind(invoice_subtotal)
            .bind(service_id)
            .execute(&mut *tx)
            .await?;
        }
    }

    sqlx::query(
        r#"INSERT INTO jobs (id, task, target_api, customer_id, service_id, request_payload)
        VALUES ($1, 'provision', $2, $3, $4, $5)"#,
    )
    .bind(job_id)
    .bind(&product.provider)
    .bind(auth.0.sub)
    .bind(service_id)
    .bind(&provision_payload)
    .execute(&mut *tx)
    .await?;

    if let Some(pid) = partner_id {
        sqlx::query(
            "UPDATE partners SET conversions = conversions + 1, updated_at = now() WHERE id = $1 AND product_id = $2 AND active = true",
        )
        .bind(pid)
        .bind(product.id)
        .execute(&mut *tx)
        .await?;
    }

    tx.commit().await?;

    crate::mailer::spawn_order_confirmation_notification(
        state.clone(),
        auth.0.sub,
        product.name.clone(),
        product.provider.clone(),
        product.category.clone(),
        service_id,
        verified_payment_intent_id
            .is_some()
            .then_some(invoice_total),
    );

    Ok(Json(
        json!({ "service_id": service_id, "job_id": job_id, "status": "pending" }),
    ))
}
