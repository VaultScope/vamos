pub mod admin;
pub mod auth;
pub mod storefront;
pub mod health;

use axum::Router;

use crate::state::AppState;

pub fn build_router(state: AppState) -> Router {
    Router::new()
        .nest("/api/health", health::routes())
        .nest("/api/auth", auth::routes())
        .nest("/api/admin", admin::routes())
        .nest("/api/storefront", storefront::routes())
        .with_state(state)
}
