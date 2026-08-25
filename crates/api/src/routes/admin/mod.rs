pub mod activity;
pub mod connectors;
pub mod customers;
pub mod dashboard;
pub mod email_templates;
pub mod notifications;
pub mod products;
pub mod services;
pub mod settings;
pub mod staff;
pub mod tickets;

use axum::Router;

use crate::state::AppState;

pub fn routes() -> Router<AppState> {
    Router::new()
        .nest("/activity", activity::routes())
        .nest("/connectors", connectors::routes())
        .nest("/customers", customers::routes())
        .nest("/dashboard", dashboard::routes())
        .nest("/email-templates", email_templates::routes())
        .nest("/notifications", notifications::routes())
        .nest("/products", products::routes())
        .nest("/services", services::routes())
        .nest("/settings", settings::routes())
        .nest("/staff", staff::routes())
        .nest("/tickets", tickets::routes())
}
