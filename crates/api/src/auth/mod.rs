pub mod jwt;

use axum::extract::{FromRef, FromRequestParts};
use axum::http::header::AUTHORIZATION;
use axum::http::request::Parts;
use axum::http::HeaderMap;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::ApiError;
use crate::state::{AppState, SseTicketEntry};

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct Claims {
    pub sub: Uuid,
    pub email: String,
    pub role: String,
    pub audience: Audience,
    pub exp: i64,
    pub iat: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth_version: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub jti: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum Audience {
    Admin,
    Storefront,
}

pub struct AuthAdmin(pub Claims);
pub struct AuthCustomer(pub Claims);

impl<S> FromRequestParts<S> for AuthAdmin
where
    S: Send + Sync,
    AppState: FromRef<S>,
{
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let app_state = AppState::from_ref(state);
        // SEC-003: Pass expected audience to JWT/ticket verification
        let claims = extract_claims(parts, &app_state, Audience::Admin).await?;
        // Double-check audience (defense in depth)
        if claims.audience != Audience::Admin {
            return Err(ApiError::Forbidden);
        }
        verify_admin_live(&app_state, &claims).await?;
        Ok(AuthAdmin(claims))
    }
}

impl<S> FromRequestParts<S> for AuthCustomer
where
    S: Send + Sync,
    AppState: FromRef<S>,
{
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let app_state = AppState::from_ref(state);
        // SEC-003: Pass expected audience to JWT/ticket verification
        let claims = extract_claims(parts, &app_state, Audience::Storefront).await?;
        // Double-check audience (defense in depth)
        if claims.audience != Audience::Storefront {
            return Err(ApiError::Forbidden);
        }
        verify_customer_live(&app_state, &claims).await?;
        Ok(AuthCustomer(claims))
    }
}

pub async fn verify_admin_live(state: &AppState, claims: &Claims) -> Result<(), ApiError> {
    let claim_version = claims.auth_version.ok_or(ApiError::Unauthorized)?;
    let db_version: Option<i64> = sqlx::query_scalar("SELECT auth_version FROM staff WHERE id = $1")
        .bind(claims.sub)
        .fetch_optional(&state.db)
        .await?;

    let Some(db_ver) = db_version else {
        return Err(ApiError::Unauthorized);
    };
    if claim_version != db_ver {
        return Err(ApiError::Unauthorized);
    }
    Ok(())
}

pub async fn verify_customer_live(state: &AppState, claims: &Claims) -> Result<(), ApiError> {
    let claim_version = claims.auth_version.ok_or(ApiError::Unauthorized)?;
    let row: Option<(String, String, i64)> = sqlx::query_as(
        "SELECT status::text, lifecycle_state, auth_version FROM customers WHERE id = $1",
    )
    .bind(claims.sub)
    .fetch_optional(&state.db)
    .await?;

    let Some((status, lifecycle_state, db_ver)) = row else {
        return Err(ApiError::Unauthorized);
    };
    if status != "active" || lifecycle_state != "active" || claim_version != db_ver {
        return Err(ApiError::Unauthorized);
    }
    Ok(())
}

/// Authenticates a Bearer JWT of either Admin or Storefront audience against the live database.
pub async fn authenticate_any_bearer(
    headers: &HeaderMap,
    state: &AppState,
) -> Result<Claims, ApiError> {
    let auth_header = headers
        .get(AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .ok_or(ApiError::Unauthorized)?;
    let token = auth_header
        .strip_prefix("Bearer ")
        .ok_or(ApiError::Unauthorized)?;

    let claims = jwt::verify_any_audience(token, &state.config.jwt_secret)?;
    match claims.audience {
        Audience::Admin => verify_admin_live(state, &claims).await?,
        Audience::Storefront => verify_customer_live(state, &claims).await?,
    }
    Ok(claims)
}

async fn extract_claims(
    parts: &Parts,
    state: &AppState,
    expected_audience: Audience,
) -> Result<Claims, ApiError> {
    if let Some(header) = parts
        .headers
        .get(AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
    {
        let token = header
            .strip_prefix("Bearer ")
            .ok_or(ApiError::Unauthorized)?;
        return jwt::verify(token, &state.config.jwt_secret, expected_audience);
    }

    // Never accept raw JWTs via `?token=` query string.
    // Only single-use short-lived `?ticket=` is accepted on SSE `/events` endpoints.
    if parts.uri.path().ends_with("/events") {
        if let Some(query) = parts.uri.query() {
            if let Some(ticket) = query.split('&').find_map(|pair| {
                let mut kv = pair.splitn(2, '=');
                if kv.next()? == "ticket" {
                    kv.next()
                } else {
                    None
                }
            }) {
                return consume_sse_ticket(state, ticket, expected_audience).await;
            }
        }
    }

    Err(ApiError::Unauthorized)
}

async fn consume_sse_ticket(
    state: &AppState,
    ticket: &str,
    expected_audience: Audience,
) -> Result<Claims, ApiError> {
    if ticket.is_empty()
        || ticket.len() > 128
        || !ticket.bytes().all(|b| b.is_ascii_hexdigit())
    {
        return Err(ApiError::Unauthorized);
    }

    let now = Utc::now().timestamp();
    let mut entry: Option<SseTicketEntry> = None;

    if let Some(mut conn) = state.redis.clone() {
        let key = format!("vs:sse:{}", ticket);
        let script = redis::Script::new(
            r#"local v = redis.call('GET', KEYS[1])
               if v then redis.call('DEL', KEYS[1]) end
               return v"#,
        );
        match script.key(&key).invoke_async::<_, Option<String>>(&mut conn).await {
            Ok(Some(raw)) => {
                entry = serde_json::from_str(&raw).ok();
            }
            Ok(None) => {}
            Err(e) => {
                tracing::warn!("Redis SSE ticket lookup failed: {}", e);
            }
        }
    }

    if entry.is_none() {
        let mut store = state.sse_tickets.write().await;
        entry = store.remove(ticket);
        store.retain(|_, v| v.expires_at >= now);
    }

    let entry = entry.ok_or(ApiError::Unauthorized)?;
    if entry.expires_at < now {
        return Err(ApiError::Unauthorized);
    }
    if entry.audience != expected_audience {
        return Err(ApiError::Forbidden);
    }

    Ok(Claims {
        sub: entry.sub,
        email: entry.email,
        role: entry.role,
        audience: entry.audience,
        exp: entry.expires_at,
        iat: now,
        auth_version: entry.auth_version,
        jti: None,
    })
}
