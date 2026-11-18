use axum::extract::{Path, State};
use axum::Json;
use serde::Deserialize;
use serde_json::Value;
use utoipa::ToSchema;
use utoipa_axum::{router::OpenApiRouter, routes};
use uuid::Uuid;

use crate::auth::AuthAdmin;
use crate::error::ApiError;
use crate::openapi::{DeletedResponse, ErrorResponse};
use crate::state::AppState;
use vaultscope_db::models::Product;

pub fn routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(list, create))
        .routes(routes!(get_one, update, delete_product))
        .routes(routes!(sync_stock))
}

#[utoipa::path(
    get,
    path = "/",
    tag = "Admin Products",
    security(("bearer_auth" = [])),
    responses((status = 200, description = "List products", body = Vec<Product>))
)]
pub(crate) async fn list(
    auth: AuthAdmin,
    State(state): State<AppState>,
) -> Result<Json<Vec<Product>>, ApiError> {
    crate::require_permission!(auth, state, "products.view");
    let products: Vec<Product> = sqlx::query_as("SELECT * FROM products ORDER BY name")
        .fetch_all(&state.db)
        .await?;
    Ok(Json(products))
}

#[utoipa::path(
    get,
    path = "/{id}",
    tag = "Admin Products",
    security(("bearer_auth" = [])),
    params(("id" = Uuid, Path, description = "Product ID")),
    responses(
        (status = 200, description = "Product details", body = Product),
        (status = 404, description = "Product not found", body = ErrorResponse)
    )
)]
pub(crate) async fn get_one(
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

#[derive(Deserialize, ToSchema)]
pub(crate) struct CreateProduct {
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

#[utoipa::path(
    post,
    path = "/",
    tag = "Admin Products",
    security(("bearer_auth" = []), ("csrf_token" = [])),
    request_body = CreateProduct,
    responses((status = 200, description = "Product created", body = Product))
)]
pub(crate) async fn create(
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

#[utoipa::path(
    put,
    path = "/{id}",
    tag = "Admin Products",
    security(("bearer_auth" = []), ("csrf_token" = [])),
    params(("id" = Uuid, Path, description = "Product ID")),
    request_body = CreateProduct,
    responses(
        (status = 200, description = "Product updated", body = Product),
        (status = 404, description = "Product not found", body = ErrorResponse)
    )
)]
pub(crate) async fn update(
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

#[utoipa::path(
    post,
    path = "/{id}/sync",
    tag = "Admin Products",
    security(("bearer_auth" = []), ("csrf_token" = [])),
    params(("id" = Uuid, Path, description = "Product ID")),
    responses(
        (status = 200, description = "Product stock synchronized", body = Product),
        (status = 404, description = "Product not found", body = ErrorResponse)
    )
)]
pub(crate) async fn sync_stock(
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

    // ReliableSite bare-metal inventory uses live Order API / SOAP Inventory stock
    let new_stock = if product.provider == "reliablesite" {
        if let Ok(rs_id) = product.target.trim().parse::<i32>() {
            let rs = state.config.reliablesite_connector();
            let fetched_stock = if !rs.is_mock() {
                rs.order_api()
                    .get_product_stock(rs_id)
                    .await
                    .ok()
                    .map(|s| s.stock)
            } else {
                None
            };

            let stock_val = match fetched_stock {
                Some(s) => Some(s),
                None => rs
                    .inventory_soap()
                    .servers_list()
                    .await
                    .ok()
                    .and_then(|list| list.into_iter().find(|s| s.product_id == rs_id))
                    .map(|s| s.stock),
            };

            if let Some(s) = stock_val {
                let _ = sqlx::query(
                    "UPDATE reliablesite_servers SET stock = $1, synced_at = now() WHERE product_id = $2",
                )
                .bind(s)
                .bind(rs_id)
                .execute(&state.db)
                .await;
                s
            } else {
                product.stock
            }
        } else {
            product.stock
        }
    } else if !product.provider.is_empty() && product.provider != "manual" {
        // Cloud-provisioned products have unlimited stock (on-demand);
        // products without a provider keep their manually-set stock.
        -1
    } else {
        product.stock
    };

    let updated_product: Product = sqlx::query_as(
        "UPDATE products SET stock=$1, updated_at=now() WHERE id=$2 RETURNING *",
    )
    .bind(new_stock)
    .bind(id)
    .fetch_one(&state.db)
    .await?;

    Ok(Json(updated_product))
}

#[utoipa::path(
    delete,
    path = "/{id}",
    tag = "Admin Products",
    security(("bearer_auth" = []), ("csrf_token" = [])),
    params(("id" = Uuid, Path, description = "Product ID")),
    responses(
        (status = 200, description = "Product deleted", body = DeletedResponse),
        (status = 400, description = "Product has linked services", body = ErrorResponse),
        (status = 404, description = "Product not found", body = ErrorResponse)
    )
)]
pub(crate) async fn delete_product(
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
