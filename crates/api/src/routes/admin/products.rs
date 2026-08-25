use axum::extract::{Path, State};
use axum::routing::get;
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
}

async fn list(
    _auth: AuthAdmin,
    State(state): State<AppState>,
) -> Result<Json<Vec<Product>>, ApiError> {
    let products: Vec<Product> = sqlx::query_as("SELECT * FROM products ORDER BY name")
        .fetch_all(&state.db)
        .await?;
    Ok(Json(products))
}

async fn get_one(
    _auth: AuthAdmin,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<Product>, ApiError> {
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
    target: String,
    specs: Value,
    cost: f64,
    price: f64,
    setup_fee: Option<f64>,
    billing_cycle: String,
}

async fn create(
    _auth: AuthAdmin,
    State(state): State<AppState>,
    Json(payload): Json<CreateProduct>,
) -> Result<Json<Product>, ApiError> {
    let id = Uuid::now_v7();
    let product: Product = sqlx::query_as(
        r#"INSERT INTO products (id, name, category, provider, target, specs, cost, price, setup_fee, billing_cycle)
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10::billing_cycle)
        RETURNING *"#
    )
    .bind(id)
    .bind(&payload.name)
    .bind(&payload.category)
    .bind(&payload.provider)
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
    _auth: AuthAdmin,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(payload): Json<CreateProduct>,
) -> Result<Json<Product>, ApiError> {
    let product: Product = sqlx::query_as(
        r#"UPDATE products SET name=$1, category=$2, provider=$3, target=$4, specs=$5,
        cost=$6, price=$7, setup_fee=$8, billing_cycle=$9::billing_cycle, updated_at=now()
        WHERE id=$10 RETURNING *"#
    )
    .bind(&payload.name)
    .bind(&payload.category)
    .bind(&payload.provider)
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
