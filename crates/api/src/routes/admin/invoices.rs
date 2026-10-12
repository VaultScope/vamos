use axum::body::Body;
use axum::extract::{Path, Query, State};
use axum::http::header;
use axum::response::Response;
use axum::routing::get;
use axum::{Json, Router};
use chrono::NaiveDate;
use serde::Deserialize;
use uuid::Uuid;

use crate::auth::AuthAdmin;
use crate::error::ApiError;
use crate::state::AppState;
use crate::validation::{
    validate_greater_than_zero, validate_percentage, validate_positive, validate_required_string,
    validate_string_length, MAX_SHORT_STRING_LENGTH, MAX_STRING_LENGTH,
};
use vaultscope_db::models::{Customer, Invoice, InvoiceLineItem, InvoiceStatus};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/stats/comprehensive", get(get_comprehensive_stats))
        .route("/stats", get(get_stats))
        .route("/", get(list).post(create))
        .route("/{id}", get(get_one).put(update).delete(delete))
        .route("/{id}/pdf", get(download_pdf))
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
    auth: AuthAdmin,
    State(state): State<AppState>,
    Query(params): Query<ListParams>,
) -> Result<Json<Vec<Invoice>>, ApiError> {
    crate::require_permission!(auth, state, "invoices.view");
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
    auth: AuthAdmin,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<Invoice>, ApiError> {
    crate::require_permission!(auth, state, "invoices.view");
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
    auth: AuthAdmin,
    State(state): State<AppState>,
    Json(payload): Json<CreateInvoice>,
) -> Result<Json<Invoice>, ApiError> {
    crate::require_permission!(auth, state, "invoices.create");
    // Validate inputs
    if payload.line_items.is_empty() {
        return Err(ApiError::BadRequest(
            "At least one line item is required".into(),
        ));
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

    let invoice_number = format!(
        "INV-{}",
        Uuid::new_v7(uuid::Timestamp::now(uuid::NoContext)).to_string()[..8].to_uppercase()
    );

    let subtotal: f64 = payload
        .line_items
        .iter()
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
    auth: AuthAdmin,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(payload): Json<UpdateInvoice>,
) -> Result<Json<Invoice>, ApiError> {
    crate::require_permission!(auth, state, "invoices.edit");
    let invoice: Invoice = sqlx::query_as(
        r#"UPDATE invoices
           SET status = COALESCE($1::invoice_status, status),
               due_date = COALESCE($2, due_date),
               notes = COALESCE($3, notes),
               updated_at = now()
           WHERE id = $4
           RETURNING *"#,
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
    auth: AuthAdmin,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<()>, ApiError> {
    crate::require_permission!(auth, state, "invoices.delete");
    let result =
        sqlx::query("DELETE FROM invoices WHERE id = $1 AND status = 'draft'::invoice_status")
            .bind(id)
            .execute(&state.db)
            .await?;

    if result.rows_affected() == 0 {
        return Err(ApiError::NotFound("draft invoice not found".into()));
    }

    Ok(Json(()))
}

async fn send(
    auth: AuthAdmin,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<Invoice>, ApiError> {
    crate::require_permission!(auth, state, "invoices.edit");
    let invoice: Invoice = sqlx::query_as(
        r#"UPDATE invoices
           SET status = 'pending'::invoice_status,
               updated_at = now()
           WHERE id = $1 AND status = 'draft'::invoice_status
           RETURNING *"#,
    )
    .bind(id)
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound("draft invoice not found".into()))?;

    let _ = dispatch_invoice_email(&state, &invoice).await;

    Ok(Json(invoice))
}

async fn dispatch_invoice_email(
    state: &AppState,
    invoice: &Invoice,
) -> Result<(), ApiError> {
    let customer: Option<(String, String)> = sqlx::query_as(
        "SELECT name, email FROM customers WHERE id = $1"
    )
    .bind(invoice.customer_id)
    .fetch_optional(&state.db)
    .await?;

    let (customer_name, customer_email) = match customer {
        Some(c) => c,
        None => return Ok(()),
    };

    // Security: Only include verified billing emails in CC
    let cc_emails: Vec<String> = sqlx::query_scalar(
        "SELECT email FROM customer_billing_emails WHERE customer_id = $1 AND verified = true"
    )
    .bind(invoice.customer_id)
    .fetch_all(&state.db)
    .await
    .unwrap_or_default();

    let smtp_host = state.config.smtp_host.clone().unwrap_or_else(|| "mail.vaultscope.de".to_string());
    let smtp_user = state.config.smtp_username.clone().unwrap_or_else(|| "system@vaultscope.de".to_string());
    let smtp_pass = state.config.smtp_password.clone().unwrap_or_else(|| "".to_string());
    let smtp_from = state.config.smtp_from.clone().unwrap_or_else(|| "system@vaultscope.de".to_string());

    let subject = format!("Invoice {} from VaultScope", invoice.invoice_number);
    let body = format!(
"Hello {},

A new invoice has been generated for your account.

Invoice Number: {}
Total Amount: €{}
Due Date: {}

You can view and manage your invoice directly in the VaultScope customer dashboard:
http://localhost:5173/dashboard/billing

Best regards,
The VaultScope Billing Team",
        customer_name, invoice.invoice_number, invoice.total, invoice.due_date
    );

    let from_m = match smtp_from.parse() {
        Ok(m) => m,
        Err(_) => return Ok(()),
    };
    let to_m = match customer_email.parse() {
        Ok(m) => m,
        Err(_) => return Ok(()),
    };

    let mut builder = lettre::Message::builder()
        .from(from_m)
        .to(to_m)
        .subject(subject);

    for cc_addr in &cc_emails {
        if let Ok(mailbox) = cc_addr.parse() {
            builder = builder.cc(mailbox);
        }
    }

    let email = match builder.body(body) {
        Ok(m) => m,
        Err(e) => {
            tracing::error!("Failed to build invoice email: {:?}", e);
            return Ok(());
        }
    };

    let creds = lettre::transport::smtp::authentication::Credentials::new(smtp_user, smtp_pass);
    let mailer = match lettre::SmtpTransport::relay(&smtp_host) {
        Ok(b) => b.credentials(creds).build(),
        Err(e) => {
            tracing::error!("Failed to build SMTP relay: {:?}", e);
            return Ok(());
        }
    };

    if let Err(e) = lettre::Transport::send(&mailer, &email) {
        tracing::error!("Could not send invoice email (primary: {}, CCs: {:?}): {:?}", customer_email, cc_emails, e);
    } else {
        tracing::info!("Invoice email sent successfully to {} with CCs: {:?}", customer_email, cc_emails);
    }

    Ok(())
}

async fn mark_paid(
    auth: AuthAdmin,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<Invoice>, ApiError> {
    crate::require_permission!(auth, state, "invoices.edit");
    let invoice: Invoice = sqlx::query_as(
        r#"UPDATE invoices
           SET status = 'paid'::invoice_status,
               paid_at = now(),
               updated_at = now()
           WHERE id = $1
           RETURNING *"#,
    )
    .bind(id)
    .fetch_one(&state.db)
    .await?;

    Ok(Json(invoice))
}

async fn list_line_items(
    auth: AuthAdmin,
    State(state): State<AppState>,
    Path(invoice_id): Path<Uuid>,
) -> Result<Json<Vec<InvoiceLineItem>>, ApiError> {
    crate::require_permission!(auth, state, "invoices.view");
    let items: Vec<InvoiceLineItem> = sqlx::query_as(
        "SELECT * FROM invoice_line_items WHERE invoice_id = $1 ORDER BY sort_order",
    )
    .bind(invoice_id)
    .fetch_all(&state.db)
    .await?;

    Ok(Json(items))
}

#[derive(serde::Serialize)]
pub struct InvoiceStats {
    pub unpaid_total: rust_decimal::Decimal,
    pub unpaid_count: i64,
}

async fn get_stats(
    auth: AuthAdmin,
    State(state): State<AppState>,
) -> Result<Json<InvoiceStats>, ApiError> {
    crate::require_permission!(auth, state, "invoices.view");
    let row: (Option<rust_decimal::Decimal>, Option<i64>) = sqlx::query_as(
        "SELECT SUM(total), COUNT(*) FROM invoices WHERE status = 'pending'::invoice_status OR status = 'overdue'::invoice_status"
    )
    .fetch_one(&state.db)
    .await?;

    Ok(Json(InvoiceStats {
        unpaid_total: row.0.unwrap_or_default(),
        unpaid_count: row.1.unwrap_or(0),
    }))
}

#[derive(serde::Serialize)]
pub struct ComprehensiveStats {
    pub unpaid_invoices: i64,
    pub paid_invoices: i64,
    pub revenue: rust_decimal::Decimal,
    pub profit: rust_decimal::Decimal,
    pub stripe_balance: rust_decimal::Decimal,
    pub revenue_per_month: Vec<MonthlyRevenue>,
    pub infrastructure_cost: rust_decimal::Decimal,
    pub top_clients: Vec<ClientSpent>,
    pub coupons_loss: rust_decimal::Decimal,
    pub top_products: Vec<ProductSales>,
}

#[derive(serde::Serialize)]
pub struct MonthlyRevenue {
    pub month: String,
    pub revenue: rust_decimal::Decimal,
}

#[derive(serde::Serialize)]
pub struct ClientSpent {
    pub customer_id: uuid::Uuid,
    pub name: String,
    pub total_spent: rust_decimal::Decimal,
}

#[derive(serde::Serialize)]
pub struct ProductSales {
    pub name: String,
    pub count: i64,
}

async fn get_comprehensive_stats(
    auth: AuthAdmin,
    State(state): State<AppState>,
) -> Result<Json<ComprehensiveStats>, ApiError> {
    crate::require_permission!(auth, state, "invoices.view");
    // 1. Unpaid vs Paid Invoices
    let unpaid: (Option<i64>,) =
        sqlx::query_as("SELECT count(*) FROM invoices WHERE status IN ('pending', 'overdue')")
            .fetch_one(&state.db)
            .await
            .unwrap_or((Some(0),));
    let paid: (Option<i64>,) =
        sqlx::query_as("SELECT count(*) FROM invoices WHERE status = 'paid'")
            .fetch_one(&state.db)
            .await
            .unwrap_or((Some(0),));

    // 2. Revenue vs Profit & 5. Infrastructure Cost
    // Infrastructure cost = sum of products.cost for active services
    let cost_row: (Option<rust_decimal::Decimal>,) = sqlx::query_as(
        "SELECT SUM(p.cost) FROM services s JOIN products p ON s.product_id = p.id WHERE s.status = 'running'"
    )
    .fetch_one(&state.db)
    .await
    .unwrap_or((None,));
    let infrastructure_cost = cost_row.0.unwrap_or_default();

    let revenue_row: (Option<rust_decimal::Decimal>,) =
        sqlx::query_as("SELECT SUM(total) FROM invoices WHERE status = 'paid'")
            .fetch_one(&state.db)
            .await
            .unwrap_or((None,));
    let revenue = revenue_row.0.unwrap_or_default();
    let profit = revenue - infrastructure_cost;

    // 3. Stripe Balance
    let mut stripe_balance = rust_decimal::Decimal::from(0);
    if let Ok(Some((key_val,))) = sqlx::query_as::<_, (serde_json::Value,)>(
        "SELECT value FROM settings WHERE key = 'stripe_secret_key'",
    )
    .fetch_optional(&state.db)
    .await
    {
        if let Some(key) = key_val.as_str() {
            let client = reqwest::Client::new();
            if let Ok(resp) = client
                .get("https://api.stripe.com/v1/balance")
                .bearer_auth(key)
                .send()
                .await
            {
                if let Ok(json) = resp.json::<serde_json::Value>().await {
                    if let Some(available) = json.get("available").and_then(|a| a.as_array()) {
                        if let Some(first) = available.first() {
                            if let Some(amount) = first.get("amount").and_then(|a| a.as_i64()) {
                                stripe_balance = rust_decimal::Decimal::new(amount, 2);
                            }
                        }
                    }
                }
            }
        }
    }

    // 4. Revenue per month (aggregated over the last 6-12 months)
    let rev_months_query = sqlx::query_as::<_, (Option<String>, Option<rust_decimal::Decimal>)>(
        "SELECT to_char(date_trunc('month', paid_at), 'YYYY-MM') as month, SUM(total) as revenue 
         FROM invoices 
         WHERE status = 'paid' AND paid_at >= now() - interval '6 months' 
         GROUP BY month 
         ORDER BY month ASC",
    )
    .fetch_all(&state.db)
    .await
    .unwrap_or_default();

    let mut revenue_per_month = Vec::new();
    for row in rev_months_query {
        if let (Some(m), Some(r)) = row {
            revenue_per_month.push(MonthlyRevenue {
                month: m,
                revenue: r,
            });
        }
    }

    // 6. Spent per Client (top clients by total paid)
    let top_clients_query =
        sqlx::query_as::<_, (uuid::Uuid, String, Option<rust_decimal::Decimal>)>(
            "SELECT c.id, c.name, SUM(i.total) as total_spent 
         FROM customers c 
         JOIN invoices i ON c.id = i.customer_id 
         WHERE i.status = 'paid' 
         GROUP BY c.id, c.name 
         ORDER BY total_spent DESC 
         LIMIT 5",
        )
        .fetch_all(&state.db)
        .await
        .unwrap_or_default();

    let top_clients = top_clients_query
        .into_iter()
        .map(|c| ClientSpent {
            customer_id: c.0,
            name: c.1,
            total_spent: c.2.unwrap_or_default(),
        })
        .collect();

    // 7. Coupons used and loss
    let coupons_loss_row: (Option<rust_decimal::Decimal>,) =
        sqlx::query_as("SELECT SUM(usage_count * discount_value) FROM coupons")
            .fetch_one(&state.db)
            .await
            .unwrap_or((None,));
    let coupons_loss = coupons_loss_row.0.unwrap_or_default();

    // 8. Top products with sales (count of active services per product)
    let top_products_query = sqlx::query_as::<_, (String, Option<i64>)>(
        "SELECT p.name, COUNT(s.id) as cnt 
         FROM products p 
         JOIN services s ON p.id = s.product_id 
         WHERE s.status = 'running' 
         GROUP BY p.id, p.name 
         ORDER BY cnt DESC 
         LIMIT 5",
    )
    .fetch_all(&state.db)
    .await
    .unwrap_or_default();

    let top_products = top_products_query
        .into_iter()
        .map(|p| ProductSales {
            name: p.0,
            count: p.1.unwrap_or(0),
        })
        .collect();

    Ok(Json(ComprehensiveStats {
        unpaid_invoices: unpaid.0.unwrap_or(0),
        paid_invoices: paid.0.unwrap_or(0),
        revenue,
        profit,
        stripe_balance,
        revenue_per_month,
        infrastructure_cost,
        top_clients,
        coupons_loss,
        top_products,
    }))
}

async fn download_pdf(
    auth: AuthAdmin,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Response, ApiError> {
    crate::require_permission!(auth, state, "invoices.view");
    let invoice: Invoice = sqlx::query_as("SELECT * FROM invoices WHERE id = $1")
        .bind(id)
        .fetch_optional(&state.db)
        .await?
        .ok_or(ApiError::NotFound("invoice not found".into()))?;

    let customer: Customer = sqlx::query_as("SELECT * FROM customers WHERE id = $1")
        .bind(invoice.customer_id)
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
