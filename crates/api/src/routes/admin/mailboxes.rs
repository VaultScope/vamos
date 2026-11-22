use std::net::{IpAddr, Ipv4Addr};
use std::time::Duration;

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

/// Returns `true` if the IPv4 address belongs to loopback, private RFC1918,
/// link-local/metadata, unspecified, CGNAT, or other non-public ranges.
fn is_forbidden_smtp_v4(v4: Ipv4Addr) -> bool {
    let o = v4.octets();
    v4.is_loopback()
        || v4.is_private()
        || v4.is_link_local()
        || v4.is_unspecified()
        || o[0] == 0
        || v4.is_broadcast()
        || v4.is_multicast()
        || v4.is_documentation()
        // CGNAT 100.64.0.0/10
        || (o[0] == 100 && (64..=127).contains(&o[1]))
        // IETF Protocol Assignments 192.0.0.0/24
        || (o[0] == 192 && o[1] == 0 && o[2] == 0)
        // Benchmarking 198.18.0.0/15
        || (o[0] == 198 && (18..=19).contains(&o[1]))
        // Reserved 240.0.0.0/4
        || o[0] >= 240
}

/// Returns `true` if the IP address is non-public or disallowed for outbound SMTP connections.
pub fn is_forbidden_smtp_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => is_forbidden_smtp_v4(v4),
        IpAddr::V6(v6) => {
            if let Some(mapped_v4) = v6.to_ipv4_mapped() {
                return is_forbidden_smtp_v4(mapped_v4);
            }
            let seg = v6.segments();
            // Deprecated IPv4-compatible ::/96 (check after :: and ::1)
            if seg[..6] == [0, 0, 0, 0, 0, 0] && !(seg[6] == 0 && seg[7] <= 1) {
                let embedded = Ipv4Addr::new(
                    (seg[6] >> 8) as u8,
                    seg[6] as u8,
                    (seg[7] >> 8) as u8,
                    seg[7] as u8,
                );
                return is_forbidden_smtp_v4(embedded);
            }
            // NAT64 well-known prefix 64:ff9b::/96
            if seg[..6] == [0x0064, 0xff9b, 0, 0, 0, 0] {
                let embedded = Ipv4Addr::new(
                    (seg[6] >> 8) as u8,
                    seg[6] as u8,
                    (seg[7] >> 8) as u8,
                    seg[7] as u8,
                );
                return is_forbidden_smtp_v4(embedded);
            }
            // 6to4 2002::/16 (embeds IPv4 in seg[1] and seg[2])
            if seg[0] == 0x2002 {
                let embedded = Ipv4Addr::new(
                    (seg[1] >> 8) as u8,
                    seg[1] as u8,
                    (seg[2] >> 8) as u8,
                    seg[2] as u8,
                );
                return is_forbidden_smtp_v4(embedded);
            }

            v6.is_loopback()
                || v6.is_unspecified()
                || v6.is_multicast()
                // ULA fc00::/7
                || (seg[0] & 0xfe00) == 0xfc00
                // Link-local fe80::/10
                || (seg[0] & 0xffc0) == 0xfe80
                // Site-local fec0::/10
                || (seg[0] & 0xffc0) == 0xfec0
                // Documentation 2001:db8::/32 or Teredo 2001:0000::/32
                || (seg[0] == 0x2001 && (seg[1] == 0x0db8 || seg[1] == 0x0000))
        }
    }
}

/// Validates that a raw port value falls in the valid TCP port range `1..=65535`.
pub fn validate_smtp_port(raw_port: i64) -> Result<u16, ApiError> {
    if !(1..=65535).contains(&raw_port) {
        return Err(ApiError::BadRequest("Invalid SMTP port".into()));
    }
    Ok(raw_port as u16)
}

/// Validates an SMTP host against SSRF rules (rejecting loopback, RFC1918, link-local/metadata,
/// ULA, CGNAT, unspecified IPs, internal hostnames, and DNS resolutions to disallowed IPs),
/// allowing only explicit `mailpit` when `dev_mode` is enabled.
pub async fn validate_smtp_host(host: &str, port: u16, dev_mode: bool) -> Result<(), ApiError> {
    let trimmed = host.trim();
    if trimmed.is_empty()
        || trimmed.chars().any(|c| {
            c.is_whitespace() || c.is_control() || matches!(c, '@' | '/' | '\\' | '?' | '#')
        })
    {
        return Err(ApiError::BadRequest("Invalid SMTP host".into()));
    }

    if dev_mode && trimmed.eq_ignore_ascii_case("mailpit") {
        return Ok(());
    }

    // Handle bracketed IPv6 literals like `[::1]`
    if trimmed.starts_with('[') || trimmed.ends_with(']') {
        if !(trimmed.starts_with('[') && trimmed.ends_with(']')) {
            return Err(ApiError::BadRequest("Invalid SMTP host".into()));
        }
        let inner = &trimmed[1..trimmed.len() - 1];
        let ipv6: std::net::Ipv6Addr = inner
            .parse()
            .map_err(|_| ApiError::BadRequest("Invalid SMTP host".into()))?;
        if is_forbidden_smtp_ip(IpAddr::V6(ipv6)) {
            return Err(ApiError::BadRequest("SMTP host is not allowed".into()));
        }
        return Ok(());
    }

    // Direct IPv4 / IPv6 literal
    if let Ok(ip) = trimmed.parse::<IpAddr>() {
        if is_forbidden_smtp_ip(ip) {
            return Err(ApiError::BadRequest("SMTP host is not allowed".into()));
        }
        return Ok(());
    }

    // Reject embedded ports or colons in hostnames
    if trimmed.contains(':') {
        return Err(ApiError::BadRequest("Invalid SMTP host".into()));
    }

    let dns_host = trimmed.strip_suffix('.').unwrap_or(trimmed);
    let lower = dns_host.to_ascii_lowercase();
    if lower == "localhost"
        || lower.ends_with(".localhost")
        || lower.ends_with(".local")
        || lower.ends_with(".internal")
        || lower.ends_with(".localdomain")
        || lower.ends_with(".arpa")
    {
        return Err(ApiError::BadRequest("SMTP host is not allowed".into()));
    }

    let labels: Vec<&str> = dns_host.split('.').collect();
    if labels.len() < 2
        || labels.iter().any(|label| {
            label.is_empty()
                || label.len() > 63
                || label.starts_with('-')
                || label.ends_with('-')
                || !label.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
        })
        || !labels
            .last()
            .is_some_and(|tld| tld.chars().any(|c| c.is_ascii_alphabetic()))
    {
        return Err(ApiError::BadRequest("Invalid SMTP host".into()));
    }

    let resolved = tokio::time::timeout(
        Duration::from_secs(5),
        tokio::net::lookup_host((dns_host, port)),
    )
    .await
    .map_err(|_| {
        tracing::error!("DNS lookup timed out for SMTP host {}", dns_host);
        ApiError::BadRequest("Failed to resolve SMTP host".into())
    })?
    .map_err(|e| {
        tracing::error!("DNS lookup failed for SMTP host {}: {}", dns_host, e);
        ApiError::BadRequest("Failed to resolve SMTP host".into())
    })?;

    let mut resolved_count = 0usize;
    for addr in resolved {
        resolved_count += 1;
        if is_forbidden_smtp_ip(addr.ip()) {
            tracing::warn!(
                "Blocked SSRF SMTP test for host {} resolving to forbidden IP {}",
                dns_host,
                addr.ip()
            );
            return Err(ApiError::BadRequest("SMTP host is not allowed".into()));
        }
    }

    if resolved_count == 0 {
        return Err(ApiError::BadRequest("Failed to resolve SMTP host".into()));
    }

    Ok(())
}

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
    let port = validate_smtp_port(payload.port as i64)?;
    validate_smtp_host(&payload.host, port, state.config.dev_mode).await?;
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
    responses(
        (status = 200, description = "Mailbox updated", body = OkResponse),
        (status = 404, description = "Mailbox not found", body = ErrorResponse)
    )
)]
pub(crate) async fn update(
    auth: AuthAdmin,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(payload): Json<UpdateMailbox>,
) -> Result<Json<Value>, ApiError> {
    crate::require_permission!(auth, state, "mail.manage");

    let existing: Mailbox = sqlx::query_as("SELECT * FROM mailboxes WHERE id = $1")
        .bind(id)
        .fetch_optional(&state.db)
        .await?
        .ok_or(ApiError::NotFound("mailbox not found".into()))?;

    let effective_port = match payload.port {
        Some(p) => validate_smtp_port(p as i64)?,
        None => validate_smtp_port(existing.port as i64)?,
    };

    if let Some(ref new_host) = payload.host {
        validate_smtp_host(new_host, effective_port, state.config.dev_mode).await?;
    }

    // Preserve existing DB password when password is omitted or empty,
    // UNLESS host, port, or user_name is being modified (prevents SMTP credential exfiltration).
    let password = payload
        .password
        .as_deref()
        .filter(|p| !p.trim().is_empty());

    let host_changed = payload
        .host
        .as_deref()
        .is_some_and(|h| !h.trim().eq_ignore_ascii_case(existing.host.trim()));
    let port_changed = payload.port.is_some_and(|p| p != existing.port);
    let user_changed = payload
        .user_name
        .as_deref()
        .is_some_and(|u| u.trim() != existing.user_name.trim());

    if (host_changed || port_changed || user_changed) && password.is_none() {
        return Err(ApiError::BadRequest(
            "Password must be re-entered when changing the SMTP host, port, or username".into(),
        ));
    }

    let res = sqlx::query(
        r#"UPDATE mailboxes
           SET name = COALESCE($1, name),
               host = COALESCE($2, host),
               port = COALESCE($3, port),
               user_name = COALESCE($4, user_name),
               password = COALESCE($5, password),
               is_default = COALESCE($6, is_default)
           WHERE id = $7"#,
    )
    .bind(payload.name.as_deref())
    .bind(payload.host.as_deref())
    .bind(payload.port)
    .bind(payload.user_name.as_deref())
    .bind(password)
    .bind(payload.is_default)
    .bind(id)
    .execute(&state.db)
    .await?;

    if res.rows_affected() == 0 {
        return Err(ApiError::NotFound("mailbox not found".into()));
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

#[allow(clippy::too_many_arguments)]
async fn execute_smtp_test(
    state: &AppState,
    host: &str,
    raw_port: i64,
    user: &str,
    pass: &str,
    from_addr: &str,
    recipient: Option<&str>,
    body_text: &str,
    connect_success_msg: &str,
    connect_rejected_msg: &str,
) -> Result<Json<Value>, ApiError> {
    let port = validate_smtp_port(raw_port)?;
    validate_smtp_host(host, port, state.config.dev_mode).await?;

    let trimmed_host = host.trim();
    let relay_host = if trimmed_host.starts_with('[') && trimmed_host.ends_with(']') {
        &trimmed_host[1..trimmed_host.len() - 1]
    } else {
        trimmed_host
    };

    let creds = Credentials::new(user.to_string(), pass.to_string());
    let mailer: AsyncSmtpTransport<Tokio1Executor> =
        AsyncSmtpTransport::<Tokio1Executor>::relay(relay_host)
            .map_err(|e| {
                tracing::error!("SMTP relay configuration error for host {}: {}", relay_host, e);
                ApiError::Internal("Failed to configure SMTP transport".into())
            })?
            .port(port)
            .timeout(Some(Duration::from_secs(10)))
            .credentials(creds)
            .build();

    if let Some(to_addr) = recipient.map(str::trim).filter(|s| !s.is_empty()) {
        let email = Message::builder()
            .from(from_addr.parse().map_err(|e: lettre::address::AddressError| {
                tracing::warn!("Invalid SMTP sender address '{}': {}", from_addr, e);
                ApiError::BadRequest("Invalid sender email address".into())
            })?)
            .to(to_addr.parse().map_err(|e: lettre::address::AddressError| {
                tracing::warn!("Invalid SMTP recipient address '{}': {}", to_addr, e);
                ApiError::BadRequest("Invalid recipient email address".into())
            })?)
            .subject("VaultScope Mailbox Test")
            .body(body_text.to_string())
            .map_err(|e| {
                tracing::error!("Failed to build SMTP test message: {}", e);
                ApiError::Internal("Failed to construct test email".into())
            })?;

        mailer.send(email).await.map_err(|e| {
            tracing::error!("SMTP test email send failed via {}: {}", relay_host, e);
            ApiError::Internal("Failed to send test email".into())
        })?;

        Ok(Json(
            json!({ "ok": true, "message": "Test email sent successfully" }),
        ))
    } else {
        let ok = mailer.test_connection().await.map_err(|e| {
            tracing::error!("SMTP test_connection failed for {}: {}", relay_host, e);
            ApiError::Internal("Failed to connect to SMTP server".into())
        })?;
        if ok {
            Ok(Json(
                json!({ "ok": true, "message": connect_success_msg }),
            ))
        } else {
            Err(ApiError::Internal(connect_rejected_msg.into()))
        }
    }
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
        (status = 400, description = "Invalid or disallowed SMTP configuration", body = ErrorResponse),
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

    execute_smtp_test(
        &state,
        &mailbox.host,
        mailbox.port as i64,
        &mailbox.user_name,
        &mailbox.password,
        &mailbox.user_name,
        payload.send_test_email.as_deref(),
        "This is a test email to verify mailbox settings.",
        "Connection successful",
        "Failed to connect to SMTP server",
    )
    .await
}

#[derive(Deserialize, ToSchema)]
pub(crate) struct TestSmtpPayload {
    smtp_host: String,
    smtp_port: Option<i64>,
    #[serde(default)]
    smtp_user: String,
    #[serde(default)]
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
    responses(
        (status = 200, description = "SMTP test result", body = OkMessageResponse),
        (status = 400, description = "Invalid or disallowed SMTP configuration", body = ErrorResponse)
    )
)]
pub(crate) async fn test_smtp_credentials(
    auth: AuthAdmin,
    State(state): State<AppState>,
    Json(payload): Json<TestSmtpPayload>,
) -> Result<Json<Value>, ApiError> {
    crate::require_permission!(auth, state, "mail.manage");

    let port = payload.smtp_port.unwrap_or(587);
    let from_addr = payload.email.as_deref().unwrap_or(&payload.smtp_user);

    execute_smtp_test(
        &state,
        &payload.smtp_host,
        port,
        &payload.smtp_user,
        &payload.smtp_pass,
        from_addr,
        payload.test_to.as_deref(),
        "This is a test email to verify your mailbox settings in VaultScope.",
        "SMTP connection successful",
        "SMTP server rejected the connection",
    )
    .await
}

#[utoipa::path(
    put,
    path = "/{id}/default",
    tag = "Admin Mailboxes",
    security(("bearer_auth" = []), ("csrf_token" = [])),
    params(("id" = Uuid, Path, description = "Mailbox ID")),
    responses(
        (status = 200, description = "Default mailbox updated", body = OkResponse),
        (status = 404, description = "Mailbox not found", body = ErrorResponse)
    )
)]
pub(crate) async fn set_default(
    auth: AuthAdmin,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, ApiError> {
    crate::require_permission!(auth, state, "mail.manage");

    let mut tx = state.db.begin().await?;

    let exists: Option<(Uuid,)> = sqlx::query_as("SELECT id FROM mailboxes WHERE id = $1")
        .bind(id)
        .fetch_optional(&mut *tx)
        .await?;
    if exists.is_none() {
        return Err(ApiError::NotFound("mailbox not found".into()));
    }

    sqlx::query("UPDATE mailboxes SET is_default = false WHERE is_default = true")
        .execute(&mut *tx)
        .await?;
    sqlx::query("UPDATE mailboxes SET is_default = true WHERE id = $1")
        .bind(id)
        .execute(&mut *tx)
        .await?;

    tx.commit().await?;

    Ok(Json(json!({ "ok": true })))
}
