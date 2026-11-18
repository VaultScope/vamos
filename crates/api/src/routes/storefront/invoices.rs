use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::header;
use axum::response::Response;
use axum::Json;
use rust_decimal::prelude::ToPrimitive;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use utoipa::ToSchema;
use utoipa_axum::{router::OpenApiRouter, routes};
use uuid::Uuid;

use crate::auth::AuthCustomer;
use crate::error::ApiError;
use crate::openapi::ErrorResponse;
use crate::state::AppState;
use vaultscope_db::models::{Customer, Invoice, InvoiceLineItem, InvoiceStatus};

pub fn routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(list_mine))
        .routes(routes!(download_pdf))
        .routes(routes!(create_invoice_payment_intent))
        .routes(routes!(pay_invoice))
}

#[allow(dead_code)]
#[derive(Serialize, ToSchema)]
pub struct InvoicePaymentIntentResponse {
    pub client_secret: String,
    pub payment_intent_id: String,
    pub amount: rust_decimal::Decimal,
    pub invoice_number: String,
}

#[utoipa::path(
    get,
    path = "/",
    tag = "Storefront Invoices",
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "Customer invoices excluding drafts", body = Vec<Invoice>),
        (status = 401, description = "Unauthorized", body = ErrorResponse)
    )
)]
pub(crate) async fn list_mine(
    auth: AuthCustomer,
    State(state): State<AppState>,
) -> Result<Json<Vec<Invoice>>, ApiError> {
    let invoices: Vec<Invoice> = sqlx::query_as(
        "SELECT * FROM invoices WHERE customer_id = $1 AND status != 'draft'::invoice_status ORDER BY issue_date DESC, created_at DESC",
    )
    .bind(auth.0.sub)
    .fetch_all(&state.db)
    .await?;
    Ok(Json(invoices))
}

#[utoipa::path(
    post,
    path = "/{id}/create-payment-intent",
    tag = "Storefront Invoices",
    security(("bearer_auth" = []), ("csrf_token" = [])),
    params(("id" = Uuid, Path, description = "Invoice ID")),
    responses(
        (status = 200, description = "Stripe PaymentIntent client secret for an open invoice", body = InvoicePaymentIntentResponse),
        (status = 400, description = "Invoice cannot be paid", body = ErrorResponse),
        (status = 404, description = "Invoice not found", body = ErrorResponse)
    )
)]
pub(crate) async fn create_invoice_payment_intent(
    auth: AuthCustomer,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, ApiError> {
    let invoice: Invoice = sqlx::query_as(
        "SELECT * FROM invoices WHERE id = $1 AND customer_id = $2",
    )
    .bind(id)
    .bind(auth.0.sub)
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound("invoice not found".into()))?;

    if invoice.status != InvoiceStatus::Pending && invoice.status != InvoiceStatus::Overdue {
        return Err(ApiError::BadRequest(
            "Only pending or overdue invoices can be paid".into(),
        ));
    }

    let stripe_secret = state
        .config
        .stripe_secret_key
        .as_deref()
        .ok_or(ApiError::Internal("Stripe not configured".into()))?;

    let total_cents = (invoice.total * rust_decimal::Decimal::from(100))
        .round()
        .to_i64()
        .unwrap_or(0)
        .max(50);

    let description = format!("VaultScope Invoice {}", invoice.invoice_number);
    let invoice_id_str = invoice.id.to_string();
    let customer_id_str = auth.0.sub.to_string();

    let client = reqwest::Client::new();
    let resp = client
        .post("https://api.stripe.com/v1/payment_intents")
        .basic_auth(stripe_secret, Option::<&str>::None)
        .form(&[
            ("amount", total_cents.to_string()),
            ("currency", "eur".to_string()),
            ("automatic_payment_methods[enabled]", "true".to_string()),
            ("description", description),
            ("metadata[invoice_id]", invoice_id_str),
            ("metadata[invoice_number]", invoice.invoice_number.clone()),
            ("metadata[customer_id]", customer_id_str),
        ])
        .send()
        .await
        .map_err(|e| ApiError::Internal(format!("Stripe request failed: {e}")))?;

    if !resp.status().is_success() {
        let body = resp.text().await.unwrap_or_default();
        return Err(ApiError::Internal(format!("Stripe error: {body}")));
    }

    let body: Value = resp
        .json()
        .await
        .map_err(|e| ApiError::Internal(format!("Stripe parse error: {e}")))?;

    let client_secret = body["client_secret"]
        .as_str()
        .ok_or(ApiError::Internal("Missing client_secret from Stripe".into()))?;
    let payment_intent_id = body["id"]
        .as_str()
        .ok_or(ApiError::Internal("Missing id from Stripe".into()))?;

    sqlx::query(
        "UPDATE invoices SET stripe_payment_intent_id = $1, updated_at = now() WHERE id = $2",
    )
    .bind(payment_intent_id)
    .bind(invoice.id)
    .execute(&state.db)
    .await?;

    Ok(Json(json!({
        "client_secret": client_secret,
        "payment_intent_id": payment_intent_id,
        "amount": invoice.total,
        "invoice_number": invoice.invoice_number,
    })))
}

#[derive(Deserialize, ToSchema)]
pub(crate) struct PayInvoicePayload {
    payment_intent_id: String,
}

#[utoipa::path(
    post,
    path = "/{id}/pay",
    tag = "Storefront Invoices",
    security(("bearer_auth" = []), ("csrf_token" = [])),
    params(("id" = Uuid, Path, description = "Invoice ID")),
    request_body = PayInvoicePayload,
    responses(
        (status = 200, description = "Verified payment and marked invoice paid", body = Invoice),
        (status = 400, description = "Payment verification failed or invalid invoice state", body = ErrorResponse),
        (status = 404, description = "Invoice not found", body = ErrorResponse)
    )
)]
pub(crate) async fn pay_invoice(
    auth: AuthCustomer,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(payload): Json<PayInvoicePayload>,
) -> Result<Json<Invoice>, ApiError> {
    let pi_id = payload.payment_intent_id.trim();
    if pi_id.is_empty() {
        return Err(ApiError::BadRequest("payment_intent_id is required".into()));
    }

    let invoice: Invoice = sqlx::query_as(
        "SELECT * FROM invoices WHERE id = $1 AND customer_id = $2",
    )
    .bind(id)
    .bind(auth.0.sub)
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound("invoice not found".into()))?;

    if invoice.status == InvoiceStatus::Paid {
        return Ok(Json(invoice));
    }

    if invoice.status != InvoiceStatus::Pending && invoice.status != InvoiceStatus::Overdue {
        return Err(ApiError::BadRequest(
            "Only pending or overdue invoices can be paid".into(),
        ));
    }

    let stripe_secret = state
        .config
        .stripe_secret_key
        .as_deref()
        .ok_or(ApiError::Internal("Stripe not configured".into()))?;

    let expected_cents = (invoice.total * rust_decimal::Decimal::from(100))
        .round()
        .to_i64()
        .unwrap_or(0)
        .max(50);

    let client = reqwest::Client::new();
    let resp = client
        .get(format!("https://api.stripe.com/v1/payment_intents/{}", pi_id))
        .basic_auth(stripe_secret, Option::<&str>::None)
        .send()
        .await
        .map_err(|e| ApiError::Internal(format!("Stripe verification failed: {e}")))?;

    if !resp.status().is_success() {
        let err_text = resp.text().await.unwrap_or_default();
        return Err(ApiError::BadRequest(format!("Invalid payment intent: {err_text}")));
    }

    let pi_data: Value = resp
        .json()
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to parse Stripe response: {e}")))?;

    let status = pi_data["status"].as_str().unwrap_or("");
    if status != "succeeded" {
        return Err(ApiError::BadRequest(format!(
            "Payment has not succeeded yet (status: {status})"
        )));
    }

    if let Some(meta_inv_id) = pi_data["metadata"]["invoice_id"].as_str() {
        if meta_inv_id != invoice.id.to_string() {
            return Err(ApiError::BadRequest(
                "Payment intent does not belong to this invoice".into(),
            ));
        }
    }

    let amount_received = pi_data["amount_received"].as_i64().unwrap_or(0);
    if amount_received < expected_cents {
        return Err(ApiError::BadRequest(format!(
            "Payment amount insufficient (received {amount_received}, expected at least {expected_cents})"
        )));
    }

    let updated: Invoice = sqlx::query_as(
        r#"UPDATE invoices
           SET status = 'paid'::invoice_status,
               paid_at = now(),
               stripe_payment_intent_id = $1,
               updated_at = now()
           WHERE id = $2 AND customer_id = $3
           RETURNING *"#,
    )
    .bind(pi_id)
    .bind(invoice.id)
    .bind(auth.0.sub)
    .fetch_one(&state.db)
    .await?;

    crate::activity::spawn_record(
        state.db.clone(),
        Some(auth.0.sub),
        vaultscope_db::models::ActorType::Customer,
        auth.0.email.clone(),
        "billing".to_string(),
        "invoice_paid".to_string(),
        updated.invoice_number.clone(),
        format!(
            "Invoice {} (€{:.2}) paid via Stripe",
            updated.invoice_number, updated.total
        ),
    );

    crate::mailer::spawn_invoice_paid_notification(state.clone(), updated.id);

    Ok(Json(updated))
}

#[utoipa::path(
    get,
    path = "/{id}/pdf",
    tag = "Storefront Invoices",
    security(("bearer_auth" = [])),
    params(("id" = Uuid, Path, description = "Invoice ID")),
    responses(
        (status = 200, description = "Generated invoice PDF", content_type = "application/pdf", body = Vec<u8>),
        (status = 404, description = "Invoice not found", body = ErrorResponse)
    )
)]
pub(crate) async fn download_pdf(
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
