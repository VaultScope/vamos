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
    Router::new().route("/", post(place_order))
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
        "SELECT * FROM products WHERE id = $1 AND hidden = false"
    )
    .bind(payload.product_id)
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound("product not found".into()))?;

    let connector_id: Option<Uuid> = sqlx::query_scalar(
        "SELECT id FROM connectors WHERE provider = $1 LIMIT 1"
    )
    .bind(&product.provider)
    .fetch_optional(&state.db)
    .await?;

    let connector_id = connector_id.ok_or(ApiError::Internal("no connector configured for this provider".into()))?;

    let service_id = Uuid::now_v7();
    let hostname = payload.hostname.unwrap_or_else(|| format!("srv-{}", &service_id.to_string()[..8]));

    sqlx::query(
        r#"INSERT INTO services (id, customer_id, product_id, name, hostname, price, status)
        VALUES ($1, $2, $3, $4, $5, $6, 'pending')"#
    )
    .bind(service_id)
    .bind(auth.0.sub)
    .bind(product.id)
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

    Ok(Json(json!({ "service_id": service_id, "job_id": job_id, "status": "pending" })))
}
