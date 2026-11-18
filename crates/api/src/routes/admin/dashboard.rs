use axum::extract::State;
use axum::Json;
use serde::Serialize;
use utoipa::ToSchema;
use utoipa_axum::{router::OpenApiRouter, routes};

use crate::auth::AuthAdmin;
use crate::error::ApiError;
use crate::openapi::ErrorResponse;
use crate::state::AppState;

pub fn routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(stats))
        .routes(routes!(export_invoices))
        .routes(routes!(export_customers))
}

#[derive(Serialize, ToSchema)]
pub(crate) struct DashboardStats {
    active_services: i64,
    total_customers: i64,
    open_tickets: i64,
    pending_jobs: i64,
    mrr: f64,
}

#[utoipa::path(
    get,
    path = "/stats",
    tag = "Admin Dashboard",
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "Admin dashboard summary metrics", body = DashboardStats),
        (status = 403, description = "Forbidden", body = ErrorResponse)
    )
)]
pub(crate) async fn stats(
    auth: AuthAdmin,
    State(state): State<AppState>,
) -> Result<Json<DashboardStats>, ApiError> {
    crate::require_permission!(auth, state, "staff.view");
    let (active_services,): (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM services WHERE status IN ('running', 'pending')")
            .fetch_one(&state.db)
            .await?;

    let (total_customers,): (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM customers WHERE status = 'active'")
            .fetch_one(&state.db)
            .await?;

    let (open_tickets,): (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM tickets WHERE status IN ('open', 'in_progress')")
            .fetch_one(&state.db)
            .await?;

    let (pending_jobs,): (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM jobs WHERE status IN ('queued', 'in_progress')")
            .fetch_one(&state.db)
            .await?;

    let mrr_row: Option<(rust_decimal::Decimal,)> =
        sqlx::query_as("SELECT COALESCE(SUM(price), 0) FROM services WHERE status = 'running'")
            .fetch_optional(&state.db)
            .await?;
    let mrr = mrr_row
        .map(|(v,)| v.to_string().parse::<f64>().unwrap_or(0.0))
        .unwrap_or(0.0);

    Ok(Json(DashboardStats {
        active_services,
        total_customers,
        open_tickets,
        pending_jobs,
        mrr,
    }))
}

#[utoipa::path(
    get,
    path = "/export/invoices",
    tag = "Admin Dashboard",
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "Invoices CSV export", content_type = "text/csv", body = String),
        (status = 403, description = "Forbidden", body = ErrorResponse)
    )
)]
pub(crate) async fn export_invoices(
    auth: AuthAdmin,
    State(state): State<AppState>,
) -> Result<impl axum::response::IntoResponse, ApiError> {
    crate::require_permission!(auth, state, "invoices.view");
    let invoices: Vec<(uuid::Uuid, String, rust_decimal::Decimal, String)> =
        sqlx::query_as("SELECT id, invoice_number, total, status FROM invoices")
            .fetch_all(&state.db)
            .await?;

    let mut csv = String::from("id,invoice_number,total,status\n");
    for i in invoices {
        csv.push_str(&format!(
            "{},{},{},{}\n",
            i.0,
            csv_escape(&i.1),
            i.2,
            csv_escape(&i.3)
        ));
    }

    Ok((
        axum::http::StatusCode::OK,
        [(axum::http::header::CONTENT_TYPE, "text/csv")],
        csv,
    ))
}

#[utoipa::path(
    get,
    path = "/export/customers",
    tag = "Admin Dashboard",
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "Customers CSV export", content_type = "text/csv", body = String),
        (status = 403, description = "Forbidden", body = ErrorResponse)
    )
)]
pub(crate) async fn export_customers(
    auth: AuthAdmin,
    State(state): State<AppState>,
) -> Result<impl axum::response::IntoResponse, ApiError> {
    crate::require_permission!(auth, state, "customers.view");
    let rows: Vec<(uuid::Uuid, String, String)> =
        sqlx::query_as("SELECT id, name, email FROM customers")
            .fetch_all(&state.db)
            .await?;

    let mut csv = String::from("id,name,email\n");
    for i in rows {
        csv.push_str(&format!(
            "{},{},{}\n",
            i.0,
            csv_escape(&i.1),
            csv_escape(&i.2)
        ));
    }

    Ok((
        axum::http::StatusCode::OK,
        [(axum::http::header::CONTENT_TYPE, "text/csv")],
        csv,
    ))
}

fn csv_escape(value: &str) -> String {
    if value.contains(',') || value.contains('"') || value.contains('\n') {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value.to_string()
    }
}
