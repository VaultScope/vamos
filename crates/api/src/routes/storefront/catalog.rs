use axum::extract::State;
use axum::routing::get;
use axum::{Json, Router};

use crate::error::ApiError;
use crate::state::AppState;
use vaultscope_db::models::Product;

pub fn routes() -> Router<AppState> {
    Router::new().route("/", get(list_visible))
}

async fn list_visible(
    State(state): State<AppState>,
) -> Result<Json<Vec<Product>>, ApiError> {
    let products: Vec<Product> = sqlx::query_as(
        "SELECT * FROM products WHERE hidden = false ORDER BY category, name"
    )
    .fetch_all(&state.db)
    .await?;
    Ok(Json(products))
}
