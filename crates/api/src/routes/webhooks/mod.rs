pub mod stripe;

use axum::{routing::post, Router};
use crate::state::AppState;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/stripe", post(stripe::handle_stripe_webhook))
}
