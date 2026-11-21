use axum::extract::State;
use axum::Json;
use utoipa_axum::{router::OpenApiRouter, routes};

use crate::error::ApiError;
use crate::openapi::ErrorResponse;
use crate::reliablesite_catalog::{load_pricing_policy, sanitize_product_for_storefront_with_policy};
use crate::state::AppState;
use vaultscope_db::models::Product;

pub fn routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new().routes(routes!(list_visible))
}

#[utoipa::path(
    get,
    path = "/",
    tag = "Storefront Catalog",
    responses(
        (status = 200, description = "Visible products in storefront catalog", body = Vec<Product>),
        (status = 500, description = "Internal error", body = ErrorResponse)
    )
)]
pub(crate) async fn list_visible(State(state): State<AppState>) -> Result<Json<Vec<Product>>, ApiError> {
    let policy = load_pricing_policy(&state.db, &state.config).await;
    let products: Vec<Product> =
        sqlx::query_as("SELECT * FROM products WHERE hidden = false ORDER BY category, name")
            .fetch_all(&state.db)
            .await?;
    Ok(Json(
        products
            .into_iter()
            .map(|p| sanitize_product_for_storefront_with_policy(p, policy))
            .collect(),
    ))
}
