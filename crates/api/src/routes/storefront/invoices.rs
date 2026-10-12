use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::header;
use axum::response::Response;
use axum::routing::get;
use axum::{Json, Router};
use uuid::Uuid;

use crate::auth::AuthCustomer;
use crate::error::ApiError;
use crate::state::AppState;
use vaultscope_db::models::{Customer, Invoice, InvoiceLineItem};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/", get(list_mine))
        .route("/{id}/pdf", get(download_pdf))
}

async fn list_mine(
    auth: AuthCustomer,
    State(state): State<AppState>,
) -> Result<Json<Vec<Invoice>>, ApiError> {
    let invoices: Vec<Invoice> =
        sqlx::query_as("SELECT * FROM invoices WHERE customer_id = $1 ORDER BY issue_date DESC")
            .bind(auth.0.sub)
            .fetch_all(&state.db)
            .await?;
    Ok(Json(invoices))
}

async fn download_pdf(
    auth: AuthCustomer,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Response, ApiError> {
    let invoice: Invoice = sqlx::query_as(
        "SELECT * FROM invoices WHERE id = $1 AND customer_id = $2"
    )
    .bind(id)
    .bind(auth.0.sub)
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound("invoice not found".into()))?;

    let customer: Customer = sqlx::query_as(
        "SELECT * FROM customers WHERE id = $1"
    )
    .bind(auth.0.sub)
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound("customer not found".into()))?;

    let line_items: Vec<InvoiceLineItem> = sqlx::query_as(
        "SELECT * FROM invoice_line_items WHERE invoice_id = $1 ORDER BY sort_order ASC, id ASC"
    )
    .bind(id)
    .fetch_all(&state.db)
    .await?;

    let pdf_bytes = crate::pdf::generate_invoice_pdf(&invoice, &customer, &line_items);
    let filename = format!("invoice-{}.pdf", invoice.invoice_number);

    let response = Response::builder()
        .header(header::CONTENT_TYPE, "application/pdf")
        .header(
            header::CONTENT_DISPOSITION,
            format!("inline; filename=\"{}\"", filename),
        )
        .body(Body::from(pdf_bytes))
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    Ok(response)
}
