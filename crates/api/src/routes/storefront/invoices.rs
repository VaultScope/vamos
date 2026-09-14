use axum::extract::State;
use axum::routing::get;
use axum::{Json, Router};

use crate::auth::AuthCustomer;
use crate::error::ApiError;
use crate::state::AppState;
use vaultscope_db::models::Invoice;

pub fn routes() -> Router<AppState> {
    Router::new().route("/", get(list_mine))
}

async fn list_mine(
    auth: AuthCustomer,
    State(state): State<AppState>,
) -> Result<Json<Vec<Invoice>>, ApiError> {
    let invoices: Vec<Invoice> =
        sqlx::query_as("SELECT * FROM invoices WHERE customer_id = $1 ORDER BY issue_date DESC")
            .bind(auth.0.sub)
            .fetch_all(&state.db)
            .await?;
    Ok(Json(invoices))
}
