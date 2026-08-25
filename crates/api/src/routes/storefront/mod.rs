pub mod catalog;
pub mod orders;
pub mod services;

use axum::Router;

use crate::state::AppState;

pub fn routes() -> Router<AppState> {
    Router::new()
        .nest("/catalog", catalog::routes())
        .nest("/orders", orders::routes())
        .nest("/services", services::routes())
}
