use axum::extract::State;
use axum::routing::get;
use axum::{Json, Router};
use serde::Serialize;

use crate::auth::AuthCustomer;
use crate::error::ApiError;
use crate::state::AppState;

pub fn routes() -> Router<AppState> {
    Router::new().route("/", get(billing_info))
}

#[derive(Serialize)]
struct BillingInfo {
    balance: String,
    next_charge: Option<String>,
    status: String,
    card_last4: Option<String>,
    card_exp: Option<String>,
    address: Option<BillingAddress>,
    vat_id: Option<String>,
    subscriptions: Vec<Subscription>,
}

#[derive(Serialize)]
struct BillingAddress {
    name: String,
    company: String,
    line1: String,
    country: String,
}

#[derive(Serialize)]
struct Subscription {
    id: String,
    name: String,
    description: String,
    price: String,
    next_date: String,
    cancel_at_period_end: bool,
}

async fn billing_info(
    auth: AuthCustomer,
    State(state): State<AppState>,
) -> Result<Json<BillingInfo>, ApiError> {
    let customer: Option<(String, String, String, String, String)> = sqlx::query_as(
        "SELECT name, company, address, country, vat_id FROM customers WHERE id = $1",
    )
    .bind(auth.0.sub)
    .fetch_optional(&state.db)
    .await?;

    let (name, company, address_line, country, vat_id) =
        customer.ok_or(ApiError::NotFound("customer not found".into()))?;

    let due: Option<(rust_decimal::Decimal,)> = sqlx::query_as(
        "SELECT COALESCE(SUM(total), 0) FROM invoices WHERE customer_id = $1 AND status IN ('pending', 'overdue')",
    )
    .bind(auth.0.sub)
    .fetch_optional(&state.db)
    .await?;

    let balance = due
        .map(|(v,)| format!("€{:.2}", v))
        .unwrap_or_else(|| "€0.00".to_string());

    let services: Vec<(String, String, rust_decimal::Decimal, Option<chrono::DateTime<chrono::Utc>>, bool)> = sqlx::query_as(
        "SELECT s.id::text, p.name, s.price, s.next_due, s.cancel_at_period_end FROM services s JOIN products p ON s.product_id = p.id WHERE s.customer_id = $1 AND s.status = 'running'"
    )
    .bind(auth.0.sub)
    .fetch_all(&state.db)
    .await?;

    let subscriptions = services
        .iter()
        .map(|(id, pname, price, next_due, cancel_at_period_end)| Subscription {
            id: id.clone(),
            name: pname.clone(),
            description: if *cancel_at_period_end {
                "Cancelling at period end".to_string()
            } else {
                "Active service".to_string()
            },
            price: format!("€{:.2}/mo", price),
            next_date: next_due
                .map(|d| d.format("%Y-%m-%d").to_string())
                .unwrap_or_else(|| "—".to_string()),
            cancel_at_period_end: *cancel_at_period_end,
        })
        .collect();

    Ok(Json(BillingInfo {
        balance,
        next_charge: None,
        status: "Payment current".to_string(),
        card_last4: None,
        card_exp: None,
        address: Some(BillingAddress {
            name,
            company,
            line1: address_line,
            country,
        }),
        vat_id: if vat_id.is_empty() {
            None
        } else {
            Some(vat_id)
        },
        subscriptions,
    }))
}
