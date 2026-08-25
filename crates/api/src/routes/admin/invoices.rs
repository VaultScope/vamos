use axum::extract::{Path, Query, State};
use axum::routing::get;
use axum::{Json, Router};
use chrono::{NaiveDate, Utc};
use serde::Deserialize;
use uuid::Uuid;

use crate::auth::AuthAdmin;
use crate::error::ApiError;
use crate::state::AppState;
use crate::validation::{
    validate_greater_than_zero, validate_percentage, validate_positive,
    validate_required_string, validate_string_length, MAX_SHORT_STRING_LENGTH, MAX_STRING_LENGTH,
};
use vaultscope_db::models::{Invoice, InvoiceLineItem, InvoiceStatus};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/", get(list).post(create))
        .route("/{id}", get(get_one).put(update).delete(delete))
        .route("/{id}/send", axum::routing::post(send))
        .route("/{id}/mark-paid", axum::routing::post(mark_paid))
        .route("/{id}/line-items", get(list_line_items))
}

#[derive(Deserialize)]
struct ListParams {
    customer_id: Option<Uuid>,
    status: Option<InvoiceStatus>,
    limit: Option<i64>,
    offset: Option<i64>,
}

async fn list(
    _auth: AuthAdmin,
    State(state): State<AppState>,
    Query(params): Query<ListParams>,
) -> Result<Json<Vec<Invoice>>, ApiError> {
    let limit = params.limit.unwrap_or(50).min(200);
    let offset = params.offset.unwrap_or(0);

    let invoices: Vec<Invoice> = match (params.customer_id, params.status) {
        (Some(cid), Some(status)) => {
            sqlx::query_as(
                "SELECT * FROM invoices WHERE customer_id = $1 AND status = $2::invoice_status ORDER BY created_at DESC LIMIT $3 OFFSET $4"
            )
            .bind(cid)
            .bind(status)
            .bind(limit)
            .bind(offset)
            .fetch_all(&state.db)
            .await?
        }
        (Some(cid), None) => {
            sqlx::query_as(
                "SELECT * FROM invoices WHERE customer_id = $1 ORDER BY created_at DESC LIMIT $2 OFFSET $3"
            )
            .bind(cid)
            .bind(limit)
            .bind(offset)
            .fetch_all(&state.db)
            .await?
        }
        (None, Some(status)) => {
            sqlx::query_as(
                "SELECT * FROM invoices WHERE status = $1::invoice_status ORDER BY created_at DESC LIMIT $2 OFFSET $3"
            )
            .bind(status)
            .bind(limit)
            .bind(offset)
            .fetch_all(&state.db)
            .await?
        }
        (None, None) => {
            sqlx::query_as(
                "SELECT * FROM invoices ORDER BY created_at DESC LIMIT $1 OFFSET $2"
            )
            .bind(limit)
            .bind(offset)
            .fetch_all(&state.db)
            .await?
        }
    };

    Ok(Json(invoices))
}

async fn get_one(
    _auth: AuthAdmin,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<Invoice>, ApiError> {
    let invoice: Invoice = sqlx::query_as("SELECT * FROM invoices WHERE id = $1")
        .bind(id)
        .fetch_optional(&state.db)
        .await?
        .ok_or(ApiError::NotFound("invoice not found".into()))?;
    Ok(Json(invoice))
}

#[derive(Deserialize)]
struct LineItemInput {
    description: String,
    quantity: i32,
    unit_price: f64,
    service_id: Option<Uuid>,
}

#[derive(Deserialize)]
struct CreateInvoice {
    customer_id: Uuid,
    due_date: NaiveDate,
    tax_rate: f64,
    notes: Option<String>,
    line_items: Vec<LineItemInput>,
}

async fn create(
    _auth: AuthAdmin,
    State(state): State<AppState>,
    Json(payload): Json<CreateInvoice>,
) -> Result<Json<Invoice>, ApiError> {
    // Validate inputs
    if payload.line_items.is_empty() {
        return Err(ApiError::BadRequest("At least one line item is required".into()));
    }

    for item in &payload.line_items {
        validate_required_string(&item.description, "Description")?;
        validate_string_length(&item.description, "Description", MAX_SHORT_STRING_LENGTH)?;
        validate_greater_than_zero(item.quantity, "Quantity")?;
        validate_positive(item.unit_price, "Unit price")?;
    }

    validate_percentage(payload.tax_rate, "Tax rate")?;

    if let Some(ref notes) = payload.notes {
        validate_string_length(notes, "Notes", MAX_STRING_LENGTH)?;
    }

    let invoice_number = format!("INV-{}", Uuid::new_v7(uuid::Timestamp::now(uuid::NoContext)).to_string()[..8].to_uppercase());

    let subtotal: f64 = payload.line_items.iter()
        .map(|item| item.quantity as f64 * item.unit_price)
        .sum();

    let tax_amount = subtotal * (payload.tax_rate / 100.0);
    let total = subtotal + tax_amount;

    let invoice: Invoice = sqlx::query_as(
        r#"INSERT INTO invoices (invoice_number, customer_id, status, subtotal, tax_rate, tax_amount, total, issue_date, due_date, notes)
           VALUES ($1, $2, $3::invoice_status, $4, $5, $6, $7, CURRENT_DATE, $8, $9)
           RETURNING *"#
    )
    .bind(&invoice_number)
    .bind(payload.customer_id)
    .bind(InvoiceStatus::Draft)
    .bind(rust_decimal::Decimal::from_f64_retain(subtotal).unwrap_or_default())
    .bind(rust_decimal::Decimal::from_f64_retain(payload.tax_rate).unwrap_or_default())
    .bind(rust_decimal::Decimal::from_f64_retain(tax_amount).unwrap_or_default())
    .bind(rust_decimal::Decimal::from_f64_retain(total).unwrap_or_default())
    .bind(payload.due_date)
    .bind(payload.notes.unwrap_or_default())
    .fetch_one(&state.db)
    .await?;

    for (idx, item) in payload.line_items.iter().enumerate() {
        let item_total = item.quantity as f64 * item.unit_price;
        sqlx::query(
            r#"INSERT INTO invoice_line_items (invoice_id, description, quantity, unit_price, total, service_id, sort_order)
               VALUES ($1, $2, $3, $4, $5, $6, $7)"#
        )
        .bind(invoice.id)
        .bind(&item.description)
        .bind(item.quantity)
        .bind(rust_decimal::Decimal::from_f64_retain(item.unit_price).unwrap_or_default())
        .bind(rust_decimal::Decimal::from_f64_retain(item_total).unwrap_or_default())
        .bind(item.service_id)
        .bind(idx as i32)
        .execute(&state.db)
        .await?;
    }

    Ok(Json(invoice))
}

#[derive(Deserialize)]
struct UpdateInvoice {
    status: Option<InvoiceStatus>,
    due_date: Option<NaiveDate>,
    notes: Option<String>,
}

async fn update(
    _auth: AuthAdmin,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(payload): Json<UpdateInvoice>,
) -> Result<Json<Invoice>, ApiError> {
    let invoice: Invoice = sqlx::query_as(
        r#"UPDATE invoices
           SET status = COALESCE($1::invoice_status, status),
               due_date = COALESCE($2, due_date),
               notes = COALESCE($3, notes),
               updated_at = now()
           WHERE id = $4
           RETURNING *"#
    )
    .bind(payload.status)
    .bind(payload.due_date)
    .bind(payload.notes)
    .bind(id)
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound("invoice not found".into()))?;

    Ok(Json(invoice))
}

async fn delete(
    _auth: AuthAdmin,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<()>, ApiError> {
    let result = sqlx::query("DELETE FROM invoices WHERE id = $1 AND status = 'draft'::invoice_status")
        .bind(id)
        .execute(&state.db)
        .await?;

    if result.rows_affected() == 0 {
        return Err(ApiError::NotFound("draft invoice not found".into()));
    }

    Ok(Json(()))
}

async fn send(
    _auth: AuthAdmin,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<Invoice>, ApiError> {
    let invoice: Invoice = sqlx::query_as(
        r#"UPDATE invoices
           SET status = 'pending'::invoice_status,
               updated_at = now()
           WHERE id = $1 AND status = 'draft'::invoice_status
           RETURNING *"#
    )
    .bind(id)
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound("draft invoice not found".into()))?;

    Ok(Json(invoice))
}

async fn mark_paid(
    _auth: AuthAdmin,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<Invoice>, ApiError> {
    let invoice: Invoice = sqlx::query_as(
        r#"UPDATE invoices
           SET status = 'paid'::invoice_status,
               paid_at = now(),
               updated_at = now()
           WHERE id = $1
           RETURNING *"#
    )
    .bind(id)
    .fetch_one(&state.db)
    .await?;

    Ok(Json(invoice))
}

async fn list_line_items(
    _auth: AuthAdmin,
    State(state): State<AppState>,
    Path(invoice_id): Path<Uuid>,
) -> Result<Json<Vec<InvoiceLineItem>>, ApiError> {
    let items: Vec<InvoiceLineItem> = sqlx::query_as(
        "SELECT * FROM invoice_line_items WHERE invoice_id = $1 ORDER BY sort_order"
    )
    .bind(invoice_id)
    .fetch_all(&state.db)
    .await?;

    Ok(Json(items))
}
