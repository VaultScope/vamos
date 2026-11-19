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

use super::orders::{is_unique_violation, stripe_http_client, validate_stripe_payment_intent_id};
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

/// Verifies a Stripe PaymentIntent JSON response against an invoice's expected
/// currency, metadata (`invoice_id`, `customer_id`, optional `purpose`), and amount.
pub fn verify_invoice_stripe_pi_response(
    pi_data: &Value,
    expected_invoice_id: Uuid,
    expected_customer_id: Uuid,
    expected_min_cents: i64,
) -> Result<(), ApiError> {
    let status = pi_data["status"].as_str().unwrap_or("");
    if status != "succeeded" {
        return Err(ApiError::BadRequest(format!(
            "Payment has not succeeded yet (status: {status})"
        )));
    }

    let currency_ok = pi_data["currency"]
        .as_str()
        .is_some_and(|c| c.eq_ignore_ascii_case("eur"));
    if !currency_ok {
        return Err(ApiError::BadRequest(
            "Payment currency must be EUR".into(),
        ));
    }

    if let Some(purpose) = pi_data["metadata"]["purpose"].as_str() {
        if purpose != "invoice" {
            return Err(ApiError::BadRequest(
                "Payment intent is not valid for invoice payments".into(),
            ));
        }
    }

    let expected_inv_str = expected_invoice_id.to_string();
    if pi_data["metadata"]["invoice_id"].as_str() != Some(expected_inv_str.as_str()) {
        return Err(ApiError::BadRequest(
            "Payment intent does not belong to this invoice".into(),
        ));
    }

    let expected_cust_str = expected_customer_id.to_string();
    if pi_data["metadata"]["customer_id"].as_str() != Some(expected_cust_str.as_str()) {
        return Err(ApiError::BadRequest(
            "Payment intent does not belong to this customer".into(),
        ));
    }

    let amount_received = pi_data["amount_received"].as_i64().unwrap_or(0);
    if amount_received < expected_min_cents {
        return Err(ApiError::BadRequest(format!(
            "Payment amount insufficient (received {amount_received}, expected at least {expected_min_cents})"
        )));
    }

    Ok(())
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

    let client = stripe_http_client()?;
    let resp = client
        .post("https://api.stripe.com/v1/payment_intents")
        .basic_auth(stripe_secret, Option::<&str>::None)
        .form(&[
            ("amount", total_cents.to_string()),
            ("currency", "eur".to_string()),
            ("automatic_payment_methods[enabled]", "true".to_string()),
            ("description", description),
            ("metadata[purpose]", "invoice".to_string()),
            ("metadata[invoice_id]", invoice_id_str),
            ("metadata[invoice_number]", invoice.invoice_number.clone()),
            ("metadata[customer_id]", customer_id_str),
        ])
        .send()
        .await
        .map_err(|e| {
            tracing::error!(invoice_id = %invoice.id, "Stripe create payment intent request failed: {e}");
            ApiError::Internal("Payment provider unavailable".into())
        })?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        tracing::error!(invoice_id = %invoice.id, %status, stripe_body = %body, "Stripe create invoice payment intent error");
        return Err(ApiError::Internal("Failed to initialize payment".into()));
    }

    let body: Value = resp.json().await.map_err(|e| {
        tracing::error!(invoice_id = %invoice.id, "Failed to parse Stripe invoice payment intent response: {e}");
        ApiError::Internal("Invalid response from payment provider".into())
    })?;

    let client_secret = body["client_secret"].as_str().ok_or_else(|| {
        tracing::error!(invoice_id = %invoice.id, "Missing client_secret in Stripe invoice payment intent response");
        ApiError::Internal("Invalid response from payment provider".into())
    })?;
    let payment_intent_id = body["id"].as_str().ok_or_else(|| {
        tracing::error!(invoice_id = %invoice.id, "Missing id in Stripe invoice payment intent response");
        ApiError::Internal("Invalid response from payment provider".into())
    })?;

    let update_res = sqlx::query(
        "UPDATE invoices SET stripe_payment_intent_id = $1, updated_at = now() WHERE id = $2 AND customer_id = $3 AND status IN ('pending'::invoice_status, 'overdue'::invoice_status)",
    )
    .bind(payment_intent_id)
    .bind(invoice.id)
    .bind(auth.0.sub)
    .execute(&state.db)
    .await
    .map_err(|e| {
        if is_unique_violation(&e) {
            ApiError::BadRequest("This payment intent has already been used".into())
        } else {
            ApiError::from(e)
        }
    })?;

    if update_res.rows_affected() != 1 {
        return Err(ApiError::BadRequest(
            "Only pending or overdue invoices can be paid".into(),
        ));
    }

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
    let pi_id = validate_stripe_payment_intent_id(&payload.payment_intent_id)?;

    let invoice: Invoice = sqlx::query_as(
        "SELECT * FROM invoices WHERE id = $1 AND customer_id = $2",
    )
    .bind(id)
    .bind(auth.0.sub)
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound("invoice not found".into()))?;

    if invoice.status == InvoiceStatus::Paid {
        if invoice.stripe_payment_intent_id.as_deref() == Some(pi_id) {
            return Ok(Json(invoice));
        }
        return Err(ApiError::BadRequest("Invoice is already paid".into()));
    }

    if invoice.status != InvoiceStatus::Pending && invoice.status != InvoiceStatus::Overdue {
        return Err(ApiError::BadRequest(
            "Only pending or overdue invoices can be paid".into(),
        ));
    }

    // Fast-fail replay check across both services and other invoices
    let used_in_services: Option<Uuid> =
        sqlx::query_scalar("SELECT id FROM services WHERE stripe_payment_intent_id = $1")
            .bind(pi_id)
            .fetch_optional(&state.db)
            .await?;
    if used_in_services.is_some() {
        return Err(ApiError::BadRequest(
            "This payment intent has already been used".into(),
        ));
    }

    let used_in_other_invoice: Option<Uuid> = sqlx::query_scalar(
        "SELECT id FROM invoices WHERE stripe_payment_intent_id = $1 AND id != $2",
    )
    .bind(pi_id)
    .bind(invoice.id)
    .fetch_optional(&state.db)
    .await?;
    if used_in_other_invoice.is_some() {
        return Err(ApiError::BadRequest(
            "This payment intent has already been used".into(),
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

    let client = stripe_http_client()?;
    let resp = client
        .get(format!("https://api.stripe.com/v1/payment_intents/{}", pi_id))
        .basic_auth(stripe_secret, Option::<&str>::None)
        .send()
        .await
        .map_err(|e| {
            tracing::error!(invoice_id = %invoice.id, pi_id = %pi_id, "Stripe verification request failed: {e}");
            ApiError::Internal("Payment verification failed".into())
        })?;

    if !resp.status().is_success() {
        let status = resp.status();
        let err_text = resp.text().await.unwrap_or_default();
        tracing::error!(invoice_id = %invoice.id, pi_id = %pi_id, %status, stripe_body = %err_text, "Stripe verification error");
        return Err(ApiError::BadRequest(
            "Invalid or unverified payment intent".into(),
        ));
    }

    let pi_data: Value = resp.json().await.map_err(|e| {
        tracing::error!(invoice_id = %invoice.id, pi_id = %pi_id, "Failed to parse Stripe verification response: {e}");
        ApiError::Internal("Invalid response from payment provider".into())
    })?;

    verify_invoice_stripe_pi_response(&pi_data, invoice.id, auth.0.sub, expected_cents)?;

    let mut tx = state.db.begin().await?;

    let locked_invoice: Invoice = sqlx::query_as(
        "SELECT * FROM invoices WHERE id = $1 AND customer_id = $2 FOR UPDATE",
    )
    .bind(invoice.id)
    .bind(auth.0.sub)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(ApiError::NotFound("invoice not found".into()))?;

    if locked_invoice.status == InvoiceStatus::Paid {
        if locked_invoice.stripe_payment_intent_id.as_deref() == Some(pi_id) {
            tx.commit().await?;
            return Ok(Json(locked_invoice));
        }
        return Err(ApiError::BadRequest("Invoice is already paid".into()));
    }

    if locked_invoice.status != InvoiceStatus::Pending
        && locked_invoice.status != InvoiceStatus::Overdue
    {
        return Err(ApiError::BadRequest(
            "Only pending or overdue invoices can be paid".into(),
        ));
    }

    let used_in_services_tx: Option<Uuid> =
        sqlx::query_scalar("SELECT id FROM services WHERE stripe_payment_intent_id = $1")
            .bind(pi_id)
            .fetch_optional(&mut *tx)
            .await?;
    if used_in_services_tx.is_some() {
        return Err(ApiError::BadRequest(
            "This payment intent has already been used".into(),
        ));
    }

    let used_in_other_invoice_tx: Option<Uuid> = sqlx::query_scalar(
        "SELECT id FROM invoices WHERE stripe_payment_intent_id = $1 AND id != $2",
    )
    .bind(pi_id)
    .bind(locked_invoice.id)
    .fetch_optional(&mut *tx)
    .await?;
    if used_in_other_invoice_tx.is_some() {
        return Err(ApiError::BadRequest(
            "This payment intent has already been used".into(),
        ));
    }

    let updated: Invoice = sqlx::query_as(
        r#"UPDATE invoices
           SET status = 'paid'::invoice_status,
               paid_at = now(),
               stripe_payment_intent_id = $1,
               updated_at = now()
           WHERE id = $2
             AND customer_id = $3
             AND status IN ('pending'::invoice_status, 'overdue'::invoice_status)
           RETURNING *"#,
    )
    .bind(pi_id)
    .bind(locked_invoice.id)
    .bind(auth.0.sub)
    .fetch_one(&mut *tx)
    .await
    .map_err(|e| {
        if is_unique_violation(&e) {
            ApiError::BadRequest("This payment intent has already been used".into())
        } else {
            ApiError::from(e)
        }
    })?;

    tx.commit().await?;

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
