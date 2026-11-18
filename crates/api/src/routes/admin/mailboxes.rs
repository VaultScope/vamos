use axum::extract::{Path, State};
use axum::Json;
use lettre::transport::smtp::authentication::Credentials;
use lettre::{AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor};
use serde::Deserialize;
use serde_json::{json, Value};
use utoipa::ToSchema;
use utoipa_axum::{router::OpenApiRouter, routes};
use uuid::Uuid;

use crate::auth::AuthAdmin;
use crate::error::ApiError;
use crate::openapi::{ErrorResponse, OkMessageResponse, OkResponse};
use crate::state::AppState;
use vaultscope_db::models::Mailbox;

pub fn routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(list, create))
        .routes(routes!(test_smtp_credentials))
        .routes(routes!(get_one, update, delete_mailbox))
        .routes(routes!(test_mailbox))
        .routes(routes!(set_default))
}

#[utoipa::path(
    get,
    path = "/",
    tag = "Admin Mailboxes",
    security(("bearer_auth" = [])),
    responses((status = 200, description = "List mailboxes", body = Vec<Mailbox>))
)]
pub(crate) async fn list(
    auth: AuthAdmin,
    State(state): State<AppState>,
) -> Result<Json<Vec<Mailbox>>, ApiError> {
    crate::require_permission!(auth, state, "mail.manage");
    let mailboxes: Vec<Mailbox> = sqlx::query_as("SELECT * FROM mailboxes ORDER BY name")
        .fetch_all(&state.db)
        .await?;
    Ok(Json(mailboxes))
}

#[utoipa::path(
    get,
    path = "/{id}",
    tag = "Admin Mailboxes",
    security(("bearer_auth" = [])),
    params(("id" = Uuid, Path, description = "Mailbox ID")),
    responses(
        (status = 200, description = "Mailbox details", body = Mailbox),
        (status = 404, description = "Mailbox not found", body = ErrorResponse)
    )
)]
pub(crate) async fn get_one(
    auth: AuthAdmin,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<Mailbox>, ApiError> {
    crate::require_permission!(auth, state, "mail.manage");
    let mailbox: Mailbox = sqlx::query_as("SELECT * FROM mailboxes WHERE id = $1")
        .bind(id)
        .fetch_optional(&state.db)
        .await?
        .ok_or(ApiError::NotFound("mailbox not found".into()))?;
    Ok(Json(mailbox))
}

#[derive(Deserialize, ToSchema)]
pub(crate) struct CreateMailbox {
    name: String,
    host: String,
    port: i32,
    user_name: String,
    password: String,
    is_default: Option<bool>,
}

#[utoipa::path(
    post,
    path = "/",
    tag = "Admin Mailboxes",
    security(("bearer_auth" = []), ("csrf_token" = [])),
    request_body = CreateMailbox,
    responses((status = 200, description = "Mailbox created", body = Mailbox))
)]
pub(crate) async fn create(
    auth: AuthAdmin,
    State(state): State<AppState>,
    Json(payload): Json<CreateMailbox>,
) -> Result<Json<Mailbox>, ApiError> {
    crate::require_permission!(auth, state, "mail.manage");
    let is_default = payload.is_default.unwrap_or(false);

    // If this is set to default, maybe we should unset other defaults, but simple implementation is fine for now
    let mailbox: Mailbox = sqlx::query_as(
        "INSERT INTO mailboxes (name, host, port, user_name, password, is_default)
         VALUES ($1, $2, $3, $4, $5, $6) RETURNING *",
    )
    .bind(payload.name)
    .bind(payload.host)
    .bind(payload.port)
    .bind(payload.user_name)
    .bind(payload.password)
    .bind(is_default)
    .fetch_one(&state.db)
    .await?;
    Ok(Json(mailbox))
}

#[derive(Deserialize, ToSchema)]
pub(crate) struct UpdateMailbox {
    name: Option<String>,
    host: Option<String>,
    port: Option<i32>,
    user_name: Option<String>,
    password: Option<String>,
    is_default: Option<bool>,
}

#[utoipa::path(
    put,
    path = "/{id}",
    tag = "Admin Mailboxes",
    security(("bearer_auth" = []), ("csrf_token" = [])),
    params(("id" = Uuid, Path, description = "Mailbox ID")),
    request_body = UpdateMailbox,
    responses((status = 200, description = "Mailbox updated", body = OkResponse))
)]
pub(crate) async fn update(
    auth: AuthAdmin,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(payload): Json<UpdateMailbox>,
) -> Result<Json<Value>, ApiError> {
    crate::require_permission!(auth, state, "mail.manage");

    // Simplistic update one by one like in email_templates
    if let Some(name) = &payload.name {
        sqlx::query("UPDATE mailboxes SET name = $1 WHERE id = $2")
            .bind(name)
            .bind(id)
            .execute(&state.db)
            .await?;
    }
    if let Some(host) = &payload.host {
        sqlx::query("UPDATE mailboxes SET host = $1 WHERE id = $2")
            .bind(host)
            .bind(id)
            .execute(&state.db)
            .await?;
    }
    if let Some(port) = payload.port {
        sqlx::query("UPDATE mailboxes SET port = $1 WHERE id = $2")
            .bind(port)
            .bind(id)
            .execute(&state.db)
            .await?;
    }
    if let Some(user_name) = &payload.user_name {
        sqlx::query("UPDATE mailboxes SET user_name = $1 WHERE id = $2")
            .bind(user_name)
            .bind(id)
            .execute(&state.db)
            .await?;
    }
    if let Some(password) = &payload.password {
        sqlx::query("UPDATE mailboxes SET password = $1 WHERE id = $2")
            .bind(password)
            .bind(id)
            .execute(&state.db)
            .await?;
    }
    if let Some(is_default) = payload.is_default {
        sqlx::query("UPDATE mailboxes SET is_default = $1 WHERE id = $2")
            .bind(is_default)
            .bind(id)
            .execute(&state.db)
            .await?;
    }

    Ok(Json(json!({ "ok": true })))
}

#[utoipa::path(
    delete,
    path = "/{id}",
    tag = "Admin Mailboxes",
    security(("bearer_auth" = []), ("csrf_token" = [])),
    params(("id" = Uuid, Path, description = "Mailbox ID")),
    responses(
        (status = 200, description = "Mailbox deleted", body = OkResponse),
        (status = 404, description = "Mailbox not found", body = ErrorResponse)
    )
)]
pub(crate) async fn delete_mailbox(
    auth: AuthAdmin,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, ApiError> {
    crate::require_permission!(auth, state, "mail.manage");
    let rows = sqlx::query("DELETE FROM mailboxes WHERE id = $1")
        .bind(id)
        .execute(&state.db)
        .await?;
    if rows.rows_affected() == 0 {
        return Err(ApiError::NotFound("mailbox not found".into()));
    }
    Ok(Json(json!({ "ok": true })))
}

#[derive(Deserialize, ToSchema)]
pub(crate) struct TestMailboxPayload {
    send_test_email: Option<String>,
}

#[utoipa::path(
    post,
    path = "/{id}/test",
    tag = "Admin Mailboxes",
    security(("bearer_auth" = []), ("csrf_token" = [])),
    params(("id" = Uuid, Path, description = "Mailbox ID")),
    request_body = TestMailboxPayload,
    responses(
        (status = 200, description = "Mailbox connection or test email result", body = OkMessageResponse),
        (status = 404, description = "Mailbox not found", body = ErrorResponse)
    )
)]
pub(crate) async fn test_mailbox(
    auth: AuthAdmin,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(payload): Json<TestMailboxPayload>,
) -> Result<Json<Value>, ApiError> {
    crate::require_permission!(auth, state, "mail.manage");
    let mailbox: Mailbox = sqlx::query_as("SELECT * FROM mailboxes WHERE id = $1")
        .bind(id)
        .fetch_optional(&state.db)
        .await?
        .ok_or(ApiError::NotFound("mailbox not found".into()))?;

    let creds = Credentials::new(mailbox.user_name.clone(), mailbox.password.clone());

    // We try to connect
    let mailer: AsyncSmtpTransport<Tokio1Executor> =
        AsyncSmtpTransport::<Tokio1Executor>::relay(&mailbox.host)
            .map_err(|e| ApiError::Internal(e.to_string()))?
            .port(mailbox.port as u16)
            .credentials(creds)
            .build();

    if let Some(recipient) = payload.send_test_email {
        let email = Message::builder()
            .from(
                mailbox
                    .user_name
                    .parse()
                    .map_err(|e: lettre::address::AddressError| {
                        ApiError::Internal(e.to_string())
                    })?,
            )
            .to(recipient
                .parse()
                .map_err(|e: lettre::address::AddressError| ApiError::Internal(e.to_string()))?)
            .subject("VaultScope Mailbox Test")
            .body(String::from(
                "This is a test email to verify mailbox settings.",
            ))
            .map_err(|e| ApiError::Internal(e.to_string()))?;

        mailer
            .send(email)
            .await
            .map_err(|e| ApiError::Internal(format!("Failed to send email: {}", e)))?;
        Ok(Json(
            json!({ "ok": true, "message": "Test email sent successfully" }),
        ))
    } else {
        let test_conn = mailer
            .test_connection()
            .await
            .map_err(|e| ApiError::Internal(format!("Connection failed: {}", e)))?;
        if !test_conn {
            return Err(ApiError::Internal(
                "Failed to connect to SMTP server".into(),
            ));
        }
        Ok(Json(
            json!({ "ok": true, "message": "Connection successful" }),
        ))
    }
}

#[derive(Deserialize, ToSchema)]
pub(crate) struct TestSmtpPayload {
    smtp_host: String,
    smtp_port: Option<i64>,
    smtp_user: String,
    smtp_pass: String,
    email: Option<String>,
    test_to: Option<String>,
}

#[utoipa::path(
    post,
    path = "/test",
    tag = "Admin Mailboxes",
    security(("bearer_auth" = []), ("csrf_token" = [])),
    request_body = TestSmtpPayload,
    responses((status = 200, description = "SMTP test result", body = OkMessageResponse))
)]
pub(crate) async fn test_smtp_credentials(
    auth: AuthAdmin,
    State(state): State<AppState>,
    Json(payload): Json<TestSmtpPayload>,
) -> Result<Json<Value>, ApiError> {
    crate::require_permission!(auth, state, "mail.manage");

    let port = payload.smtp_port.unwrap_or(587) as u16;
    let creds = Credentials::new(payload.smtp_user.clone(), payload.smtp_pass.clone());

    let mailer: AsyncSmtpTransport<Tokio1Executor> =
        AsyncSmtpTransport::<Tokio1Executor>::relay(&payload.smtp_host)
            .map_err(|e| ApiError::Internal(e.to_string()))?
            .port(port)
            .credentials(creds)
            .build();

    if let Some(recipient) = payload.test_to.filter(|s| !s.is_empty()) {
        let from_addr = payload.email.as_deref().unwrap_or(&payload.smtp_user);
        let email = Message::builder()
            .from(
                from_addr
                    .parse()
                    .map_err(|e: lettre::address::AddressError| {
                        ApiError::Internal(e.to_string())
                    })?,
            )
            .to(recipient
                .parse()
                .map_err(|e: lettre::address::AddressError| ApiError::Internal(e.to_string()))?)
            .subject("VaultScope Mailbox Test")
            .body(String::from(
                "This is a test email to verify your mailbox settings in VaultScope.",
            ))
            .map_err(|e| ApiError::Internal(e.to_string()))?;

        mailer
            .send(email)
            .await
            .map_err(|e| ApiError::Internal(format!("Failed to send test email: {}", e)))?;

        Ok(Json(
            json!({ "ok": true, "message": "Test email sent successfully" }),
        ))
    } else {
        let ok = mailer
            .test_connection()
            .await
            .map_err(|e| ApiError::Internal(format!("Connection failed: {}", e)))?;
        if ok {
            Ok(Json(
                json!({ "ok": true, "message": "SMTP connection successful" }),
            ))
        } else {
            Err(ApiError::Internal(
                "SMTP server rejected the connection".into(),
            ))
        }
    }
}

#[utoipa::path(
    put,
    path = "/{id}/default",
    tag = "Admin Mailboxes",
    security(("bearer_auth" = []), ("csrf_token" = [])),
    params(("id" = Uuid, Path, description = "Mailbox ID")),
    responses((status = 200, description = "Default mailbox updated", body = OkResponse))
)]
pub(crate) async fn set_default(
    auth: AuthAdmin,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, ApiError> {
    crate::require_permission!(auth, state, "mail.manage");
    // Unset all, then set this one
    sqlx::query("UPDATE mailboxes SET is_default = false")
        .execute(&state.db)
        .await?;
    sqlx::query("UPDATE mailboxes SET is_default = true WHERE id = $1")
        .bind(id)
        .execute(&state.db)
        .await?;
    Ok(Json(json!({ "ok": true })))
}
