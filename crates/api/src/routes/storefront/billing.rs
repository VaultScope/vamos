use axum::extract::State;
use axum::Json;
use chrono::Utc;
use serde::Serialize;
use utoipa::ToSchema;
use utoipa_axum::{router::OpenApiRouter, routes};

use crate::auth::AuthCustomer;
use crate::error::ApiError;
use crate::openapi::ErrorResponse;
use crate::state::AppState;

pub fn routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new().routes(routes!(billing_info))
}

#[derive(Serialize, ToSchema)]
pub(crate) struct BillingInfo {
    balance: String,
    amount_due: String,
    has_unpaid_invoices: bool,
    has_overdue_invoices: bool,
    open_invoices_count: i64,
    overdue_invoices_count: i64,
    next_charge: Option<String>,
    upcoming_renewals_count: usize,
    status: String,
    payment_provider: String,
    card_last4: Option<String>,
    card_exp: Option<String>,
    address: Option<BillingAddress>,
    vat_id: Option<String>,
    subscriptions: Vec<Subscription>,
}

#[derive(Serialize, ToSchema)]
pub(crate) struct BillingAddress {
    name: String,
    company: String,
    line1: String,
    country: String,
}

#[derive(Serialize, ToSchema)]
pub(crate) struct Subscription {
    id: String,
    name: String,
    description: String,
    price: String,
    next_date: String,
    cancel_at_period_end: bool,
    days_until_due: Option<i64>,
    due_soon: bool,
}

#[utoipa::path(
    get,
    path = "/",
    tag = "Storefront Billing",
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "Customer billing summary, open balances, and active subscriptions", body = BillingInfo),
        (status = 401, description = "Unauthorized", body = ErrorResponse),
        (status = 404, description = "Customer not found", body = ErrorResponse)
    )
)]
pub(crate) async fn billing_info(
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

    let (due_total, open_invoices_count, overdue_invoices_count): (
        rust_decimal::Decimal,
        i64,
        i64,
    ) = sqlx::query_as(
        r#"SELECT
               COALESCE(SUM(total), 0),
               COUNT(*) FILTER (WHERE status IN ('pending', 'overdue')),
               COUNT(*) FILTER (WHERE status = 'overdue')
           FROM invoices
           WHERE customer_id = $1 AND status IN ('pending', 'overdue')"#,
    )
    .bind(auth.0.sub)
    .fetch_one(&state.db)
    .await?;

    let balance = format!("€{:.2}", due_total);
    let has_unpaid_invoices = open_invoices_count > 0;
    let has_overdue_invoices = overdue_invoices_count > 0;

    let status = if overdue_invoices_count > 0 {
        format!(
            "{} overdue invoice{} requiring immediate payment",
            overdue_invoices_count,
            if overdue_invoices_count == 1 { "" } else { "s" }
        )
    } else if open_invoices_count > 0 {
        format!(
            "{} open invoice{} awaiting payment",
            open_invoices_count,
            if open_invoices_count == 1 { "" } else { "s" }
        )
    } else {
        "No outstanding balance".to_string()
    };

    let services: Vec<(String, String, rust_decimal::Decimal, Option<chrono::DateTime<chrono::Utc>>, bool)> = sqlx::query_as(
        "SELECT s.id::text, p.name, s.price, s.next_due, s.cancel_at_period_end FROM services s JOIN products p ON s.product_id = p.id WHERE s.customer_id = $1 AND s.status = 'running' ORDER BY s.next_due ASC NULLS LAST"
    )
    .bind(auth.0.sub)
    .fetch_all(&state.db)
    .await?;

    let now = Utc::now();
    let mut next_charge: Option<String> = None;
    let mut upcoming_renewals_count = 0usize;

    let subscriptions: Vec<Subscription> = services
        .iter()
        .map(|(id, pname, price, next_due, cancel_at_period_end)| {
            let days_until_due = next_due.map(|d| (d - now).num_days());
            let due_soon = !*cancel_at_period_end
                && *price > rust_decimal::Decimal::ZERO
                && days_until_due.is_some_and(|days| (0..=7).contains(&days));

            if due_soon {
                upcoming_renewals_count += 1;
            }
            if !*cancel_at_period_end && next_charge.is_none() {
                if let Some(d) = next_due {
                    next_charge = Some(d.format("%Y-%m-%d").to_string());
                }
            }

            Subscription {
                id: id.clone(),
                name: pname.clone(),
                description: if *cancel_at_period_end {
                    "Cancelling at period end".to_string()
                } else if due_soon {
                    "Renewal due within 7 days".to_string()
                } else {
                    "Active service".to_string()
                },
                price: format!("€{:.2}/mo", price),
                next_date: next_due
                    .map(|d| d.format("%Y-%m-%d").to_string())
                    .unwrap_or_else(|| "—".to_string()),
                cancel_at_period_end: *cancel_at_period_end,
                days_until_due,
                due_soon,
            }
        })
        .collect();

    Ok(Json(BillingInfo {
        balance: balance.clone(),
        amount_due: balance,
        has_unpaid_invoices,
        has_overdue_invoices,
        open_invoices_count,
        overdue_invoices_count,
        next_charge,
        upcoming_renewals_count,
        status,
        payment_provider: "Stripe".to_string(),
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
