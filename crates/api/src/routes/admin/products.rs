use axum::extract::{Path, State};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::Value;
use uuid::Uuid;

use crate::auth::AuthAdmin;
use crate::error::ApiError;
use crate::state::AppState;
use vaultscope_db::models::Product;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/", get(list).post(create))
        .route("/{id}", get(get_one).put(update))
        .route("/{id}/sync", post(sync_stock))
}

async fn list(
    auth: AuthAdmin,
    State(state): State<AppState>,
) -> Result<Json<Vec<Product>>, ApiError> {
    crate::require_permission!(auth, state, "products.view");
    let products: Vec<Product> = sqlx::query_as("SELECT * FROM products ORDER BY name")
        .fetch_all(&state.db)
        .await?;
    Ok(Json(products))
}

async fn get_one(
    auth: AuthAdmin,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<Product>, ApiError> {
    crate::require_permission!(auth, state, "products.view");
    let product: Product = sqlx::query_as("SELECT * FROM products WHERE id = $1")
        .bind(id)
        .fetch_optional(&state.db)
        .await?
        .ok_or(ApiError::NotFound("product not found".into()))?;
    Ok(Json(product))
}

#[derive(Deserialize)]
struct CreateProduct {
    name: String,
    category: String,
    provider: String,
    connector_id: Option<Uuid>,
    target: String,
    specs: Value,
    cost: f64,
    price: f64,
    setup_fee: Option<f64>,
    billing_cycle: String,
}

async fn create(
    auth: AuthAdmin,
    State(state): State<AppState>,
    Json(payload): Json<CreateProduct>,
) -> Result<Json<Product>, ApiError> {
    crate::require_permission!(auth, state, "products.create");
    let id = Uuid::now_v7();
    let product: Product = sqlx::query_as(
        r#"INSERT INTO products (id, name, category, provider, connector_id, target, specs, cost, price, setup_fee, billing_cycle)
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11::billing_cycle)
        RETURNING *"#
    )
    .bind(id)
    .bind(&payload.name)
    .bind(&payload.category)
    .bind(&payload.provider)
    .bind(payload.connector_id)
    .bind(&payload.target)
    .bind(&payload.specs)
    .bind(rust_decimal::Decimal::from_f64_retain(payload.cost).unwrap_or_default())
    .bind(rust_decimal::Decimal::from_f64_retain(payload.price).unwrap_or_default())
    .bind(rust_decimal::Decimal::from_f64_retain(payload.setup_fee.unwrap_or(0.0)).unwrap_or_default())
    .bind(&payload.billing_cycle)
    .fetch_one(&state.db)
    .await?;

    Ok(Json(product))
}

async fn update(
    auth: AuthAdmin,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(payload): Json<CreateProduct>,
) -> Result<Json<Product>, ApiError> {
    crate::require_permission!(auth, state, "products.edit");
    let product: Product = sqlx::query_as(
        r#"UPDATE products SET name=$1, category=$2, provider=$3, connector_id=$4, target=$5, specs=$6,
        cost=$7, price=$8, setup_fee=$9, billing_cycle=$10::billing_cycle, updated_at=now()
        WHERE id=$11 RETURNING *"#
    )
    .bind(&payload.name)
    .bind(&payload.category)
    .bind(&payload.provider)
    .bind(payload.connector_id)
    .bind(&payload.target)
    .bind(&payload.specs)
    .bind(rust_decimal::Decimal::from_f64_retain(payload.cost).unwrap_or_default())
    .bind(rust_decimal::Decimal::from_f64_retain(payload.price).unwrap_or_default())
    .bind(rust_decimal::Decimal::from_f64_retain(payload.setup_fee.unwrap_or(0.0)).unwrap_or_default())
    .bind(&payload.billing_cycle)
    .bind(id)
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound("product not found".into()))?;

    Ok(Json(product))
}

async fn sync_stock(
    auth: AuthAdmin,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<Product>, ApiError> {
    crate::require_permission!(auth, state, "products.manage");

    let product: Product = sqlx::query_as("SELECT * FROM products WHERE id = $1")
        .bind(id)
        .fetch_optional(&state.db)
        .await?
        .ok_or(ApiError::NotFound("product not found".into()))?;

    // Cloud-provisioned products have unlimited stock (on-demand);
    // products without a connector keep their manually-set stock.
    let new_stock = if product.connector_id.is_some() { -1 } else { product.stock };

    let updated_product: Product = sqlx::query_as(
        "UPDATE products SET stock=$1, updated_at=now() WHERE id=$2 RETURNING *",
    )
    .bind(new_stock)
    .bind(id)
    .fetch_one(&state.db)
    .await?;

    Ok(Json(updated_product))
}
