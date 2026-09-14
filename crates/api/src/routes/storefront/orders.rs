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

    Ok(Json(json!({ "client_secret": client_secret })))
}

#[derive(Deserialize)]
struct PlaceOrder {
    product_id: Uuid,
    hostname: Option<String>,
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

    let connector_id: Option<Uuid> =
        sqlx::query_scalar("SELECT id FROM connectors WHERE provider = $1 LIMIT 1")
            .bind(&product.provider)
            .fetch_optional(&state.db)
            .await?;

    let connector_id = connector_id.ok_or(ApiError::Internal(
        "no connector configured for this provider".into(),
    ))?;

    let service_id = Uuid::now_v7();
    let hostname = payload
        .hostname
        .unwrap_or_else(|| format!("srv-{}", &service_id.to_string()[..8]));

    sqlx::query(
        r#"INSERT INTO services (id, customer_id, product_id, connector_id, name, hostname, price, status)
        VALUES ($1, $2, $3, $4, $5, $6, $7, 'pending')"#,
    )
    .bind(service_id)
    .bind(auth.0.sub)
    .bind(product.id)
    .bind(connector_id)
    .bind(&product.name)
    .bind(&hostname)
    .bind(product.price)
    .execute(&state.db)
    .await?;

    let job_id = Uuid::now_v7();
    let provision_payload = serde_json::json!({
        "server_type": product.specs.get("server_type").and_then(|v| v.as_str()).unwrap_or("cx22"),
        "location": product.specs.get("location").and_then(|v| v.as_str()).unwrap_or("fsn1"),
        "image": product.specs.get("image").and_then(|v| v.as_str()).unwrap_or("ubuntu-22.04"),
        "ssh_keys": [],
        "name": hostname,
        "extra": {}
    });

    sqlx::query(
        r#"INSERT INTO jobs (id, task, target_api, connector_id, customer_id, service_id, request_payload)
        VALUES ($1, 'provision', $2, $3, $4, $5, $6)"#
    )
    .bind(job_id)
    .bind(&product.provider)
    .bind(connector_id)
    .bind(auth.0.sub)
    .bind(service_id)
    .bind(&provision_payload)
    .execute(&state.db)
    .await?;

    Ok(Json(
        json!({ "service_id": service_id, "job_id": job_id, "status": "pending" }),
    ))
}
