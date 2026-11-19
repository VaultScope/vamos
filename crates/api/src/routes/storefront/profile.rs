use axum::extract::{Path, State};
use axum::Json;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use utoipa::ToSchema;
use utoipa_axum::{router::OpenApiRouter, routes};
use uuid::Uuid;

use crate::auth::AuthCustomer;
use crate::error::ApiError;
use crate::openapi::{ErrorResponse, OkMessageResponse, OkResponse};
use crate::state::AppState;
use vaultscope_db::models::{Customer, CustomerBillingEmail, InvoiceBillingSnapshot};
use totp_rs::{Algorithm, Secret, TOTP};

pub fn routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(get_profile, update_profile, delete_own_account))
        .routes(routes!(list_billing_emails, add_billing_email))
        .routes(routes!(delete_billing_email))
        .routes(routes!(resend_billing_email_verification))
        .routes(routes!(setup_mfa))
        .routes(routes!(enable_mfa))
        .routes(routes!(disable_mfa))
        .routes(routes!(verify_vat))
}

#[utoipa::path(
    get,
    path = "/",
    tag = "Storefront Profile",
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "Customer profile details", body = ProfileResponse),
        (status = 404, description = "Customer not found", body = ErrorResponse)
    )
)]
pub(crate) async fn get_profile(
    auth: AuthCustomer,
    State(state): State<AppState>,
) -> Result<Json<ProfileResponse>, ApiError> {
    let customer = sqlx::query_as::<_, Customer>("SELECT * FROM customers WHERE id = $1")
        .bind(auth.0.sub)
        .fetch_optional(&state.db)
        .await?
        .ok_or(ApiError::NotFound("customer not found".into()))?;

    Ok(Json(ProfileResponse {
        id: customer.id,
        username: customer.username,
        name: customer.name,
        email: customer.email,
        phone: customer.phone,
        company: customer.company,
        address: customer.address,
        postal_code: customer.postal_code,
        city: customer.city,
        country: customer.country,
        vat_id: customer.vat_id,
        vat_verified: customer.vat_verified,
        two_factor_enabled: customer.two_factor_enabled,
        email_verified: customer.email_verified,
        created_at: customer.created_at,
        updated_at: customer.updated_at,
        last_login: customer.last_login,
    }))
}

/// A filtered response that does NOT expose internal admin fields
/// like notes, stripe_customer_id, or status.
#[derive(Serialize, ToSchema)]
pub(crate) struct ProfileResponse {
    id: uuid::Uuid,
    username: String,
    name: String,
    email: String,
    phone: String,
    company: String,
    address: String,
    postal_code: String,
    city: String,
    country: String,
    vat_id: String,
    vat_verified: bool,
    two_factor_enabled: bool,
    email_verified: bool,
    created_at: chrono::DateTime<chrono::Utc>,
    updated_at: chrono::DateTime<chrono::Utc>,
    last_login: Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(Deserialize, ToSchema)]
pub(crate) struct UpdateProfile {
    username: Option<String>,
    name: Option<String>,
    phone: Option<String>,
    company: Option<String>,
    address: Option<String>,
    postal_code: Option<String>,
    city: Option<String>,
    country: Option<String>,
    vat_id: Option<String>,
    #[serde(default)]
    #[schema(read_only)]
    vat_verified: Option<bool>,
}

#[utoipa::path(
    put,
    path = "/",
    tag = "Storefront Profile",
    security(("bearer_auth" = []), ("csrf_token" = [])),
    request_body = UpdateProfile,
    responses(
        (status = 200, description = "Updated customer profile", body = ProfileResponse),
        (status = 400, description = "Validation error", body = ErrorResponse)
    )
)]
pub(crate) async fn update_profile(
    auth: AuthCustomer,
    State(state): State<AppState>,
    Json(payload): Json<UpdateProfile>,
) -> Result<Json<ProfileResponse>, ApiError> {
    if payload.vat_verified.is_some() {
        return Err(ApiError::BadRequest(
            "vat_verified cannot be updated directly; verify via /verify-vat".into(),
        ));
    }

    // Validate username if provided
    let trimmed_username = match payload.username {
        Some(ref username) => {
            let trimmed = username.trim();
            if trimmed.is_empty() {
                return Err(ApiError::BadRequest("Username cannot be empty".into()));
            }
            if trimmed.len() > 150 {
                return Err(ApiError::BadRequest(
                    "Username exceeds maximum length of 150 characters".into(),
                ));
            }
            Some(trimmed.to_string())
        }
        None => None,
    };

    // Validate name if provided
    let trimmed_name = match payload.name {
        Some(ref name) => {
            let trimmed = name.trim();
            if trimmed.is_empty() {
                return Err(ApiError::BadRequest("Name cannot be empty".into()));
            }
            if trimmed.len() > 255 {
                return Err(ApiError::BadRequest(
                    "Name exceeds maximum length of 255 characters".into(),
                ));
            }
            Some(trimmed.to_string())
        }
        None => None,
    };

    let normalized_vat_id = payload
        .vat_id
        .map(|v| v.trim().to_uppercase().replace([' ', '.', '-'], ""));

    let customer: Customer = sqlx::query_as(
        r#"UPDATE customers
           SET name = COALESCE($1, name),
               phone = COALESCE($2, phone),
               company = COALESCE($3, company),
               address = COALESCE($4, address),
               postal_code = COALESCE($5, postal_code),
               city = COALESCE($6, city),
               country = COALESCE($7, country),
               vat_id = COALESCE($8, vat_id),
               vat_verified = CASE
                   WHEN $8 IS NOT NULL AND $8 IS DISTINCT FROM vat_id THEN false
                   ELSE vat_verified
               END,
               username = COALESCE($9, username),
               updated_at = now()
           WHERE id = $10
           RETURNING *"#,
    )
    .bind(trimmed_name)
    .bind(payload.phone)
    .bind(payload.company)
    .bind(payload.address)
    .bind(payload.postal_code)
    .bind(payload.city)
    .bind(payload.country)
    .bind(normalized_vat_id)
    .bind(trimmed_username)
    .bind(auth.0.sub)
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound("customer not found".into()))?;

    crate::activity::spawn_record(
        state.db.clone(),
        Some(auth.0.sub),
        vaultscope_db::models::ActorType::Customer,
        customer.name.clone(),
        "account".to_string(),
        "profile_updated".to_string(),
        customer.email.clone(),
        "Updated contact and account profile details".to_string(),
    );

    Ok(Json(ProfileResponse {
        id: customer.id,
        username: customer.username,
        name: customer.name,
        email: customer.email,
        phone: customer.phone,
        company: customer.company,
        address: customer.address,
        postal_code: customer.postal_code,
        city: customer.city,
        country: customer.country,
        vat_id: customer.vat_id,
        vat_verified: customer.vat_verified,
        two_factor_enabled: customer.two_factor_enabled,
        email_verified: customer.email_verified,
        created_at: customer.created_at,
        updated_at: customer.updated_at,
        last_login: customer.last_login,
    }))
}

#[derive(Deserialize, ToSchema)]
pub struct AddBillingEmailPayload {
    pub email: String,
}

#[utoipa::path(
    get,
    path = "/billing-emails",
    tag = "Storefront Profile",
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "List customer billing CC email addresses", body = Vec<CustomerBillingEmail>)
    )
)]
pub(crate) async fn list_billing_emails(
    auth: AuthCustomer,
    State(state): State<AppState>,
) -> Result<Json<Vec<CustomerBillingEmail>>, ApiError> {
    let emails = sqlx::query_as::<_, CustomerBillingEmail>(
        "SELECT * FROM customer_billing_emails WHERE customer_id = $1 ORDER BY created_at ASC"
    )
    .bind(auth.0.sub)
    .fetch_all(&state.db)
    .await?;

    Ok(Json(emails))
}

#[utoipa::path(
    post,
    path = "/billing-emails",
    tag = "Storefront Profile",
    security(("bearer_auth" = []), ("csrf_token" = [])),
    request_body = AddBillingEmailPayload,
    responses(
        (status = 200, description = "Added billing CC email address", body = CustomerBillingEmail),
        (status = 400, description = "Invalid or disallowed billing email", body = ErrorResponse)
    )
)]
pub(crate) async fn add_billing_email(
    auth: AuthCustomer,
    State(state): State<AppState>,
    Json(payload): Json<AddBillingEmailPayload>,
) -> Result<Json<CustomerBillingEmail>, ApiError> {
    let email = payload.email.trim().to_lowercase();
    crate::validation::validate_email(&email)?;

    // Security Check 1: Customer must be a business customer with non-empty company
    let customer = sqlx::query_as::<_, Customer>("SELECT * FROM customers WHERE id = $1")
        .bind(auth.0.sub)
        .fetch_optional(&state.db)
        .await?
        .ok_or(ApiError::NotFound("customer not found".into()))?;

    if customer.company.trim().is_empty() {
        return Err(ApiError::BadRequest(
            "Additional billing email recipients are only available for business customers. Please add a company name to your profile first.".into(),
        ));
    }

    // Security Check 2: Cannot duplicate primary email
    if email == customer.email.to_lowercase() {
        return Err(ApiError::BadRequest(
            "This email address is already your primary account email.".into(),
        ));
    }

    // Security Check 3: Abuse prevention - limit to max 5 billing emails per account
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM customer_billing_emails WHERE customer_id = $1"
    )
    .bind(auth.0.sub)
    .fetch_one(&state.db)
    .await?;

    if count >= 5 {
        return Err(ApiError::BadRequest(
            "Maximum limit of 5 billing email recipients reached.".into(),
        ));
    }

    // Security Check 4: Disposable email domains blocking
    let disposable_domains = [
        "@mailinator.com", "@tempmail.com", "@10minutemail.com", "@guerrillamail.com",
        "@trashmail.com", "@throwawaymail.com", "@yopmail.com", "@sharklasers.com",
        "@getairmail.com", "@temp-mail.org", "@dispostable.com"
    ];
    if disposable_domains.iter().any(|d| email.ends_with(d)) {
        return Err(ApiError::BadRequest(
            "Temporary or disposable email domains are not allowed".into(),
        ));
    }

    let token_bytes: [u8; 32] = rand::random();
    let token = hex::encode(token_bytes);
    let id = Uuid::now_v7();

    let billing_email = sqlx::query_as::<_, CustomerBillingEmail>(
        r#"INSERT INTO customer_billing_emails (id, customer_id, email, verified, verification_token, verification_token_expires_at)
           VALUES ($1, $2, $3, false, $4, now() + interval '24 hours')
           ON CONFLICT (customer_id, email) DO UPDATE
           SET verification_token = EXCLUDED.verification_token,
               verification_token_expires_at = EXCLUDED.verification_token_expires_at
           RETURNING *"#
    )
    .bind(id)
    .bind(auth.0.sub)
    .bind(&email)
    .bind(&token)
    .fetch_one(&state.db)
    .await?;

    crate::routes::auth::send_billing_email_verification(
        &state.config,
        Some(state.db.clone()),
        &email,
        &customer.company,
        &token,
    );

    Ok(Json(billing_email))
}

#[utoipa::path(
    delete,
    path = "/billing-emails/{id}",
    tag = "Storefront Profile",
    security(("bearer_auth" = []), ("csrf_token" = [])),
    params(("id" = Uuid, Path, description = "Billing email ID")),
    responses(
        (status = 200, description = "Billing email removed", body = OkResponse),
        (status = 404, description = "Billing email not found", body = ErrorResponse)
    )
)]
pub(crate) async fn delete_billing_email(
    auth: AuthCustomer,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let rows = sqlx::query("DELETE FROM customer_billing_emails WHERE id = $1 AND customer_id = $2")
        .bind(id)
        .bind(auth.0.sub)
        .execute(&state.db)
        .await?
        .rows_affected();

    if rows == 0 {
        return Err(ApiError::NotFound("Billing email not found".into()));
    }

    Ok(Json(serde_json::json!({ "ok": true })))
}

#[utoipa::path(
    post,
    path = "/billing-emails/{id}/resend",
    tag = "Storefront Profile",
    security(("bearer_auth" = []), ("csrf_token" = [])),
    params(("id" = Uuid, Path, description = "Billing email ID")),
    responses(
        (status = 200, description = "Verification email resent", body = OkMessageResponse),
        (status = 404, description = "Billing email not found", body = ErrorResponse)
    )
)]
pub(crate) async fn resend_billing_email_verification(
    auth: AuthCustomer,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let billing_email = sqlx::query_as::<_, CustomerBillingEmail>(
        "SELECT * FROM customer_billing_emails WHERE id = $1 AND customer_id = $2"
    )
    .bind(id)
    .bind(auth.0.sub)
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound("Billing email not found".into()))?;

    if billing_email.verified {
        return Ok(Json(serde_json::json!({
            "ok": true,
            "message": "This billing email is already verified."
        })));
    }

    let customer = sqlx::query_as::<_, Customer>("SELECT * FROM customers WHERE id = $1")
        .bind(auth.0.sub)
        .fetch_optional(&state.db)
        .await?
        .ok_or(ApiError::NotFound("customer not found".into()))?;

    let token_bytes: [u8; 32] = rand::random();
    let token = hex::encode(token_bytes);

    sqlx::query(
        "UPDATE customer_billing_emails SET verification_token = $1, verification_token_expires_at = now() + interval '24 hours' WHERE id = $2",
    )
        .bind(&token)
        .bind(id)
        .execute(&state.db)
        .await?;

    crate::routes::auth::send_billing_email_verification(
        &state.config,
        Some(state.db.clone()),
        &billing_email.email,
        &customer.company,
        &token,
    );

    Ok(Json(serde_json::json!({
        "ok": true,
        "message": "Verification email resent. Please check the recipient inbox and spam folder."
    })))
}

#[derive(Deserialize, ToSchema)]
pub struct MfaEnablePayload {
    pub code: String,
}

#[derive(Deserialize, ToSchema)]
pub struct MfaDisablePayload {
    pub code: Option<String>,
    pub password: Option<String>,
}

#[derive(Deserialize, ToSchema)]
pub struct VerifyVatPayload {
    pub vat_id: String,
}

#[allow(dead_code)]
#[derive(Serialize, ToSchema)]
pub struct MfaSetupResponse {
    pub secret: String,
    pub qr_url: String,
    pub qr_base64: String,
    pub url: String,
}

#[allow(dead_code)]
#[derive(Serialize, ToSchema)]
pub struct MfaEnableResponse {
    pub ok: bool,
    pub message: String,
    pub recovery_codes: Vec<String>,
}

#[allow(dead_code)]
#[derive(Serialize, ToSchema)]
pub struct VerifyVatResponse {
    pub valid: bool,
    pub vat_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub company_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub address: Option<String>,
    pub message: String,
}

#[utoipa::path(
    post,
    path = "/mfa/setup",
    tag = "Storefront Profile",
    security(("bearer_auth" = []), ("csrf_token" = [])),
    responses(
        (status = 200, description = "Generated TOTP secret and QR code payload", body = MfaSetupResponse),
        (status = 400, description = "2FA already enabled", body = ErrorResponse)
    )
)]
pub(crate) async fn setup_mfa(
    auth: AuthCustomer,
    State(state): State<AppState>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let customer = sqlx::query_as::<_, Customer>("SELECT * FROM customers WHERE id = $1")
        .bind(auth.0.sub)
        .fetch_optional(&state.db)
        .await?
        .ok_or(ApiError::NotFound("Customer not found".into()))?;

    if customer.two_factor_enabled {
        return Err(ApiError::BadRequest(
            "Two-factor authentication is already enabled. Disable it first before reconfiguring.".into(),
        ));
    }

    let secret = Secret::generate_secret();
    let secret_base32 = secret.to_encoded().to_string();

    let totp = TOTP::new(
        Algorithm::SHA1,
        6,
        1,
        30,
        secret.to_bytes().map_err(|e| ApiError::Internal(format!("Secret generation error: {e}")))?,
        Some("VaultScope".to_string()),
        customer.email.clone(),
    )
    .map_err(|e| ApiError::Internal(format!("TOTP init error: {e}")))?;

    let qr_base64 = totp.get_qr_base64().map_err(|e| ApiError::Internal(format!("QR generation error: {e}")))?;
    let qr_url = format!("data:image/png;base64,{}", qr_base64);
    let url = totp.get_url();

    // Store encrypted secret on customer
    let encrypted_secret =
        crate::routes::auth::encrypt_totp_secret(&state.config.encryption_key_bytes(), &secret_base32)?;
    sqlx::query("UPDATE customers SET totp_secret = $1 WHERE id = $2")
        .bind(&encrypted_secret)
        .bind(auth.0.sub)
        .execute(&state.db)
        .await?;

    Ok(Json(serde_json::json!({
        "secret": secret_base32,
        "qr_url": qr_url,
        "qr_base64": qr_base64,
        "url": url,
    })))
}

fn normalize_recovery_code(raw: &str) -> String {
    raw.trim().replace(['-', ' '], "").to_uppercase()
}

fn hash_recovery_code(normalized: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(normalized.as_bytes());
    hex::encode(hasher.finalize())
}

/// Generates `count` single-use recovery codes with 80 bits (10 CSPRNG bytes) of
/// entropy each, returning `(formatted_code, sha256_hash)` pairs.
pub fn generate_recovery_codes(count: usize) -> Vec<(String, String)> {
    let mut out = Vec::with_capacity(count);
    for _ in 0..count {
        let mut bytes = [0u8; 10];
        rand::rngs::OsRng.fill_bytes(&mut bytes);
        let hex_str = hex::encode_upper(bytes);
        let formatted = format!(
            "{}-{}-{}-{}-{}",
            &hex_str[0..4],
            &hex_str[4..8],
            &hex_str[8..12],
            &hex_str[12..16],
            &hex_str[16..20]
        );
        let normalized = normalize_recovery_code(&formatted);
        let hash = hash_recovery_code(&normalized);
        out.push((formatted, hash));
    }
    out
}

/// Validates and parses an EU VAT ID strictly in ASCII before slicing to prevent
/// UTF-8 char-boundary panics and VIES URL path traversal.
pub fn parse_and_validate_vat_id(input: &str) -> Result<(String, String, String), ApiError> {
    let cleaned: String = input
        .trim()
        .chars()
        .filter(|c| !matches!(c, ' ' | '.' | '-'))
        .collect();

    if !cleaned.is_ascii() {
        return Err(ApiError::BadRequest(
            "VAT ID must contain only ASCII characters (e.g. DE123456789).".into(),
        ));
    }

    let normalized = cleaned.to_ascii_uppercase();
    if normalized.len() < 4 || normalized.len() > 17 {
        return Err(ApiError::BadRequest(
            "VAT ID must be between 4 and 17 characters (2-letter country code + 2..15 alphanumeric characters).".into(),
        ));
    }

    let (country_code, vat_number) = normalized.split_at(2);
    if !country_code.bytes().all(|b| b.is_ascii_alphabetic()) {
        return Err(ApiError::BadRequest(
            "VAT ID must start with a 2-letter ASCII country code (e.g. DE).".into(),
        ));
    }

    if !(2..=15).contains(&vat_number.len())
        || !vat_number.bytes().all(|b| b.is_ascii_alphanumeric())
    {
        return Err(ApiError::BadRequest(
            "VAT number must be 2 to 15 ASCII alphanumeric characters.".into(),
        ));
    }

    let cc = country_code.to_string();
    let num = vat_number.to_string();
    Ok((normalized, cc, num))
}

#[utoipa::path(
    post,
    path = "/mfa/enable",
    tag = "Storefront Profile",
    security(("bearer_auth" = []), ("csrf_token" = [])),
    request_body = MfaEnablePayload,
    responses(
        (status = 200, description = "2FA enabled with single-use recovery codes", body = MfaEnableResponse),
        (status = 400, description = "Invalid verification code or setup not initiated", body = ErrorResponse)
    )
)]
pub(crate) async fn enable_mfa(
    auth: AuthCustomer,
    State(state): State<AppState>,
    Json(payload): Json<MfaEnablePayload>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let customer = sqlx::query_as::<_, Customer>("SELECT * FROM customers WHERE id = $1")
        .bind(auth.0.sub)
        .fetch_optional(&state.db)
        .await?
        .ok_or(ApiError::NotFound("Customer not found".into()))?;

    if customer.two_factor_enabled {
        return Err(ApiError::BadRequest(
            "Two-factor authentication is already enabled.".into(),
        ));
    }

    let secret_stored = customer.totp_secret.ok_or_else(|| {
        ApiError::BadRequest("MFA setup was not initiated. Please generate a setup key first.".into())
    })?;
    let secret_str =
        crate::routes::auth::decrypt_totp_secret(&state.config.encryption_key_bytes(), &secret_stored)?;
    let encrypted_secret =
        crate::routes::auth::encrypt_totp_secret(&state.config.encryption_key_bytes(), &secret_str)?;

    let Some(clean_code) =
        crate::routes::auth::verify_totp_code(&secret_str, &customer.email, &payload.code)?
    else {
        return Err(ApiError::BadRequest("Invalid 6-digit verification code. Please check your authenticator app and system clock.".into()));
    };

    if !crate::routes::auth::check_and_mark_totp_used(&state, auth.0.sub, &clean_code).await {
        return Err(ApiError::BadRequest(
            "This verification code was already used. Please wait for the next 30-second code.".into(),
        ));
    }

    let pairs = generate_recovery_codes(8);
    let recovery_codes: Vec<String> = pairs.iter().map(|(code, _)| code.clone()).collect();
    let code_hashes: Vec<String> = pairs.into_iter().map(|(_, hash)| hash).collect();

    let mut tx = state.db.begin().await?;

    sqlx::query(
        "UPDATE customers SET two_factor_enabled = true, totp_secret = $2, auth_version = auth_version + 1, updated_at = now() WHERE id = $1",
    )
    .bind(auth.0.sub)
    .bind(&encrypted_secret)
    .execute(&mut *tx)
    .await?;

    sqlx::query("DELETE FROM customer_mfa_recovery_codes WHERE customer_id = $1")
        .bind(auth.0.sub)
        .execute(&mut *tx)
        .await?;

    for code_hash in &code_hashes {
        sqlx::query(
            "INSERT INTO customer_mfa_recovery_codes (id, customer_id, code_hash) VALUES ($1, $2, $3)",
        )
        .bind(Uuid::now_v7())
        .bind(auth.0.sub)
        .bind(code_hash)
        .execute(&mut *tx)
        .await?;
    }

    tx.commit().await?;

    crate::activity::spawn_record(
        state.db.clone(),
        Some(auth.0.sub),
        vaultscope_db::models::ActorType::Customer,
        customer.name.clone(),
        "security".to_string(),
        "2fa_enabled".to_string(),
        "TOTP Authenticator".to_string(),
        "Two-factor authentication (TOTP) successfully enabled".to_string(),
    );

    Ok(Json(serde_json::json!({
        "ok": true,
        "message": "Two-factor authentication successfully activated.",
        "recovery_codes": recovery_codes,
    })))
}

#[utoipa::path(
    post,
    path = "/mfa/disable",
    tag = "Storefront Profile",
    security(("bearer_auth" = []), ("csrf_token" = [])),
    request_body = MfaDisablePayload,
    responses(
        (status = 200, description = "2FA disabled", body = OkMessageResponse),
        (status = 400, description = "Invalid password, TOTP code, or recovery code", body = ErrorResponse)
    )
)]
pub(crate) async fn disable_mfa(
    auth: AuthCustomer,
    State(state): State<AppState>,
    Json(payload): Json<MfaDisablePayload>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let customer = sqlx::query_as::<_, Customer>("SELECT * FROM customers WHERE id = $1")
        .bind(auth.0.sub)
        .fetch_optional(&state.db)
        .await?
        .ok_or(ApiError::NotFound("Customer not found".into()))?;

    if !customer.two_factor_enabled {
        return Err(ApiError::BadRequest(
            "Two-factor authentication is not currently enabled.".into(),
        ));
    }

    // Require password verification whenever the account has a local password
    // (prevents session-only hijack; password alone is never sufficient).
    if let Some(ref hash_str) = customer.password_hash {
        let pw = payload
            .password
            .as_deref()
            .filter(|s| !s.is_empty())
            .ok_or_else(|| {
                ApiError::BadRequest(
                    "Current password and a valid 2FA or recovery code are required to disable two-factor authentication.".into(),
                )
            })?
            .to_string();
        let hash_owned = hash_str.clone();
        let pw_valid = tokio::task::spawn_blocking(move || bcrypt::verify(&pw, &hash_owned))
            .await
            .map_err(|e| ApiError::Internal(format!("Password verification task failed: {e}")))?
            .unwrap_or(false);
        if !pw_valid {
            return Err(ApiError::BadRequest(
                "Invalid current password, 2FA code, or recovery code to disable two-factor authentication.".into(),
            ));
        }
    }

    let raw_code = payload
        .code
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            ApiError::BadRequest(
                "A valid 2FA code or unused recovery code is required to disable two-factor authentication.".into(),
            )
        })?;

    let mut code_authorized = false;
    let mut matched_recovery_id: Option<Uuid> = None;

    // 1. Verify TOTP code
    if let Some(ref secret_stored) = customer.totp_secret {
        if let Ok(secret_str) =
            crate::routes::auth::decrypt_totp_secret(&state.config.encryption_key_bytes(), secret_stored)
        {
            if let Ok(Some(clean_code)) =
                crate::routes::auth::verify_totp_code(&secret_str, &customer.email, raw_code)
            {
                if crate::routes::auth::check_and_mark_totp_used(&state, auth.0.sub, &clean_code).await {
                    code_authorized = true;
                }
            }
        }
    }

    // 2. Or verify an unused single-use recovery code
    if !code_authorized {
        let normalized = normalize_recovery_code(raw_code);
        if !normalized.is_empty() {
            let code_hash = hash_recovery_code(&normalized);
            matched_recovery_id = sqlx::query_scalar(
                "SELECT id FROM customer_mfa_recovery_codes WHERE customer_id = $1 AND code_hash = $2 AND used_at IS NULL LIMIT 1",
            )
            .bind(auth.0.sub)
            .bind(&code_hash)
            .fetch_optional(&state.db)
            .await?;

            if matched_recovery_id.is_some() {
                code_authorized = true;
            }
        }
    }

    if !code_authorized {
        return Err(ApiError::BadRequest(
            "Invalid account password, 2FA code, or recovery code to disable two-factor authentication.".into(),
        ));
    }

    let mut tx = state.db.begin().await?;

    if let Some(rc_id) = matched_recovery_id {
        let consumed = sqlx::query(
            "UPDATE customer_mfa_recovery_codes SET used_at = now() WHERE id = $1 AND used_at IS NULL",
        )
        .bind(rc_id)
        .execute(&mut *tx)
        .await?;
        if consumed.rows_affected() == 0 {
            return Err(ApiError::BadRequest(
                "Recovery code has already been used.".into(),
            ));
        }
    }

    sqlx::query(
        "UPDATE customers SET two_factor_enabled = false, totp_secret = NULL, auth_version = auth_version + 1, updated_at = now() WHERE id = $1",
    )
    .bind(auth.0.sub)
    .execute(&mut *tx)
    .await?;

    sqlx::query("DELETE FROM customer_mfa_recovery_codes WHERE customer_id = $1")
        .bind(auth.0.sub)
        .execute(&mut *tx)
        .await?;

    tx.commit().await?;

    crate::activity::spawn_record(
        state.db.clone(),
        Some(auth.0.sub),
        vaultscope_db::models::ActorType::Customer,
        customer.name.clone(),
        "security".to_string(),
        "2fa_disabled".to_string(),
        "TOTP Authenticator".to_string(),
        "Two-factor authentication disabled".to_string(),
    );

    Ok(Json(serde_json::json!({
        "ok": true,
        "message": "Two-factor authentication has been disabled."
    })))
}

#[utoipa::path(
    post,
    path = "/verify-vat",
    tag = "Storefront Profile",
    security(("bearer_auth" = []), ("csrf_token" = [])),
    request_body = VerifyVatPayload,
    responses(
        (status = 200, description = "EU VIES VAT verification result", body = VerifyVatResponse),
        (status = 400, description = "Malformed VAT ID", body = ErrorResponse)
    )
)]
pub(crate) async fn verify_vat(
    auth: AuthCustomer,
    State(state): State<AppState>,
    Json(payload): Json<VerifyVatPayload>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let (raw_vat, country_code, vat_number) = parse_and_validate_vat_id(&payload.vat_id)?;

    let url = format!(
        "https://ec.europa.eu/taxation_customs/vies/rest-api/ms/{}/vat/{}",
        country_code, vat_number
    );

    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(std::time::Duration::from_secs(8))
        .build()
        .map_err(|e| {
            tracing::error!("Failed to build VIES HTTP client: {}", e);
            ApiError::Internal("Failed to initialize VAT verification service.".into())
        })?;

    let resp = client
        .get(&url)
        .header("User-Agent", "VaultScope-VIES-Validator/1.0")
        .send()
        .await;

    match resp {
        Ok(res) if res.status().is_success() => {
            let data: serde_json::Value = res.json().await.unwrap_or_default();
            let is_valid = data.get("isValid").and_then(|v| v.as_bool()).unwrap_or(false);
            let name = data.get("name").and_then(|v| v.as_str()).unwrap_or("---").trim().to_string();
            let address = data.get("address").and_then(|v| v.as_str()).unwrap_or("---").trim().to_string();

            if is_valid {
                sqlx::query("UPDATE customers SET vat_id = $1, vat_verified = true, updated_at = now() WHERE id = $2")
                    .bind(&raw_vat)
                    .bind(auth.0.sub)
                    .execute(&state.db)
                    .await?;

                Ok(Json(serde_json::json!({
                    "valid": true,
                    "vat_id": raw_vat,
                    "company_name": name,
                    "address": address,
                    "message": "VAT ID verified successfully via EU VIES registry."
                })))
            } else {
                sqlx::query("UPDATE customers SET vat_verified = false, updated_at = now() WHERE id = $1")
                    .bind(auth.0.sub)
                    .execute(&state.db)
                    .await?;

                Ok(Json(serde_json::json!({
                    "valid": false,
                    "vat_id": raw_vat,
                    "message": "VAT ID was not found or is invalid in the EU VIES registry."
                })))
            }
        }
        Ok(res) => {
            let status = res.status();
            tracing::warn!("VIES API returned HTTP status {}", status);
            Ok(Json(serde_json::json!({
                "valid": false,
                "vat_id": raw_vat,
                "message": "EU VIES service is temporarily unavailable. Please try again shortly."
            })))
        }
        Err(e) => {
            tracing::warn!("VIES API request failed: {}", e);
            Ok(Json(serde_json::json!({
                "valid": false,
                "vat_id": raw_vat,
                "message": "EU VIES service is currently unreachable. Please try again shortly."
            })))
        }
    }
}

#[utoipa::path(
    delete,
    path = "/",
    tag = "Storefront Profile",
    security(("bearer_auth" = []), ("csrf_token" = [])),
    responses(
        (status = 200, description = "Customer account deleted", body = OkMessageResponse),
        (status = 400, description = "Active services prevent account deletion", body = ErrorResponse)
    )
)]
pub(crate) async fn delete_own_account(
    auth: AuthCustomer,
    State(state): State<AppState>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let customer_id = auth.0.sub;
    let mut tx = state.db.begin().await?;

    let customer: Customer = sqlx::query_as("SELECT * FROM customers WHERE id = $1 FOR UPDATE")
        .bind(customer_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(|| ApiError::NotFound("Customer not found".into()))?;

    let active_services: (i64,) = sqlx::query_as(
        "SELECT count(*) FROM services WHERE customer_id = $1 AND status IN ('running'::service_status, 'pending'::service_status)",
    )
    .bind(customer_id)
    .fetch_one(&mut *tx)
    .await?;

    if active_services.0 > 0 {
        return Err(ApiError::BadRequest(
            "Cannot delete account while you have active running or pending services. Please terminate your services first.".into(),
        ));
    }

    // Bump monotonic auth_version immediately inside the transaction
    sqlx::query(
        "UPDATE customers SET auth_version = auth_version + 1, lifecycle_state = 'deletion_pending', deletion_requested_at = now(), updated_at = now() WHERE id = $1",
    )
    .bind(customer_id)
    .execute(&mut *tx)
    .await?;

    // Queue provider teardown jobs for any provisioned services not already pending deletion
    let provisioned_services: Vec<(Uuid, String, String)> = sqlx::query_as(
        r#"SELECT s.id, s.provider_resource_id, COALESCE(NULLIF(s.config->>'provider', ''), p.provider)
           FROM services s
           JOIN products p ON p.id = s.product_id
           WHERE s.customer_id = $1
             AND s.provider_resource_id <> ''
             AND NOT EXISTS (
                 SELECT 1 FROM jobs j
                 WHERE j.service_id = s.id
                   AND j.task = 'delete'
                   AND j.status IN ('queued'::job_status, 'in_progress'::job_status)
             )"#,
    )
    .bind(customer_id)
    .fetch_all(&mut *tx)
    .await?;

    for (svc_id, resource_id, target_api) in provisioned_services {
        let delete_payload = serde_json::json!({
            "resource_id": resource_id,
            "service_id": svc_id,
            "customer_id": customer_id,
        });
        sqlx::query(
            "INSERT INTO jobs (id, task, target_api, customer_id, service_id, request_payload) VALUES ($1, 'delete', $2, NULL, NULL, $3)",
        )
        .bind(Uuid::now_v7())
        .bind(&target_api)
        .bind(&delete_payload)
        .execute(&mut *tx)
        .await?;
    }

    // Delete only unissued draft invoices not under statutory retention hold
    sqlx::query(
        r#"DELETE FROM invoice_line_items
           WHERE invoice_id IN (
               SELECT id FROM invoices
               WHERE customer_id = $1
                 AND status = 'draft'::invoice_status
                 AND retention_hold = false
                 AND NOT EXISTS (SELECT 1 FROM invoices c WHERE c.corrects_invoice_id = invoices.id)
           )"#,
    )
    .bind(customer_id)
    .execute(&mut *tx)
    .await?;

    sqlx::query(
        r#"DELETE FROM invoices
           WHERE customer_id = $1
             AND status = 'draft'::invoice_status
             AND retention_hold = false
             AND NOT EXISTS (SELECT 1 FROM invoices c WHERE c.corrects_invoice_id = invoices.id)"#,
    )
    .bind(customer_id)
    .execute(&mut *tx)
    .await?;

    // Preserve statutory (non-draft / retention-held) invoices & their line items:
    // capture honest billing_snapshot if missing, and unlink customer_id = NULL.
    let mut missing_fields = Vec::new();
    let name = customer.name.trim().to_string();
    let address = customer.address.trim().to_string();
    let postal_code = customer.postal_code.trim().to_string();
    let city = customer.city.trim().to_string();
    let country = customer.country.trim().to_string();
    if name.is_empty() {
        missing_fields.push("name".to_string());
    }
    if address.is_empty() {
        missing_fields.push("address_line1".to_string());
    }
    if postal_code.is_empty() {
        missing_fields.push("postal_code".to_string());
    }
    if city.is_empty() {
        missing_fields.push("city".to_string());
    }
    if country.is_empty() {
        missing_fields.push("country".to_string());
    }
    let snapshot = InvoiceBillingSnapshot {
        name,
        company: customer.company.trim().to_string(),
        address_line1: address,
        postal_code,
        city,
        country,
        vat_id: customer.vat_id.trim().to_string(),
        snapshot_complete: missing_fields.is_empty(),
        missing_fields,
        snapshot_source: "account_deletion_unlink".to_string(),
        captured_at: Some(chrono::Utc::now()),
    };

    sqlx::query(
        r#"UPDATE invoices
           SET customer_id = NULL,
               billing_snapshot = COALESCE(billing_snapshot, $2),
               updated_at = now()
           WHERE customer_id = $1"#,
    )
    .bind(customer_id)
    .bind(sqlx::types::Json(snapshot))
    .execute(&mut *tx)
    .await?;

    // Delete non-statutory customer-owned records and unlink references
    sqlx::query("DELETE FROM customer_billing_emails WHERE customer_id = $1")
        .bind(customer_id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM customer_mfa_recovery_codes WHERE customer_id = $1")
        .bind(customer_id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM customer_consents WHERE customer_id = $1")
        .bind(customer_id)
        .execute(&mut *tx)
        .await?;
    sqlx::query(
        "DELETE FROM ticket_messages WHERE ticket_id IN (SELECT id FROM tickets WHERE customer_id = $1)",
    )
    .bind(customer_id)
    .execute(&mut *tx)
    .await?;
    sqlx::query("DELETE FROM tickets WHERE customer_id = $1")
        .bind(customer_id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("UPDATE jobs SET customer_id = NULL WHERE customer_id = $1")
        .bind(customer_id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("UPDATE deletion_workflows SET customer_id = NULL WHERE customer_id = $1")
        .bind(customer_id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("UPDATE cancellation_requests SET customer_id = NULL WHERE customer_id = $1")
        .bind(customer_id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM services WHERE customer_id = $1")
        .bind(customer_id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM customers WHERE id = $1")
        .bind(customer_id)
        .execute(&mut *tx)
        .await?;

    tx.commit().await?;

    crate::activity::spawn_record(
        state.db.clone(),
        Some(customer_id),
        vaultscope_db::models::ActorType::Customer,
        customer.name.clone(),
        "customer".to_string(),
        "account_closed".to_string(),
        customer.email.clone(),
        format!("Customer self-service account closure: {}", customer.email),
    );

    Ok(Json(serde_json::json!({
        "ok": true,
        "message": "Your account has been successfully deleted."
    })))
}



