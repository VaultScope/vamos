use std::collections::BTreeMap;

use axum::extract::{Path, State};
use axum::Json;
use serde::Deserialize;
use utoipa::ToSchema;
use utoipa_axum::{router::OpenApiRouter, routes};
use uuid::Uuid;

use crate::auth::AuthCustomer;
use crate::error::ApiError;
use crate::openapi::ErrorResponse;
use crate::reliablesite_catalog::{
    self, DedicatedProductOptionsResponse, DedicatedQuote,
};
use crate::state::AppState;
use vaultscope_db::models::Product;

pub fn routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(get_product_options))
        .routes(routes!(quote_product))
}

#[utoipa::path(
    get,
    path = "/products/{id}/options",
    tag = "Storefront Dedicated",
    security(("bearer_auth" = [])),
    params(("id" = Uuid, Path, description = "VaultScope product ID")),
    responses(
        (status = 200, description = "Dedicated server configuration groups, addons, and available billing cycles", body = DedicatedProductOptionsResponse),
        (status = 400, description = "Product does not support dedicated hardware configuration", body = ErrorResponse),
        (status = 404, description = "Product or server configuration not found", body = ErrorResponse)
    )
)]
pub(crate) async fn get_product_options(
    _auth: AuthCustomer,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<DedicatedProductOptionsResponse>, ApiError> {
    let product = sqlx::query_as::<_, Product>("SELECT * FROM products WHERE id = $1 AND hidden = false")
        .bind(id)
        .fetch_optional(&state.db)
        .await?
        .ok_or_else(|| ApiError::NotFound("product not found".into()))?;

    if product.provider != "reliablesite" {
        return Err(ApiError::BadRequest(
            "Selected product does not support dedicated hardware configuration".into(),
        ));
    }

    let mut live_stock = None;
    let rs = state.config.reliablesite_connector();
    if !rs.is_mock() {
        if let Ok(rs_id) = product.target.trim().parse::<i32>() {
            if let Ok(stock_resp) = rs.order_api().get_product_stock(rs_id).await {
                let _ = sqlx::query("UPDATE products SET stock = $1, updated_at = now() WHERE id = $2")
                    .bind(stock_resp.stock)
                    .bind(product.id)
                    .execute(&state.db)
                    .await;
                let _ = sqlx::query(
                    "UPDATE reliablesite_servers SET stock = $1, synced_at = now() WHERE product_id = $2",
                )
                .bind(stock_resp.stock)
                .bind(rs_id)
                .execute(&state.db)
                .await;
                live_stock = Some(stock_resp.stock);
            }
        }
    }

    let policy = reliablesite_catalog::load_pricing_policy(&state.db, &state.config).await;
    let resp = reliablesite_catalog::get_dedicated_options_with_policy(
        &state.db,
        &product,
        live_stock,
        policy,
    )
    .await?;
    Ok(Json(resp))
}

#[derive(Deserialize, ToSchema)]
pub(crate) struct DedicatedQuoteRequest {
    billing_cycle: Option<String>,
    config_options: Option<BTreeMap<String, u32>>,
}

#[utoipa::path(
    post,
    path = "/products/{id}/quote",
    tag = "Storefront Dedicated",
    security(("bearer_auth" = []), ("csrf_token" = [])),
    params(("id" = Uuid, Path, description = "VaultScope product ID")),
    request_body = DedicatedQuoteRequest,
    responses(
        (status = 200, description = "Authoritative pricing breakdown for dedicated server configuration", body = DedicatedQuote),
        (status = 400, description = "Invalid billing cycle or config option mapping", body = ErrorResponse),
        (status = 404, description = "Product not found", body = ErrorResponse)
    )
)]
pub(crate) async fn quote_product(
    _auth: AuthCustomer,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(payload): Json<DedicatedQuoteRequest>,
) -> Result<Json<DedicatedQuote>, ApiError> {
    let product = sqlx::query_as::<_, Product>("SELECT * FROM products WHERE id = $1 AND hidden = false")
        .bind(id)
        .fetch_optional(&state.db)
        .await?
        .ok_or_else(|| ApiError::NotFound("product not found".into()))?;

    if product.provider != "reliablesite" {
        return Err(ApiError::BadRequest(
            "Selected product does not support dedicated hardware configuration".into(),
        ));
    }

    let policy = reliablesite_catalog::load_pricing_policy(&state.db, &state.config).await;
    let tax_policy = reliablesite_catalog::load_tax_policy(&state.db, &state.config).await;
    let quote = reliablesite_catalog::compute_quote_with_policy(
        &state.db,
        &product,
        payload.billing_cycle.as_deref(),
        payload.config_options.as_ref(),
        policy,
        &tax_policy,
    )
    .await?;

    Ok(Json(quote.into_white_label()))
}
