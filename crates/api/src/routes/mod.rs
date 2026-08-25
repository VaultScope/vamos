pub mod admin;
pub mod auth;
pub mod storefront;
pub mod health;

use axum::{Router, Json};
use axum_csrf::CsrfToken;
use serde_json::{json, Value};

use crate::state::AppState;
use crate::middleware::rate_limit::{rate_limit_middleware, RateLimiter};
use crate::middleware::csrf::csrf_middleware;

async fn csrf_token(token: CsrfToken) -> (CsrfToken, Json<Value>) {
    (token.clone(), Json(json!({ "token": token.authenticity_token().unwrap_or_default() })))
}

pub fn build_router(state: AppState) -> Router {
    let admin_rate_limiter = RateLimiter::new(100, 60); // 100 requests per 60 seconds

    Router::new()
        .route("/api/csrf", axum::routing::get(csrf_token))
        .nest("/api/health", health::routes())
        .nest("/api/auth", auth::routes())
        .nest(
            "/api/admin",
            admin::routes()
                .layer(axum::middleware::from_fn(rate_limit_middleware))
                .layer(axum::extract::Extension(admin_rate_limiter)),
        )
        .nest("/api/storefront", storefront::routes())
        .layer(axum::middleware::from_fn_with_state(state.clone(), csrf_middleware))
        .with_state(state)
}
