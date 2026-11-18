pub mod stripe;

use utoipa_axum::{router::OpenApiRouter, routes};

use crate::state::AppState;

pub fn routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new().routes(routes!(stripe::handle_stripe_webhook))
}
