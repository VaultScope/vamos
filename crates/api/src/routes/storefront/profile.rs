use axum::extract::{Path, State};
use axum::routing::{get, post, delete};
use axum::{Json, Router};
use serde::Deserialize;
use uuid::Uuid;

use crate::auth::AuthCustomer;
use crate::error::ApiError;
use crate::state::AppState;
use vaultscope_db::models::{Customer, CustomerBillingEmail};
use totp_rs::{Algorithm, Secret, TOTP};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/", get(get_profile).put(update_profile))
        .route("/billing-emails", get(list_billing_emails).post(add_billing_email))
        .route("/billing-emails/{id}", delete(delete_billing_email))
        .route("/billing-emails/{id}/resend", post(resend_billing_email_verification))
        .route("/mfa/setup", post(setup_mfa))
        .route("/mfa/enable", post(enable_mfa))
        .route("/mfa/disable", post(disable_mfa))
        .route("/verify-vat", post(verify_vat))
}

async fn get_profile(
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
#[derive(serde::Serialize)]
struct ProfileResponse {
    id: uuid::Uuid,
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

#[derive(Deserialize)]
struct UpdateProfile {
    name: Option<String>,
    phone: Option<String>,
    company: Option<String>,
    address: Option<String>,
    postal_code: Option<String>,
    city: Option<String>,
    country: Option<String>,
    vat_id: Option<String>,
}

async fn update_profile(
    auth: AuthCustomer,
    State(state): State<AppState>,
    Json(payload): Json<UpdateProfile>,
) -> Result<Json<ProfileResponse>, ApiError> {
    // Validate name if provided
    if let Some(ref name) = payload.name {
        if name.trim().is_empty() {
            return Err(ApiError::BadRequest("Name cannot be empty".into()));
        }
        if name.len() > 255 {
            return Err(ApiError::BadRequest(
                "Name exceeds maximum length of 255 characters".into(),
            ));
        }
    }

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
               updated_at = now()
           WHERE id = $9
           RETURNING *"#,
    )
    .bind(payload.name)
    .bind(payload.phone)
    .bind(payload.company)
    .bind(payload.address)
    .bind(payload.postal_code)
    .bind(payload.city)
    .bind(payload.country)
    .bind(payload.vat_id)
    .bind(auth.0.sub)
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound("customer not found".into()))?;

    Ok(Json(ProfileResponse {
        id: customer.id,
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

#[derive(Deserialize)]
pub struct AddBillingEmailPayload {
    pub email: String,
}

async fn list_billing_emails(
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

async fn add_billing_email(
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
        r#"INSERT INTO customer_billing_emails (id, customer_id, email, verified, verification_token)
           VALUES ($1, $2, $3, false, $4)
           ON CONFLICT (customer_id, email) DO UPDATE
           SET verification_token = EXCLUDED.verification_token
           RETURNING *"#
    )
    .bind(id)
    .bind(auth.0.sub)
    .bind(&email)
    .bind(&token)
    .fetch_one(&state.db)
    .await?;

    crate::routes::auth::send_billing_email_verification(&state.config, &email, &customer.company, &token);

    Ok(Json(billing_email))
}

async fn delete_billing_email(
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

async fn resend_billing_email_verification(
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

    sqlx::query("UPDATE customer_billing_emails SET verification_token = $1 WHERE id = $2")
        .bind(&token)
        .bind(id)
        .execute(&state.db)
        .await?;

    crate::routes::auth::send_billing_email_verification(&state.config, &billing_email.email, &customer.company, &token);

    Ok(Json(serde_json::json!({
        "ok": true,
        "message": "Verification email resent. Please check the recipient inbox and spam folder."
    })))
}

#[derive(Deserialize)]
pub struct MfaEnablePayload {
    pub code: String,
}

#[derive(Deserialize)]
pub struct MfaDisablePayload {
    pub code: Option<String>,
    pub password: Option<String>,
}

#[derive(Deserialize)]
pub struct VerifyVatPayload {
    pub vat_id: String,
}

async fn setup_mfa(
    auth: AuthCustomer,
    State(state): State<AppState>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let customer = sqlx::query_as::<_, Customer>("SELECT * FROM customers WHERE id = $1")
        .bind(auth.0.sub)
        .fetch_optional(&state.db)
        .await?
        .ok_or(ApiError::NotFound("Customer not found".into()))?;

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
    let url = totp.get_url();

    // Store unconfirmed secret on customer
    sqlx::query("UPDATE customers SET totp_secret = $1 WHERE id = $2")
        .bind(&secret_base32)
        .bind(auth.0.sub)
        .execute(&state.db)
        .await?;

    Ok(Json(serde_json::json!({
        "secret": secret_base32,
        "qr_base64": qr_base64,
        "url": url,
    })))
}

async fn enable_mfa(
    auth: AuthCustomer,
    State(state): State<AppState>,
    Json(payload): Json<MfaEnablePayload>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let customer = sqlx::query_as::<_, Customer>("SELECT * FROM customers WHERE id = $1")
        .bind(auth.0.sub)
        .fetch_optional(&state.db)
        .await?
        .ok_or(ApiError::NotFound("Customer not found".into()))?;

    let secret_str = customer.totp_secret.ok_or_else(|| {
        ApiError::BadRequest("MFA setup was not initiated. Please generate a setup key first.".into())
    })?;

    let secret = Secret::Encoded(secret_str);
    let totp = TOTP::new(
        Algorithm::SHA1,
        6,
        1,
        30,
        secret.to_bytes().map_err(|e| ApiError::Internal(format!("Secret parse error: {e}")))?,
        Some("VaultScope".to_string()),
        customer.email,
    )
    .map_err(|e| ApiError::Internal(format!("TOTP init error: {e}")))?;

    let clean_code = payload.code.trim().replace(' ', "");
    if !totp.check_current(&clean_code).unwrap_or(false) {
        return Err(ApiError::BadRequest("Invalid 6-digit verification code. Please check your authenticator app and system clock.".into()));
    }

    sqlx::query("UPDATE customers SET two_factor_enabled = true, updated_at = now() WHERE id = $1")
        .bind(auth.0.sub)
        .execute(&state.db)
        .await?;

    Ok(Json(serde_json::json!({
        "ok": true,
        "message": "Two-factor authentication successfully activated."
    })))
}

async fn disable_mfa(
    auth: AuthCustomer,
    State(state): State<AppState>,
    Json(payload): Json<MfaDisablePayload>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let customer = sqlx::query_as::<_, Customer>("SELECT * FROM customers WHERE id = $1")
        .bind(auth.0.sub)
        .fetch_optional(&state.db)
        .await?
        .ok_or(ApiError::NotFound("Customer not found".into()))?;

    let mut authorized = false;

    // Verify password if provided
    if let Some(ref pw) = payload.password {
        if let Some(ref hash_str) = customer.password_hash {
            if bcrypt::verify(pw, hash_str).unwrap_or(false) {
                authorized = true;
            }
        }
    }

    // Or verify TOTP code
    if !authorized {
        if let (Some(ref code), Some(ref secret_str)) = (&payload.code, &customer.totp_secret) {
            let secret = Secret::Encoded(secret_str.clone());
            if let Ok(bytes) = secret.to_bytes() {
                if let Ok(totp) = TOTP::new(
                    Algorithm::SHA1,
                    6,
                    1,
                    30,
                    bytes,
                    Some("VaultScope".to_string()),
                    customer.email.clone(),
                ) {
                    let clean_code = code.trim().replace(' ', "");
                    if totp.check_current(&clean_code).unwrap_or(false) {
                        authorized = true;
                    }
                }
            }
        }
    }

    if !authorized {
        return Err(ApiError::BadRequest("Invalid account password or 2FA code to disable two-factor authentication.".into()));
    }

    sqlx::query("UPDATE customers SET two_factor_enabled = false, totp_secret = NULL, updated_at = now() WHERE id = $1")
        .bind(auth.0.sub)
        .execute(&state.db)
        .await?;

    Ok(Json(serde_json::json!({
        "ok": true,
        "message": "Two-factor authentication has been disabled."
    })))
}

async fn verify_vat(
    auth: AuthCustomer,
    State(state): State<AppState>,
    Json(payload): Json<VerifyVatPayload>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let raw_vat = payload.vat_id.trim().to_uppercase().replace([' ', '.', '-'], "");
    if raw_vat.len() < 4 {
        return Err(ApiError::BadRequest("VAT ID format is too short (must include country code, e.g. DE123456789).".into()));
    }

    let country_code = &raw_vat[..2];
    let vat_number = &raw_vat[2..];

    let url = format!(
        "https://ec.europa.eu/taxation_customs/vies/rest-api/ms/{}/vat/{}",
        country_code, vat_number
    );

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(8))
        .build()
        .map_err(|e| ApiError::Internal(e.to_string()))?;

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
                "message": format!("EU VIES service returned HTTP {status}. Please try again shortly.")
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


