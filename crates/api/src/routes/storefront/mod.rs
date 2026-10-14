pub mod activity;
pub mod billing;
pub mod blog;
pub mod catalog;
pub mod config;
pub mod invoices;
pub mod options;
pub mod orders;
pub mod partners;
pub mod profile;
pub mod services;
pub mod tickets;

use axum::Router;

use crate::state::AppState;

pub fn routes() -> Router<AppState> {
    Router::new()
        .nest("/activity", activity::routes())
        .nest("/billing", billing::routes())
        .nest("/blog", blog::routes())
        .nest("/catalog", catalog::routes())
        .nest("/config", config::routes())
        .nest("/invoices", invoices::routes())
        .nest("/options", options::routes())
        .nest("/orders", orders::routes())
        .nest("/partners", partners::routes())
        .nest("/ps", partners::routes())
        .nest("/profile", profile::routes())
        .nest("/services", services::routes())
        .nest("/tickets", tickets::routes())
}
