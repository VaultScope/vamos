use axum::extract::State;
use axum::routing::get;
use axum::{Json, Router};
use serde::Serialize;

use crate::auth::AuthAdmin;
use crate::error::ApiError;
use crate::state::AppState;

pub fn routes() -> Router<AppState> {
    Router::new().route("/stats", get(stats))
}

#[derive(Serialize)]
struct DashboardStats {
    active_services: i64,
    total_customers: i64,
    open_tickets: i64,
    pending_jobs: i64,
    mrr: f64,
}

async fn stats(
    _auth: AuthAdmin,
    State(state): State<AppState>,
) -> Result<Json<DashboardStats>, ApiError> {
    let (active_services,): (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM services WHERE status IN ('running', 'pending')"
    ).fetch_one(&state.db).await?;

    let (total_customers,): (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM customers WHERE status = 'active'"
    ).fetch_one(&state.db).await?;

    let (open_tickets,): (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM tickets WHERE status IN ('open', 'in_progress')"
    ).fetch_one(&state.db).await?;

    let (pending_jobs,): (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM jobs WHERE status IN ('queued', 'in_progress')"
    ).fetch_one(&state.db).await?;

    let mrr_row: Option<(rust_decimal::Decimal,)> = sqlx::query_as(
        "SELECT COALESCE(SUM(price), 0) FROM services WHERE status = 'running'"
    ).fetch_optional(&state.db).await?;
    let mrr = mrr_row.map(|(v,)| v.to_string().parse::<f64>().unwrap_or(0.0)).unwrap_or(0.0);

    Ok(Json(DashboardStats {
        active_services,
        total_customers,
        open_tickets,
        pending_jobs,
        mrr,
    }))
}
