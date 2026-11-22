use std::net::IpAddr;
use std::sync::LazyLock;

use axum::extract::{ConnectInfo, FromRequestParts, State};
use axum::http::request::Parts;
use axum::http::HeaderMap;
use axum::Json;
use bcrypt::{hash, verify, DEFAULT_COST};
use chrono::Utc;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use totp_rs::{Algorithm, Secret, TOTP};
use utoipa::ToSchema;
use utoipa_axum::{router::OpenApiRouter, routes};
use uuid::Uuid;

use crate::auth::{authenticate_any_bearer, jwt, Audience, AuthAdmin, Claims};
use crate::error::ApiError;
use crate::middleware::rate_limit::{extract_trusted_client_ip, rate_limit_middleware, RateLimiter};
use crate::openapi::{ErrorResponse, OkMessageResponse, OkResponse};
use crate::state::{AppState, SseTicketEntry};
use crate::validation::{validate_email, validate_required_string, validate_string_length};

const ENCRYPTED_TOTP_PREFIX: &str = "enc:v1:";
const MAX_LOCAL_SSE_TICKETS: usize = 50_000;

static DUMMY_BCRYPT_HASH: LazyLock<String> = LazyLock::new(|| {
    hash("vaultscope-constant-time-dummy-password", DEFAULT_COST)
        .expect("dummy bcrypt hash generation")
});

/// Optional peer TCP address extractor that gracefully yields `None` in `oneshot` unit tests
/// where `ConnectInfo<SocketAddr>` is absent.
pub(crate) struct OptionalPeerIp(pub Option<IpAddr>);

impl<S> FromRequestParts<S> for OptionalPeerIp
where
    S: Send + Sync,
{
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        let ip = parts
            .extensions
            .get::<ConnectInfo<std::net::SocketAddr>>()
            .map(|ci| ci.0.ip());
        Ok(OptionalPeerIp(ip))
    }
}

pub fn routes(_state: &AppState) -> OpenApiRouter<AppState> {
    let strict_auth_limiter = RateLimiter::new(20, 60);

    let strict_routes = OpenApiRouter::new()
        .routes(routes!(login))
        .routes(routes!(register))
        .routes(routes!(verify_email))
        .routes(routes!(resend_verification))
        .routes(routes!(admin_mfa_setup))
        .routes(routes!(admin_mfa_enable))
        .routes(routes!(admin_mfa_disable))
        .layer(axum::middleware::from_fn(rate_limit_middleware))
        .layer(axum::extract::Extension(strict_auth_limiter));

    OpenApiRouter::new()
        .merge(strict_routes)
        .routes(routes!(logout))
        .routes(routes!(create_sse_ticket))
}

pub(crate) fn encrypt_totp_secret(key: &[u8; 32], secret_base32: &str) -> Result<String, ApiError> {
    let encrypted = vaultscope_common::crypto::encrypt(key, secret_base32.as_bytes())
        .map_err(|e| ApiError::Internal(format!("Failed to encrypt TOTP secret: {e}")))?;
    Ok(format!("{ENCRYPTED_TOTP_PREFIX}{}", hex::encode(encrypted)))
}

pub(crate) fn decrypt_totp_secret(key: &[u8; 32], stored: &str) -> Result<String, ApiError> {
    if let Some(hex_ct) = stored.strip_prefix(ENCRYPTED_TOTP_PREFIX) {
        let bytes = hex::decode(hex_ct)
            .map_err(|_| ApiError::Internal("Corrupt encrypted TOTP secret".into()))?;
        let plaintext = vaultscope_common::crypto::decrypt(key, &bytes)
            .map_err(|_| ApiError::Internal("Failed to decrypt TOTP secret".into()))?;
        String::from_utf8(plaintext)
            .map_err(|_| ApiError::Internal("Invalid UTF-8 in TOTP secret".into()))
    } else {
        // Legacy plaintext Base32 row fallback.
        Ok(stored.to_string())
    }
}

pub(crate) fn verify_totp_code(
    secret_base32: &str,
    email: &str,
    raw_code: &str,
) -> Result<Option<String>, ApiError> {
    let secret_bytes = Secret::Encoded(secret_base32.to_string())
        .to_bytes()
        .map_err(|e| ApiError::Internal(format!("Secret parse error: {e}")))?;
    let totp = TOTP::new(
        Algorithm::SHA1,
        6,
        1,
        30,
        secret_bytes,
        Some("VaultScope".to_string()),
        email.to_string(),
    )
    .map_err(|e| ApiError::Internal(format!("TOTP init error: {e}")))?;

    let clean_code = raw_code.trim().replace(' ', "");
    if totp.check_current(&clean_code).unwrap_or(false) {
        Ok(Some(clean_code))
    } else {
        Ok(None)
    }
}

pub(crate) async fn check_and_mark_totp_used(
    state: &AppState,
    subject_id: Uuid,
    clean_code: &str,
) -> bool {
    let Some(mut conn) = state.redis.clone() else {
        return true;
    };
    let key = format!("vs:totp_used:{}:{}", subject_id, clean_code);
    let res: redis::RedisResult<Option<String>> = redis::cmd("SET")
        .arg(&key)
        .arg("1")
        .arg("NX")
        .arg("EX")
        .arg(90)
        .query_async(&mut conn)
        .await;
    match res {
        Ok(Some(_)) => true,
        Ok(None) => false,
        Err(e) => {
            tracing::warn!("Redis TOTP anti-replay check failed: {}", e);
            true
        }
    }
}

pub(crate) async fn verify_password_constant_time(
    password: &str,
    stored_hash: Option<&str>,
) -> Result<bool, ApiError> {
    if password.len() > 72 {
        return Ok(false);
    }
    let pw = password.to_string();
    let had_real_hash = stored_hash.is_some();
    let target_hash = stored_hash
        .map(str::to_string)
        .unwrap_or_else(|| DUMMY_BCRYPT_HASH.clone());

    let verified = tokio::task::spawn_blocking(move || verify(&pw, &target_hash).unwrap_or(false))
        .await
        .map_err(|e| ApiError::Internal(format!("Password verification task failed: {e}")))?;

    Ok(had_real_hash && verified)
}

#[derive(Deserialize, ToSchema)]
pub(crate) struct LoginPayload {
    #[serde(default)]
    email: String,
    #[serde(default)]
    username: Option<String>,
    password: String,
    audience: String,
    totp_code: Option<String>,
}

#[derive(Serialize, ToSchema)]
pub struct LoginSuccessResponse {
    pub token: String,
    pub claims: Claims,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub email_verified: bool,
}

#[derive(Serialize, ToSchema)]
pub struct MfaChallengeResponse {
    pub mfa_required: bool,
    pub email: String,
    pub message: String,
}

#[allow(dead_code)]
#[derive(Serialize, ToSchema)]
#[serde(untagged)]
pub enum LoginOutcome {
    Token(LoginSuccessResponse),
    MfaChallenge(MfaChallengeResponse),
}

#[utoipa::path(
    post,
    path = "/login",
    tag = "Auth",
    request_body = LoginPayload,
    responses(
        (status = 200, description = "JWT session or 2FA challenge required", body = LoginOutcome),
        (status = 400, description = "Validation, unverified email, or invalid 2FA/recovery code", body = ErrorResponse),
        (status = 401, description = "Invalid credentials", body = ErrorResponse)
    )
)]
#[allow(clippy::type_complexity)]
pub(crate) async fn login(
    State(state): State<AppState>,
    headers: HeaderMap,
    OptionalPeerIp(peer_ip): OptionalPeerIp,
    Json(payload): Json<LoginPayload>,
) -> Result<Json<Value>, ApiError> {
    let identifier = if !payload.email.trim().is_empty() {
        payload.email.trim().to_string()
    } else if let Some(ref u) = payload.username {
        u.trim().to_string()
    } else {
        String::new()
    };

    validate_required_string(&identifier, "Email or username")?;
    validate_required_string(&payload.password, "Password")?;

    let is_admin = payload.audience == "admin";

    if is_admin {
        if identifier.contains('@') {
            validate_email(&identifier)?;
        }
        let staff: Option<(
            Uuid,
            String,
            String,
            Option<String>,
            bool,
            bool,
            Option<String>,
            i64,
            String,
        )> = sqlx::query_as(
            "SELECT s.id, s.email, r.name, s.password_hash, s.email_verified, s.mfa_enabled, s.totp_secret, s.auth_version, s.external_id
             FROM staff s
             JOIN roles r ON s.role_id = r.id
             WHERE (LOWER(s.email) = LOWER($1) OR (s.username <> '' AND LOWER(s.username) = LOWER($1)))
             LIMIT 1",
        )
        .bind(&identifier)
        .fetch_optional(&state.db)
        .await?;

        let linked_customer: Option<(Uuid, String, Option<String>, bool, Option<String>)> =
            if let Some((_, ref s_email, _, _, _, _, _, _, ref s_ext_id)) = staff {
                sqlx::query_as(
                    "SELECT id, email, password_hash, two_factor_enabled, totp_secret
                     FROM customers
                     WHERE LOWER(email) = LOWER($1) OR ('customer_' || id::text) = $2
                     LIMIT 1",
                )
                .bind(s_email)
                .bind(s_ext_id)
                .fetch_optional(&state.db)
                .await?
            } else {
                None
            };

        let staff_hash = staff.as_ref().and_then(|row| row.3.as_deref());
        let customer_hash = linked_customer.as_ref().and_then(|row| row.2.as_deref());

        let mut password_ok = verify_password_constant_time(&payload.password, staff_hash).await?;
        let mut matched_via_customer_hash = false;
        if !password_ok
            && customer_hash.is_some()
            && customer_hash != staff_hash
            && verify_password_constant_time(&payload.password, customer_hash).await?
        {
            password_ok = true;
            matched_via_customer_hash = true;
        }

        if !password_ok {
            return Err(ApiError::Unauthorized);
        }

        let (
            staff_id,
            email,
            role_name,
            _,
            email_verified,
            mfa_enabled,
            totp_secret,
            auth_version,
            _,
        ) = staff.ok_or(ApiError::Unauthorized)?;

        let (linked_customer_id, linked_customer_email, linked_customer_pw, linked_customer_2fa, linked_customer_totp) =
            match linked_customer {
                Some((cid, cemail, cpw, c2fa, ctotp)) => (Some(cid), Some(cemail), cpw, c2fa, ctotp),
                None => (None, None, None, false, None),
            };

        // Keep staff password_hash synchronized if authenticated via linked VaultScope customer credentials
        if matched_via_customer_hash {
            if let Some(ref cpw) = linked_customer_pw {
                let _ = sqlx::query("UPDATE staff SET password_hash = $1, updated_at = now() WHERE id = $2")
                    .bind(cpw)
                    .bind(staff_id)
                    .execute(&state.db)
                    .await;
            }
        }

        let staff_secret_clean = totp_secret.filter(|s| !s.trim().is_empty());
        let customer_secret_clean = linked_customer_totp.filter(|s| !s.trim().is_empty());
        let effective_mfa_enabled = mfa_enabled || linked_customer_2fa;

        // Backfill staff MFA columns if linked VaultScope customer has 2FA enabled
        if linked_customer_2fa && (!mfa_enabled || staff_secret_clean.is_none()) {
            if let Some(ref c_sec) = customer_secret_clean {
                let _ = sqlx::query(
                    "UPDATE staff SET mfa_enabled = true, totp_secret = COALESCE(NULLIF(trim(totp_secret), ''), $1), updated_at = now() WHERE id = $2",
                )
                .bind(c_sec)
                .bind(staff_id)
                .execute(&state.db)
                .await;
            }
        }

        if effective_mfa_enabled {
            let candidate_secrets: Vec<(&str, &str)> = {
                let mut v = Vec::new();
                if let Some(ref c_sec) = customer_secret_clean {
                    if linked_customer_2fa {
                        v.push((c_sec.as_str(), linked_customer_email.as_deref().unwrap_or(&email)));
                    }
                }
                if let Some(ref s_sec) = staff_secret_clean {
                    if v.iter().all(|(existing, _)| *existing != s_sec.as_str()) {
                        v.push((s_sec.as_str(), email.as_str()));
                    }
                }
                if v.is_empty() {
                    if let Some(ref c_sec) = customer_secret_clean {
                        v.push((c_sec.as_str(), linked_customer_email.as_deref().unwrap_or(&email)));
                    }
                }
                v
            };

            if candidate_secrets.is_empty() {
                tracing::error!(staff_id = %staff_id, "Admin MFA enabled without configured TOTP secret");
                return Err(ApiError::Unauthorized);
            }

            let provided_code = payload
                .totp_code
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty());

            let Some(raw_code) = provided_code else {
                return Ok(Json(json!({
                    "mfa_required": true,
                    "email": email,
                    "message": "Two-factor authentication code required"
                })));
            };

            let mut mfa_ok = false;
            for (secret_stored, acct_email) in candidate_secrets {
                if let Ok(secret_base32) =
                    decrypt_totp_secret(&state.config.encryption_key_bytes(), secret_stored)
                {
                    if let Ok(Some(clean_code)) = verify_totp_code(&secret_base32, acct_email, raw_code) {
                        let staff_fresh = check_and_mark_totp_used(&state, staff_id, &clean_code).await;
                        let cust_fresh = match linked_customer_id {
                            Some(cid) => check_and_mark_totp_used(&state, cid, &clean_code).await,
                            None => true,
                        };
                        if staff_fresh && cust_fresh {
                            mfa_ok = true;
                            break;
                        }
                    }
                }
            }

            // Also support single-use recovery codes from the linked VaultScope customer account
            if !mfa_ok {
                if let Some(cid) = linked_customer_id {
                    let normalized = raw_code.replace(['-', ' '], "").to_uppercase();
                    if !normalized.is_empty() {
                        let mut hasher = Sha256::new();
                        hasher.update(normalized.as_bytes());
                        let code_hash = hex::encode(hasher.finalize());

                        let consumed: Option<Uuid> = sqlx::query_scalar(
                            r#"WITH target AS (
                                   SELECT id FROM customer_mfa_recovery_codes
                                   WHERE customer_id = $1 AND code_hash = $2 AND used_at IS NULL
                                   LIMIT 1
                                   FOR UPDATE SKIP LOCKED
                               )
                               UPDATE customer_mfa_recovery_codes r
                               SET used_at = now()
                               FROM target
                               WHERE r.id = target.id AND r.used_at IS NULL
                               RETURNING r.id"#,
                        )
                        .bind(cid)
                        .bind(&code_hash)
                        .fetch_optional(&state.db)
                        .await?;

                        if consumed.is_some() {
                            mfa_ok = true;
                            crate::activity::spawn_record(
                                state.db.clone(),
                                Some(staff_id),
                                vaultscope_db::models::ActorType::Staff,
                                email.clone(),
                                "security".to_string(),
                                "2fa_recovery_used".to_string(),
                                "CAMOS Admin".to_string(),
                                format!("Shared recovery code used during CAMOS admin login for {}", email),
                            );
                        }
                    }
                }
            }

            if !mfa_ok {
                return Err(ApiError::BadRequest(
                    "Invalid two-factor authentication code".into(),
                ));
            }
        }

        let client_ip = extract_trusted_client_ip(&headers, peer_ip).unwrap_or_default();
        sqlx::query(
            "UPDATE staff SET last_login = now(), last_login_ip = COALESCE(NULLIF($2, ''), last_login_ip) WHERE id = $1",
        )
        .bind(staff_id)
        .bind(&client_ip)
        .execute(&state.db)
        .await?;

        let claims = Claims {
            sub: staff_id,
            email: email.clone(),
            role: role_name,
            audience: Audience::Admin,
            iat: Utc::now().timestamp(),
            exp: jwt::new_expiry(),
            auth_version: Some(auth_version),
            jti: Some(Uuid::now_v7().to_string()),
        };

        let token = jwt::sign(&claims, &state.config.jwt_secret)?;

        crate::activity::spawn_record(
            state.db.clone(),
            Some(staff_id),
            vaultscope_db::models::ActorType::Staff,
            email.clone(),
            "auth".to_string(),
            "login".to_string(),
            "CAMOS Admin".to_string(),
            format!("Staff login by @{}", email),
        );

        Ok(Json(json!({ "token": token, "claims": claims, "email_verified": email_verified })))
    } else {
        if identifier.contains('@') {
            validate_email(&identifier)?;
        }
        let customer: Option<(
            Uuid,
            String,
            String,
            String,
            Option<String>,
            bool,
            bool,
            Option<String>,
            String,
            String,
            i64,
        )> = sqlx::query_as(
            "SELECT id, COALESCE(username, '') as username, name, email, password_hash,
                    email_verified, two_factor_enabled, totp_secret, status::text,
                    lifecycle_state, auth_version
             FROM customers
             WHERE (LOWER(email) = LOWER($1) OR (username <> '' AND LOWER(username) = LOWER($1)))
             LIMIT 1",
        )
        .bind(&identifier)
        .fetch_optional(&state.db)
        .await?;

        let stored_hash = customer
            .as_ref()
            .and_then(|row| row.4.as_deref());
        if !verify_password_constant_time(&payload.password, stored_hash).await? {
            return Err(ApiError::Unauthorized);
        }

        let (
            customer_id,
            username,
            name,
            email,
            _,
            email_verified,
            two_factor_enabled,
            totp_secret,
            status,
            lifecycle_state,
            auth_version,
        ) = customer.ok_or(ApiError::Unauthorized)?;

        if status != "active" || lifecycle_state != "active" {
            return Err(ApiError::Unauthorized);
        }

        if !email_verified {
            return Err(ApiError::BadRequest(
                "Please verify your email address before signing in.".into(),
            ));
        }

        if two_factor_enabled {
            let provided_code = payload
                .totp_code
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty());

            if let Some(raw_code) = provided_code {
                let mut mfa_ok = false;

                if let Some(ref secret_stored) = totp_secret {
                    if let Ok(secret_base32) =
                        decrypt_totp_secret(&state.config.encryption_key_bytes(), secret_stored)
                    {
                        if let Ok(Some(clean_code)) =
                            verify_totp_code(&secret_base32, &email, raw_code)
                        {
                            if check_and_mark_totp_used(&state, customer_id, &clean_code).await {
                                mfa_ok = true;
                            }
                        }
                    }
                }

                if !mfa_ok {
                    let normalized = raw_code.replace(['-', ' '], "").to_uppercase();
                    let mut hasher = Sha256::new();
                    hasher.update(normalized.as_bytes());
                    let code_hash = hex::encode(hasher.finalize());

                    let consumed: Option<Uuid> = sqlx::query_scalar(
                        r#"WITH target AS (
                               SELECT id FROM customer_mfa_recovery_codes
                               WHERE customer_id = $1 AND code_hash = $2 AND used_at IS NULL
                               LIMIT 1
                               FOR UPDATE SKIP LOCKED
                           )
                           UPDATE customer_mfa_recovery_codes r
                           SET used_at = now()
                           FROM target
                           WHERE r.id = target.id AND r.used_at IS NULL
                           RETURNING r.id"#,
                    )
                    .bind(customer_id)
                    .bind(&code_hash)
                    .fetch_optional(&state.db)
                    .await?;

                    if consumed.is_some() {
                        mfa_ok = true;
                        crate::activity::spawn_record(
                            state.db.clone(),
                            Some(customer_id),
                            vaultscope_db::models::ActorType::Customer,
                            name.clone(),
                            "security".to_string(),
                            "2fa_recovery_used".to_string(),
                            "TOTP Recovery Code".to_string(),
                            format!("Recovery code used during login for {}", email),
                        );
                    }
                }

                if !mfa_ok {
                    return Err(ApiError::BadRequest(
                        "Invalid two-factor authentication or recovery code".into(),
                    ));
                }
            } else {
                return Ok(Json(json!({
                    "mfa_required": true,
                    "email": email,
                    "message": "Two-factor authentication code required"
                })));
            }
        }

        sqlx::query("UPDATE customers SET last_login = now() WHERE id = $1")
            .bind(customer_id)
            .execute(&state.db)
            .await?;

        let claims = Claims {
            sub: customer_id,
            email: email.clone(),
            role: "customer".to_string(),
            audience: Audience::Storefront,
            iat: Utc::now().timestamp(),
            exp: jwt::new_expiry(),
            auth_version: Some(auth_version),
            jti: Some(Uuid::now_v7().to_string()),
        };

        let token = jwt::sign(&claims, &state.config.jwt_secret)?;

        crate::activity::spawn_record(
            state.db.clone(),
            Some(customer_id),
            vaultscope_db::models::ActorType::Customer,
            name.clone(),
            "auth".to_string(),
            "login".to_string(),
            "Customer Portal".to_string(),
            format!("Customer login by {}", email),
        );

        Ok(Json(json!({
            "token": token,
            "claims": claims,
            "username": username,
            "name": name,
            "email_verified": email_verified
        })))
    }
}

#[utoipa::path(
    post,
    path = "/logout",
    tag = "Auth",
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "Session revoked by bumping auth_version", body = OkResponse),
        (status = 401, description = "Unauthorized", body = ErrorResponse)
    )
)]
pub(crate) async fn logout(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    let claims = authenticate_any_bearer(&headers, &state).await?;

    let rows_affected = match claims.audience {
        Audience::Admin => {
            sqlx::query("UPDATE staff SET auth_version = auth_version + 1, updated_at = now() WHERE id = $1")
                .bind(claims.sub)
                .execute(&state.db)
                .await?
                .rows_affected()
        }
        Audience::Storefront => {
            sqlx::query("UPDATE customers SET auth_version = auth_version + 1, updated_at = now() WHERE id = $1")
                .bind(claims.sub)
                .execute(&state.db)
                .await?
                .rows_affected()
        }
    };

    if rows_affected == 0 {
        return Err(ApiError::Unauthorized);
    }

    Ok(Json(json!({ "ok": true })))
}

#[derive(Serialize, ToSchema)]
pub struct SseTicketResponse {
    pub ticket: String,
}

#[utoipa::path(
    post,
    path = "/sse-ticket",
    tag = "Auth",
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "Short-lived single-use SSE ticket", body = SseTicketResponse),
        (status = 401, description = "Unauthorized", body = ErrorResponse)
    )
)]
pub(crate) async fn create_sse_ticket(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    let claims = authenticate_any_bearer(&headers, &state).await?;

    let mut ticket_bytes = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut ticket_bytes);
    let ticket = hex::encode(ticket_bytes);
    let now = Utc::now().timestamp();
    let entry = SseTicketEntry {
        sub: claims.sub,
        email: claims.email,
        role: claims.role,
        audience: claims.audience,
        auth_version: claims.auth_version,
        expires_at: now + 30,
    };

    let mut stored_in_redis = false;
    if let Some(mut conn) = state.redis.clone() {
        if let Ok(serialized) = serde_json::to_string(&entry) {
            let key = format!("vs:sse:{}", ticket);
            let res: redis::RedisResult<()> = redis::cmd("SET")
                .arg(&key)
                .arg(&serialized)
                .arg("EX")
                .arg(30)
                .query_async(&mut conn)
                .await;
            match res {
                Ok(()) => stored_in_redis = true,
                Err(e) => {
                    tracing::warn!("Failed to store SSE ticket in Redis, falling back to memory: {}", e);
                }
            }
        }
    }

    if !stored_in_redis {
        let mut store = state.sse_tickets.write().await;
        store.retain(|_, v| v.expires_at >= now);
        if store.len() >= MAX_LOCAL_SSE_TICKETS {
            if let Some(oldest_key) = store
                .iter()
                .min_by_key(|(_, v)| v.expires_at)
                .map(|(k, _)| k.clone())
            {
                store.remove(&oldest_key);
            }
        }
        store.insert(ticket.clone(), entry);
    }

    Ok(Json(json!({ "ticket": ticket })))
}

#[derive(Deserialize, ToSchema)]
pub(crate) struct AdminMfaCodePayload {
    code: String,
}

#[derive(Deserialize, ToSchema)]
pub(crate) struct AdminMfaDisablePayload {
    code: String,
    password: Option<String>,
}

#[utoipa::path(
    post,
    path = "/admin/mfa/setup",
    tag = "Auth",
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "Generated admin TOTP secret and QR payload", body = Value),
        (status = 400, description = "2FA already enabled", body = ErrorResponse)
    )
)]
pub(crate) async fn admin_mfa_setup(
    auth: AuthAdmin,
    State(state): State<AppState>,
) -> Result<Json<Value>, ApiError> {
    let row: Option<(String, bool, String)> =
        sqlx::query_as("SELECT email, mfa_enabled, external_id FROM staff WHERE id = $1")
            .bind(auth.0.sub)
            .fetch_optional(&state.db)
            .await?;

    let (email, mfa_enabled, external_id) =
        row.ok_or_else(|| ApiError::NotFound("Staff account not found".into()))?;
    let linked_cust_2fa: Option<bool> = sqlx::query_scalar(
        "SELECT two_factor_enabled FROM customers WHERE LOWER(email) = LOWER($1) OR ('customer_' || id::text) = $2 LIMIT 1",
    )
    .bind(&email)
    .bind(&external_id)
    .fetch_optional(&state.db)
    .await?;

    if mfa_enabled || linked_cust_2fa.unwrap_or(false) {
        return Err(ApiError::BadRequest(
            "Two-factor authentication is already enabled. Disable it first before reconfiguring.".into(),
        ));
    }

    let secret = Secret::generate_secret();
    let secret_base32 = secret.to_encoded().to_string();
    let secret_bytes = secret
        .to_bytes()
        .map_err(|e| ApiError::Internal(format!("Secret generation error: {e}")))?;

    let totp = TOTP::new(
        Algorithm::SHA1,
        6,
        1,
        30,
        secret_bytes,
        Some("VaultScope Admin".to_string()),
        email.clone(),
    )
    .map_err(|e| ApiError::Internal(format!("TOTP init error: {e}")))?;

    let qr_base64 = totp
        .get_qr_base64()
        .map_err(|e| ApiError::Internal(format!("QR generation error: {e}")))?;
    let qr_url = format!("data:image/png;base64,{}", qr_base64);
    let url = totp.get_url();

    let encrypted_secret = encrypt_totp_secret(&state.config.encryption_key_bytes(), &secret_base32)?;
    sqlx::query("UPDATE staff SET totp_secret = $1, updated_at = now() WHERE id = $2")
        .bind(&encrypted_secret)
        .bind(auth.0.sub)
        .execute(&state.db)
        .await?;

    let _ = sqlx::query(
        "UPDATE customers SET totp_secret = $1, updated_at = now() WHERE (LOWER(email) = LOWER($2) OR ('customer_' || id::text) = $3) AND two_factor_enabled = false",
    )
    .bind(&encrypted_secret)
    .bind(&email)
    .bind(&external_id)
    .execute(&state.db)
    .await;

    Ok(Json(json!({
        "secret": secret_base32,
        "qr_url": qr_url,
        "qr_base64": qr_base64,
        "url": url,
    })))
}

#[utoipa::path(
    post,
    path = "/admin/mfa/enable",
    tag = "Auth",
    security(("bearer_auth" = [])),
    request_body = AdminMfaCodePayload,
    responses(
        (status = 200, description = "Admin 2FA enabled", body = OkMessageResponse),
        (status = 400, description = "Invalid TOTP code or setup not initiated", body = ErrorResponse)
    )
)]
pub(crate) async fn admin_mfa_enable(
    auth: AuthAdmin,
    State(state): State<AppState>,
    Json(payload): Json<AdminMfaCodePayload>,
) -> Result<Json<Value>, ApiError> {
    let row: Option<(String, bool, Option<String>, String)> =
        sqlx::query_as("SELECT email, mfa_enabled, totp_secret, external_id FROM staff WHERE id = $1")
            .bind(auth.0.sub)
            .fetch_optional(&state.db)
            .await?;

    let (email, mfa_enabled, totp_secret, external_id) =
        row.ok_or_else(|| ApiError::NotFound("Staff account not found".into()))?;
    if mfa_enabled {
        return Err(ApiError::BadRequest(
            "Two-factor authentication is already enabled.".into(),
        ));
    }

    let secret_stored = totp_secret.ok_or_else(|| {
        ApiError::BadRequest("MFA setup was not initiated. Please generate a setup key first.".into())
    })?;
    let secret_base32 = decrypt_totp_secret(&state.config.encryption_key_bytes(), &secret_stored)?;
    let encrypted_secret = encrypt_totp_secret(&state.config.encryption_key_bytes(), &secret_base32)?;

    let Some(clean_code) = verify_totp_code(&secret_base32, &email, &payload.code)? else {
        return Err(ApiError::BadRequest(
            "Invalid 6-digit verification code. Please check your authenticator app and system clock.".into(),
        ));
    };

    if !check_and_mark_totp_used(&state, auth.0.sub, &clean_code).await {
        return Err(ApiError::BadRequest(
            "This verification code was already used. Please wait for the next 30-second code.".into(),
        ));
    }

    sqlx::query(
        "UPDATE staff SET mfa_enabled = true, totp_secret = $2, auth_version = auth_version + 1, updated_at = now() WHERE id = $1",
    )
    .bind(auth.0.sub)
    .bind(&encrypted_secret)
    .execute(&state.db)
    .await?;

    // Synchronize 2FA activation to linked VaultScope customer account if present
    let _ = sqlx::query(
        "UPDATE customers SET two_factor_enabled = true, totp_secret = $1, auth_version = auth_version + 1, updated_at = now() WHERE LOWER(email) = LOWER($2) OR ('customer_' || id::text) = $3",
    )
    .bind(&encrypted_secret)
    .bind(&email)
    .bind(&external_id)
    .execute(&state.db)
    .await;

    Ok(Json(json!({
        "ok": true,
        "message": "Admin two-factor authentication successfully activated."
    })))
}

#[utoipa::path(
    post,
    path = "/admin/mfa/disable",
    tag = "Auth",
    security(("bearer_auth" = [])),
    request_body = AdminMfaDisablePayload,
    responses(
        (status = 200, description = "Admin 2FA disabled", body = OkMessageResponse),
        (status = 400, description = "Invalid password or TOTP code", body = ErrorResponse)
    )
)]
#[allow(clippy::type_complexity)]
pub(crate) async fn admin_mfa_disable(
    auth: AuthAdmin,
    State(state): State<AppState>,
    Json(payload): Json<AdminMfaDisablePayload>,
) -> Result<Json<Value>, ApiError> {
    let row: Option<(String, bool, Option<String>, Option<String>, String)> = sqlx::query_as(
        "SELECT email, mfa_enabled, totp_secret, password_hash, external_id FROM staff WHERE id = $1",
    )
    .bind(auth.0.sub)
    .fetch_optional(&state.db)
    .await?;

    let (email, mfa_enabled, totp_secret, password_hash, external_id) =
        row.ok_or_else(|| ApiError::NotFound("Staff account not found".into()))?;

    let linked_cust: Option<(Uuid, Option<String>, bool, Option<String>)> = sqlx::query_as(
        "SELECT id, password_hash, two_factor_enabled, totp_secret FROM customers WHERE LOWER(email) = LOWER($1) OR ('customer_' || id::text) = $2 LIMIT 1",
    )
    .bind(&email)
    .bind(&external_id)
    .fetch_optional(&state.db)
    .await?;

    let cust_2fa = linked_cust.as_ref().is_some_and(|c| c.2);
    if !mfa_enabled && !cust_2fa {
        return Err(ApiError::BadRequest(
            "Two-factor authentication is not currently enabled.".into(),
        ));
    }

    let cust_pw_hash = linked_cust.as_ref().and_then(|c| c.1.as_deref());
    if password_hash.is_some() || cust_pw_hash.is_some() {
        let pw = payload
            .password
            .as_deref()
            .filter(|s| !s.is_empty())
            .ok_or_else(|| {
                ApiError::BadRequest(
                    "Current password and a valid 2FA code are required to disable two-factor authentication.".into(),
                )
            })?;
        let mut pw_ok = verify_password_constant_time(pw, password_hash.as_deref()).await?;
        if !pw_ok && cust_pw_hash.is_some() && cust_pw_hash != password_hash.as_deref() {
            pw_ok = verify_password_constant_time(pw, cust_pw_hash).await?;
        }
        if !pw_ok {
            return Err(ApiError::BadRequest(
                "Invalid current password or 2FA code to disable two-factor authentication.".into(),
            ));
        }
    }

    let effective_secret = totp_secret
        .filter(|s| !s.trim().is_empty())
        .or_else(|| linked_cust.as_ref().and_then(|c| c.3.clone()).filter(|s| !s.trim().is_empty()))
        .ok_or_else(|| ApiError::BadRequest("No TOTP secret configured on account.".into()))?;

    let secret_base32 = decrypt_totp_secret(&state.config.encryption_key_bytes(), &effective_secret)?;
    let Some(clean_code) = verify_totp_code(&secret_base32, &email, &payload.code)? else {
        return Err(ApiError::BadRequest(
            "Invalid 2FA code to disable two-factor authentication.".into(),
        ));
    };

    if !check_and_mark_totp_used(&state, auth.0.sub, &clean_code).await {
        return Err(ApiError::BadRequest(
            "This verification code was already used. Please wait for the next 30-second code.".into(),
        ));
    }

    sqlx::query(
        "UPDATE staff SET mfa_enabled = false, totp_secret = NULL, auth_version = auth_version + 1, updated_at = now() WHERE id = $1",
    )
    .bind(auth.0.sub)
    .execute(&state.db)
    .await?;

    if let Some((cid, _, _, _)) = linked_cust {
        let _ = sqlx::query(
            "UPDATE customers SET two_factor_enabled = false, totp_secret = NULL, auth_version = auth_version + 1, updated_at = now() WHERE id = $1",
        )
        .bind(cid)
        .execute(&state.db)
        .await;
        let _ = sqlx::query("DELETE FROM customer_mfa_recovery_codes WHERE customer_id = $1")
            .bind(cid)
            .execute(&state.db)
            .await;
    }

    Ok(Json(json!({
        "ok": true,
        "message": "Admin two-factor authentication has been disabled."
    })))
}

const EU_MEMBER_STATE_CODES: &[&str] = &[
    "AT", "BE", "BG", "HR", "CY", "CZ", "DK", "EE", "FI", "FR", "DE", "GR", "HU", "IE", "IT",
    "LV", "LT", "LU", "MT", "NL", "PL", "PT", "RO", "SK", "SI", "ES", "SE",
];

#[derive(Deserialize, ToSchema)]
pub(crate) struct RegisterPayload {
    username: String,
    email: String,
    password: String,
    name: String,
    company: Option<String>,
    address: String,
    postal_code: Option<String>,
    city: String,
    country: String,
    phone: Option<String>,
    vat_id: Option<String>,
}

#[utoipa::path(
    post,
    path = "/register",
    tag = "Auth",
    request_body = RegisterPayload,
    responses(
        (status = 200, description = "Account created and verification email dispatched", body = OkMessageResponse),
        (status = 400, description = "Validation error or account already exists", body = ErrorResponse)
    )
)]
pub(crate) async fn register(
    State(state): State<AppState>,
    Json(payload): Json<RegisterPayload>,
) -> Result<Json<Value>, ApiError> {
    validate_required_string(&payload.username, "Username")?;
    validate_required_string(&payload.email, "Email")?;
    validate_required_string(&payload.password, "Password")?;
    validate_required_string(&payload.name, "Name")?;
    validate_required_string(&payload.address, "Address")?;
    validate_required_string(&payload.city, "City")?;
    validate_required_string(&payload.country, "Country")?;
    validate_email(&payload.email)?;
    validate_string_length(&payload.username, "Username", 150)?;
    if payload.username.contains('@') {
        return Err(ApiError::BadRequest(
            "Username cannot contain '@'".into(),
        ));
    }
    validate_string_length(&payload.name, "Name", 255)?;
    validate_string_length(&payload.address, "Address", 500)?;
    validate_string_length(&payload.city, "City", 150)?;
    validate_string_length(&payload.country, "Country", 100)?;
    let normalized_country = payload.country.trim().to_uppercase();
    if !EU_MEMBER_STATE_CODES.contains(&normalized_country.as_str()) {
        return Err(ApiError::BadRequest(
            "Account registration is only available for European Union (EU) member states.".into(),
        ));
    }
    if let Some(ref company) = payload.company {
        validate_string_length(company, "Company", 255)?;
    }
    if let Some(ref postal) = payload.postal_code {
        validate_string_length(postal, "Postal code", 32)?;
    }
    if let Some(ref phone) = payload.phone {
        validate_string_length(phone, "Phone", 64)?;
    }
    if let Some(ref vat_id) = payload.vat_id {
        validate_string_length(vat_id, "VAT ID", 32)?;
    }

    // Abuse prevention: block known disposable email domains
    let email_lower = payload.email.to_lowercase();
    let disposable_domains = [
        "@mailinator.com", "@tempmail.com", "@10minutemail.com", "@guerrillamail.com",
        "@trashmail.com", "@throwawaymail.com", "@yopmail.com", "@sharklasers.com",
        "@getairmail.com", "@temp-mail.org", "@dispostable.com"
    ];
    if disposable_domains.iter().any(|d| email_lower.ends_with(d)) {
        return Err(ApiError::BadRequest("Temporary or disposable email domains are not allowed".into()));
    }

    if payload.password.len() < 8 {
        return Err(ApiError::BadRequest("Password must be at least 8 characters".into()));
    }
    if payload.password.len() > 72 {
        return Err(ApiError::BadRequest("Password must be at most 72 bytes".into()));
    }

    let existing: Option<(Uuid,)> = sqlx::query_as(
        "SELECT id FROM customers WHERE LOWER(email) = LOWER($1) OR (username <> '' AND LOWER(username) = LOWER($2))"
    )
    .bind(payload.email.trim())
    .bind(payload.username.trim())
    .fetch_optional(&state.db)
    .await?;

    if existing.is_some() {
        return Err(ApiError::BadRequest("An account with this email or username already exists".into()));
    }

    tracing::info!("Registering customer directly: {}", payload.email);
    let customer_id = Uuid::now_v7();
    let external_id = format!("local_{}", customer_id);

    let pw_owned = payload.password.clone();
    let password_hash = tokio::task::spawn_blocking(move || hash(&pw_owned, DEFAULT_COST))
        .await
        .map_err(|e| ApiError::Internal(format!("Password hashing task failed: {e}")))?
        .map_err(|_| ApiError::Internal("Failed to hash password".into()))?;

    let postal_code = payload.postal_code.as_deref().unwrap_or("").trim();
    let mut token_bytes = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut token_bytes);
    let verification_token = hex::encode(token_bytes);

    sqlx::query(
        r#"INSERT INTO customers (id, external_id, username, name, email, phone, company, address, postal_code, city, country, vat_id, status, email_verified, password_hash, verification_token, verification_token_expires_at)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, 'active', false, $13, $14, now() + interval '24 hours')"#
    )
    .bind(customer_id)
    .bind(external_id)
    .bind(payload.username.trim())
    .bind(payload.name.trim())
    .bind(payload.email.trim())
    .bind(payload.phone.as_deref().unwrap_or(""))
    .bind(payload.company.as_deref().unwrap_or(""))
    .bind(payload.address.trim())
    .bind(postal_code)
    .bind(payload.city.trim())
    .bind(&normalized_country)
    .bind(payload.vat_id.as_deref().unwrap_or(""))
    .bind(password_hash)
    .bind(&verification_token)
    .execute(&state.db)
    .await?;

    crate::activity::spawn_record(
        state.db.clone(),
        Some(customer_id),
        vaultscope_db::models::ActorType::Customer,
        payload.name.clone(),
        "customer".to_string(),
        "registered".to_string(),
        payload.email.clone(),
        format!("Account registered for {}", payload.name),
    );

    send_verification_email(
        &state.config,
        Some(state.db.clone()),
        payload.email.trim(),
        payload.name.trim(),
        &verification_token,
    );

    Ok(Json(json!({
        "ok": true,
        "message": "Account created successfully. Please check your email and spam folder to verify your account."
    })))
}

fn dispatch_plain_email(
    config: &crate::config::Config,
    db: Option<sqlx::PgPool>,
    to_email: String,
    subject: String,
    body: String,
) {
    let smtp_host = config
        .smtp_host
        .clone()
        .unwrap_or_else(|| "mail.vaultscope.de".to_string());
    let smtp_user = config
        .smtp_username
        .clone()
        .unwrap_or_else(|| "system@vaultscope.de".to_string());
    let smtp_pass = config.smtp_password.clone().unwrap_or_default();
    let smtp_from = config
        .smtp_from
        .clone()
        .unwrap_or_else(|| "system@vaultscope.de".to_string());

    let from_mailbox = match smtp_from.parse() {
        Ok(m) => m,
        Err(e) => {
            tracing::error!("Invalid smtp_from address: {:?}", e);
            return;
        }
    };
    let to_mailbox = match to_email.parse() {
        Ok(m) => m,
        Err(e) => {
            tracing::error!("Invalid to_email address: {:?}", e);
            return;
        }
    };

    let safe_subject = crate::mailer::sanitize_email_header(&subject);
    let email = match lettre::Message::builder()
        .from(from_mailbox)
        .to(to_mailbox)
        .subject(safe_subject.clone())
        .body(body)
    {
        Ok(msg) => msg,
        Err(e) => {
            tracing::error!("Failed to build verification email: {:?}", e);
            return;
        }
    };

    if tokio::runtime::Handle::try_current().is_err() {
        return;
    }

    tokio::spawn(async move {
        let recipient_log = to_email.clone();
        let send_res = tokio::task::spawn_blocking(move || -> Result<(), String> {
            let creds =
                lettre::transport::smtp::authentication::Credentials::new(smtp_user, smtp_pass);
            let mailer = lettre::SmtpTransport::relay(&smtp_host)
                .map_err(|e| format!("Failed to create SMTP relay transport: {e:?}"))?
                .credentials(creds)
                .build();
            lettre::Transport::send(&mailer, &email).map_err(|e| e.to_string())?;
            Ok(())
        })
        .await;

        let (success, err_msg) = match send_res {
            Ok(Ok(())) => {
                tracing::info!("Verification email sent successfully to {}", recipient_log);
                (true, None)
            }
            Ok(Err(e)) => {
                tracing::error!("Could not send verification email to {}: {}", recipient_log, e);
                (false, Some(e))
            }
            Err(e) => {
                tracing::error!("Verification email task panicked for {}: {}", recipient_log, e);
                (false, Some(e.to_string()))
            }
        };

        if let Some(pool) = db {
            let status = if success {
                vaultscope_db::models::EmailDeliveryStatus::Delivered
            } else {
                vaultscope_db::models::EmailDeliveryStatus::Failed
            };
            let _ = sqlx::query(
                r#"INSERT INTO email_log (id, recipient, subject, template_id, status, error, sent_at)
                   VALUES ($1, $2, $3, $4, $5, $6, $7)"#,
            )
            .bind(Uuid::now_v7())
            .bind(to_email)
            .bind(subject)
            .bind(None::<Uuid>)
            .bind(status)
            .bind(err_msg)
            .bind(Utc::now())
            .execute(&pool)
            .await;
        }
    });
}

pub fn send_verification_email(
    config: &crate::config::Config,
    db: Option<sqlx::PgPool>,
    to_email: &str,
    to_name: &str,
    token: &str,
) {
    let base = config.storefront_url.trim_end_matches('/');
    let verify_link = format!("{}/verify-email?token={}", base, token);
    tracing::info!("Dispatching verification email to {}", to_email);

    let body = format!(
"Hello {},

Welcome to VaultScope!

Please confirm and verify your email address by clicking the link below:
{}

Important: If you don't see our emails in your inbox, please make sure to check your Spam or Junk folder.

If you did not sign up for a VaultScope account, please safely ignore this email.

Best regards,
The VaultScope Team",
        to_name, verify_link
    );

    dispatch_plain_email(
        config,
        db,
        to_email.to_string(),
        "Verify your VaultScope email address".to_string(),
        body,
    );
}

pub fn send_billing_email_verification(
    config: &crate::config::Config,
    db: Option<sqlx::PgPool>,
    to_email: &str,
    company_name: &str,
    token: &str,
) {
    let base = config.storefront_url.trim_end_matches('/');
    let verify_link = format!("{}/verify-email?token={}&type=billing", base, token);
    tracing::info!("Dispatching billing email verification to {}", to_email);

    let body = format!(
"Hello,

This email address has been added as a secondary Billing & Invoice CC recipient for {} on VaultScope.

To confirm this email address and begin receiving invoice copies and billing notices, please verify by clicking:
{}

Please check your Spam or Junk folder if this message was filtered.

If you did not authorize this, you can safely ignore this email.

Best regards,
The VaultScope Team",
        company_name, verify_link
    );

    dispatch_plain_email(
        config,
        db,
        to_email.to_string(),
        format!("Confirm Billing CC Email for {} - VaultScope", company_name),
        body,
    );
}

#[derive(Deserialize, ToSchema)]
pub(crate) struct VerifyEmailPayload {
    token: String,
}

#[utoipa::path(
    post,
    path = "/verify-email",
    tag = "Auth",
    request_body = VerifyEmailPayload,
    responses(
        (status = 200, description = "Email verified", body = OkMessageResponse),
        (status = 400, description = "Invalid or expired verification token", body = ErrorResponse)
    )
)]
pub(crate) async fn verify_email(
    State(state): State<AppState>,
    Json(payload): Json<VerifyEmailPayload>,
) -> Result<Json<Value>, ApiError> {
    validate_required_string(&payload.token, "Token")?;
    let token = payload.token.trim();

    // 1. Atomically verify and consume primary customer verification token
    let verified_customer: Option<(Uuid, String)> = sqlx::query_as(
        r#"UPDATE customers
           SET email_verified = true,
               verification_token = NULL,
               verification_token_expires_at = NULL,
               updated_at = now()
           WHERE verification_token = $1
             AND (verification_token_expires_at IS NULL OR verification_token_expires_at > now())
           RETURNING id, email"#,
    )
    .bind(token)
    .fetch_optional(&state.db)
    .await?;

    if let Some((customer_id, email)) = verified_customer {
        let _ = sqlx::query(
            "UPDATE staff SET email_verified = true, updated_at = now() WHERE LOWER(email) = LOWER($1) OR external_id = $2",
        )
        .bind(&email)
        .bind(format!("customer_{customer_id}"))
        .execute(&state.db)
        .await;

        tracing::info!("Customer primary email verified successfully: {}", email);

        return Ok(Json(json!({
            "ok": true,
            "message": "Account email address verified successfully."
        })));
    }

    // 2. Atomically verify and consume billing CC verification token
    let verified_billing: Option<(Uuid, String)> = sqlx::query_as(
        r#"UPDATE customer_billing_emails
           SET verified = true,
               verification_token = NULL,
               verification_token_expires_at = NULL,
               verified_at = now()
           WHERE verification_token = $1
             AND (verification_token_expires_at IS NULL OR verification_token_expires_at > now())
           RETURNING id, email"#,
    )
    .bind(token)
    .fetch_optional(&state.db)
    .await?;

    if let Some((_id, email)) = verified_billing {
        tracing::info!("Customer billing CC email verified successfully: {}", email);

        return Ok(Json(json!({
            "ok": true,
            "message": "Billing CC email verified successfully. Invoices will now be sent to this address."
        })));
    }

    Err(ApiError::BadRequest(
        "Invalid or expired verification token.".into(),
    ))
}

#[derive(Deserialize, ToSchema)]
pub(crate) struct ResendVerificationPayload {
    email: String,
}

#[utoipa::path(
    post,
    path = "/resend-verification",
    tag = "Auth",
    request_body = ResendVerificationPayload,
    responses(
        (status = 200, description = "Verification email resent if account exists and is unverified", body = OkMessageResponse),
        (status = 400, description = "Invalid email format", body = ErrorResponse)
    )
)]
pub(crate) async fn resend_verification(
    State(state): State<AppState>,
    Json(payload): Json<ResendVerificationPayload>,
) -> Result<Json<Value>, ApiError> {
    validate_required_string(&payload.email, "Email")?;
    validate_email(&payload.email)?;

    let customer: Option<(Uuid, String, String, bool)> = sqlx::query_as(
        "SELECT id, name, email, email_verified FROM customers WHERE LOWER(email) = LOWER($1) LIMIT 1",
    )
    .bind(payload.email.trim())
    .fetch_optional(&state.db)
    .await?;

    if let Some((customer_id, name, email, email_verified)) = customer {
        if !email_verified {
            let mut token_bytes = [0u8; 32];
            rand::rngs::OsRng.fill_bytes(&mut token_bytes);
            let verification_token = hex::encode(token_bytes);

            sqlx::query(
                "UPDATE customers SET verification_token = $1, verification_token_expires_at = now() + interval '24 hours', updated_at = now() WHERE id = $2",
            )
            .bind(&verification_token)
            .bind(customer_id)
            .execute(&state.db)
            .await?;

            send_verification_email(
                &state.config,
                Some(state.db.clone()),
                &email,
                &name,
                &verification_token,
            );
        }
    }

    Ok(Json(json!({
        "ok": true,
        "message": "If your account exists and is unverified, a verification link has been sent. Please check your inbox and spam folder."
    })))
}
