pub mod activity;
pub mod blog;
pub mod coupons;
pub mod customers;
pub mod dashboard;
pub mod departments;
pub mod email_logs;
pub mod email_templates;
pub mod internal_tools;
pub mod invoices;
pub mod jobs;
pub mod mailboxes;
pub mod media;
pub mod notifications;
pub mod partners;
pub mod products;
pub mod service_forms;
pub mod services;
pub mod settings;
pub mod staff;
pub mod tax_rates;
pub mod tickets;

use axum::Router;

use crate::state::AppState;

pub fn routes() -> Router<AppState> {
    Router::new()
        .nest("/activity", activity::routes())
        .nest("/blog", blog::routes())
        .nest("/coupons", coupons::routes())
        .nest("/customers", customers::routes())
        .nest("/dashboard", dashboard::routes())
        .nest("/departments", departments::routes())
        .nest("/email-logs", email_logs::routes())
        .nest("/email-templates", email_templates::routes())
        .nest("/internal-tools", internal_tools::routes())
        .nest("/invoices", invoices::routes())
        .nest("/jobs", jobs::routes())
        .nest("/media", media::routes())
        .nest("/notifications", notifications::routes())
        .nest("/partners", partners::routes())
        .nest("/products", products::routes())
        .nest("/services", services::routes())
        .nest("/service-forms", service_forms::routes())
        .nest("/settings", settings::routes())
        .nest("/staff", staff::routes())
        .nest("/tax-rates", tax_rates::routes())
        .nest("/tickets", tickets::routes())
        .nest("/mailboxes", mailboxes::routes())
}
