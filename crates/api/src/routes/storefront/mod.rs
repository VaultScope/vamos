pub mod activity;
pub mod billing;
pub mod blog;
pub mod catalog;
pub mod config;
pub mod dedicated;
pub mod invoices;
pub mod meta;
pub mod options;
pub mod orders;
pub mod partners;
pub mod profile;
pub mod services;
pub mod tickets;

use utoipa_axum::router::OpenApiRouter;

use crate::state::AppState;

pub fn routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .nest("/activity", activity::routes())
        .nest("/billing", billing::routes())
        .nest("/blog", blog::routes())
        .nest("/catalog", catalog::routes())
        .nest("/config", config::routes())
        .nest("/dedicated", dedicated::routes())
        .nest("/invoices", invoices::routes())
        .nest("/meta", meta::routes())
        .nest("/options", options::routes())
        .nest("/orders", orders::routes())
        .nest("/partners", partners::routes())
        .nest("/ps", partners::routes())
        .nest("/profile", profile::routes())
        .nest("/services", services::routes())
        .nest("/tickets", tickets::routes())
}
