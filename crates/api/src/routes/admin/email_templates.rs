use axum::extract::{Path, State};
use axum::routing::{get, post, put};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use uuid::Uuid;

use crate::auth::AuthAdmin;
use crate::error::ApiError;
use crate::state::AppState;
use vaultscope_db::models::EmailTemplate;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/", get(list))
        .route("/{id}", get(get_one))
        .route("/{id}", put(update))
        .route("/{id}/test", post(test_send))
}

async fn list(
    auth: AuthAdmin,
    State(state): State<AppState>,
) -> Result<Json<Vec<EmailTemplate>>, ApiError> {
    crate::require_permission!(auth, state, "templates.manage");
    let templates: Vec<EmailTemplate> =
        sqlx::query_as("SELECT * FROM email_templates ORDER BY category, name")
            .fetch_all(&state.db)
            .await?;
    Ok(Json(templates))
}

async fn get_one(
    auth: AuthAdmin,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<EmailTemplate>, ApiError> {
    crate::require_permission!(auth, state, "templates.manage");
    let template: EmailTemplate = sqlx::query_as("SELECT * FROM email_templates WHERE id = $1")
        .bind(id)
        .fetch_optional(&state.db)
        .await?
        .ok_or(ApiError::NotFound("template not found".into()))?;
    Ok(Json(template))
}

#[derive(Deserialize)]
struct UpdateTemplate {
    subject: Option<String>,
    body: Option<String>,
    enabled: Option<bool>,
    mailbox_id: Option<Uuid>,
}

async fn update(
    auth: AuthAdmin,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(payload): Json<UpdateTemplate>,
) -> Result<Json<Value>, ApiError> {
    crate::require_permission!(auth, state, "templates.manage");
    if let Some(subject) = &payload.subject {
        sqlx::query("UPDATE email_templates SET subject = $1, updated_at = now() WHERE id = $2")
            .bind(subject)
            .bind(id)
            .execute(&state.db)
            .await?;
    }
    if let Some(body) = &payload.body {
        sqlx::query("UPDATE email_templates SET body = $1, updated_at = now() WHERE id = $2")
            .bind(body)
            .bind(id)
            .execute(&state.db)
            .await?;
    }
    if let Some(enabled) = payload.enabled {
        sqlx::query("UPDATE email_templates SET enabled = $1, updated_at = now() WHERE id = $2")
            .bind(enabled)
            .bind(id)
            .execute(&state.db)
            .await?;
    }
    if let Some(mailbox_id) = payload.mailbox_id {
        sqlx::query("UPDATE email_templates SET mailbox_id = $1, updated_at = now() WHERE id = $2")
            .bind(mailbox_id)
            .bind(id)
            .execute(&state.db)
            .await?;
    }
    Ok(Json(json!({ "ok": true })))
}

#[derive(Deserialize)]
struct TestSendPayload {
    recipient: String,
}

#[derive(Serialize)]
struct TestSendResponse {
    ok: bool,
    recipient: String,
    rendered_subject: String,
    rendered_body: String,
}

async fn test_send(
    auth: AuthAdmin,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(payload): Json<TestSendPayload>,
) -> Result<Json<TestSendResponse>, ApiError> {
    crate::require_permission!(auth, state, "templates.manage");
    let template: EmailTemplate = sqlx::query_as("SELECT * FROM email_templates WHERE id = $1")
        .bind(id)
        .fetch_optional(&state.db)
        .await?
        .ok_or(ApiError::NotFound("template not found".into()))?;

    let mut rendered_subject = template.subject.clone();
    let mut rendered_body = template.body.clone();

    let replacements = [
        ("{{company_name}}", "VaultScope"),
        ("{{company_address}}", "Technologiepark 1, 10115 Berlin"),
        ("{{company_email}}", "support@vaultscope.de"),
        ("{{company_phone}}", "+49 30 12345678"),
        ("{{tos_url}}", "https://vaultscope.de/terms"),
        ("{{privacy_url}}", "https://vaultscope.de/privacy"),
        ("{{current_year}}", "2026"),
        ("{{customer_name}}", "Test Customer"),
        ("{{customer_email}}", &payload.recipient),
        ("{{invoice_number}}", "INV-2026-TEST"),
        ("{{total}}", "49.99"),
        ("{{due_date}}", "2026-11-01"),
        ("{{ticket_number}}", "TKT-TEST-1234"),
        ("{{ticket_subject}}", "Test ticket issue"),
        ("{{service_name}}", "Hetzner Dedicated CX41"),
        ("{{verification_url}}", "http://localhost:5173/verify?token=demo_token"),
    ];

    for (var, val) in replacements {
        rendered_subject = rendered_subject.replace(var, val);
        rendered_body = rendered_body.replace(var, val);
    }

    tracing::info!(
        "[EMAIL TEMPLATE TEST] Dispatch to: {}, Subject: {}\nBody:\n{}",
        payload.recipient,
        rendered_subject,
        rendered_body
    );

    Ok(Json(TestSendResponse {
        ok: true,
        recipient: payload.recipient,
        rendered_subject,
        rendered_body,
    }))
}
