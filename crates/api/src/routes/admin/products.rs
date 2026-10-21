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
        .route("/{id}", get(get_one).put(update).delete(delete_product))
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
    target: String,
    specs: Value,
    cost: f64,
    price: f64,
    setup_fee: Option<f64>,
    billing_cycle: String,
    trial_days: Option<i32>,
    trial_quota_gb: Option<i32>,
    quota_gb: Option<i32>,
    service_form_id: Option<Uuid>,
    hidden: Option<bool>,
}

async fn create(
    auth: AuthAdmin,
    State(state): State<AppState>,
    Json(payload): Json<CreateProduct>,
) -> Result<Json<Product>, ApiError> {
    crate::require_permission!(auth, state, "products.create");
    let id = Uuid::now_v7();
    let product: Product = sqlx::query_as(
        r#"INSERT INTO products (id, name, category, provider, target, specs, cost, price, setup_fee, billing_cycle, trial_days, trial_quota_gb, quota_gb, service_form_id, hidden)
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10::billing_cycle, $11, $12, $13, $14, $15)
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
    .bind(payload.trial_days)
    .bind(payload.trial_quota_gb)
    .bind(payload.quota_gb)
    .bind(payload.service_form_id)
    .bind(payload.hidden.unwrap_or(false))
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
        r#"UPDATE products SET name=$1, category=$2, provider=$3, target=$4, specs=$5,
        cost=$6, price=$7, setup_fee=$8, billing_cycle=$9::billing_cycle, trial_days=$10,
        trial_quota_gb=$11, quota_gb=$12, service_form_id=$13, hidden=COALESCE($14, hidden), updated_at=now()
        WHERE id=$15 RETURNING *"#
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
    .bind(payload.trial_days)
    .bind(payload.trial_quota_gb)
    .bind(payload.quota_gb)
    .bind(payload.service_form_id)
    .bind(payload.hidden)
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
    // products without a provider keep their manually-set stock.
    let new_stock = if !product.provider.is_empty() && product.provider != "manual" { -1 } else { product.stock };

    let updated_product: Product = sqlx::query_as(
        "UPDATE products SET stock=$1, updated_at=now() WHERE id=$2 RETURNING *",
    )
    .bind(new_stock)
    .bind(id)
    .fetch_one(&state.db)
    .await?;

    Ok(Json(updated_product))
}

async fn delete_product(
    auth: AuthAdmin,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, ApiError> {
    crate::require_permission!(auth, state, "products.edit");

    // Check if any services (active, suspended or pending) reference this product
    let service_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM services WHERE product_id = $1"
    )
    .bind(id)
    .fetch_one(&state.db)
    .await?;

    if service_count > 0 {
        return Err(ApiError::BadRequest(format!(
            "Cannot delete product: {} active/existing service(s) are linked to this product. Deactivate/hide it from the storefront instead to prevent breaking customer services.",
            service_count
        )));
    }

    let rows = sqlx::query("DELETE FROM products WHERE id = $1")
        .bind(id)
        .execute(&state.db)
        .await?
        .rows_affected();

    if rows == 0 {
        return Err(ApiError::NotFound("product not found".into()));
    }

    Ok(Json(serde_json::json!({ "deleted": true })))
}
