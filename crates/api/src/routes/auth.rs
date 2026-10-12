use axum::extract::State;
use axum::routing::post;
use axum::{Json, Router};
use chrono::Utc;
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::auth::{jwt, Audience, Claims};
use crate::error::ApiError;
use crate::state::AppState;
use crate::validation::{validate_email, validate_required_string, validate_string_length};
use bcrypt::{hash, verify, DEFAULT_COST};

pub fn routes(state: &AppState) -> Router<AppState> {
    let mut router = Router::new()
        .route("/login", post(login))
        .route("/register", post(register))
        .route("/verify-email", post(verify_email))
        .route("/resend-verification", post(resend_verification));

    if state.config.dev_mode {
        router = router.route("/dev-login", post(dev_login));
    }

    router
}

#[derive(Deserialize)]
struct LoginPayload {
    email: String,
    password: String,
    audience: String,
    totp_code: Option<String>,
}

async fn login(
    State(state): State<AppState>,
    Json(payload): Json<LoginPayload>,
) -> Result<Json<Value>, ApiError> {
    validate_required_string(&payload.email, "Email")?;
    validate_required_string(&payload.password, "Password")?;
    validate_email(&payload.email)?;

    let is_admin = payload.audience == "admin";

    if is_admin {
        let staff: Option<(Uuid, String, String, Option<String>, bool)> = sqlx::query_as(
            "SELECT s.id, s.email, r.name, s.password_hash, s.email_verified FROM staff s JOIN roles r ON s.role_id = r.id WHERE s.email = $1 LIMIT 1"
        )
        .bind(&payload.email)
        .fetch_optional(&state.db)
        .await?;

        let (staff_id, email, role_name, password_hash, email_verified) = staff.ok_or_else(|| {
            ApiError::Unauthorized
        })?;

        let hash_str = password_hash.ok_or_else(|| {
            tracing::warn!("Staff {} has no password hash", email);
            ApiError::Unauthorized
        })?;

        if !verify(&payload.password, &hash_str).unwrap_or(false) {
            return Err(ApiError::Unauthorized);
        }

        sqlx::query("UPDATE staff SET last_login = now() WHERE id = $1")
            .bind(staff_id)
            .execute(&state.db)
            .await?;

        let claims = Claims {
            sub: staff_id,
            email,
            role: role_name,
            audience: Audience::Admin,
            iat: Utc::now().timestamp(),
            exp: jwt::new_expiry(),
        };

        let token = jwt::sign(&claims, &state.config.jwt_secret)?;
        Ok(Json(json!({ "token": token, "claims": claims, "email_verified": email_verified })))
    } else {
        let customer: Option<(Uuid, String, Option<String>, bool, bool, Option<String>)> = sqlx::query_as(
            "SELECT id, email, password_hash, email_verified, two_factor_enabled, totp_secret FROM customers WHERE email = $1 AND status = 'active' LIMIT 1"
        )
        .bind(&payload.email)
        .fetch_optional(&state.db)
        .await?;

        let (customer_id, email, password_hash, email_verified, two_factor_enabled, totp_secret) = customer.ok_or_else(|| {
            ApiError::Unauthorized
        })?;

        let hash_str = password_hash.ok_or_else(|| {
            tracing::warn!("Customer {} has no password hash", email);
            ApiError::Unauthorized
        })?;

        if !verify(&payload.password, &hash_str).unwrap_or(false) {
            return Err(ApiError::Unauthorized);
        }

        if two_factor_enabled {
            if let Some(ref code) = payload.totp_code {
                let secret_str = totp_secret.ok_or(ApiError::Unauthorized)?;
                let secret = totp_rs::Secret::Encoded(secret_str);
                let bytes = secret.to_bytes().map_err(|_| ApiError::Unauthorized)?;
                let totp = totp_rs::TOTP::new(
                    totp_rs::Algorithm::SHA1,
                    6,
                    1,
                    30,
                    bytes,
                    Some("VaultScope".to_string()),
                    email.clone(),
                ).map_err(|_| ApiError::Unauthorized)?;

                let clean_code = code.trim().replace(' ', "");
                if !totp.check_current(&clean_code).unwrap_or(false) {
                    return Err(ApiError::BadRequest("Invalid two-factor authentication code".into()));
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
            email,
            role: "customer".to_string(),
            audience: Audience::Storefront,
            iat: Utc::now().timestamp(),
            exp: jwt::new_expiry(),
        };

        let token = jwt::sign(&claims, &state.config.jwt_secret)?;
        Ok(Json(json!({ "token": token, "claims": claims, "email_verified": email_verified })))
    }
}

#[derive(Deserialize)]
struct RegisterPayload {
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

async fn register(
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
    validate_string_length(&payload.name, "Name", 255)?;

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

    let existing: Option<(Uuid,)> = sqlx::query_as(
        "SELECT id FROM customers WHERE email = $1"
    )
    .bind(&payload.email)
    .fetch_optional(&state.db)
    .await?;

    if existing.is_some() {
        return Err(ApiError::BadRequest("An account with this email already exists".into()));
    }

    tracing::info!("Registering customer directly: {}", payload.email);
    let customer_id = Uuid::now_v7();
    let external_id = format!("local_{}", customer_id);
    let password_hash = hash(&payload.password, DEFAULT_COST).map_err(|_| ApiError::Internal("Failed to hash password".into()))?;

    let postal_code = payload.postal_code.as_deref().unwrap_or("").trim();
    let token_bytes: [u8; 32] = rand::random();
    let verification_token = hex::encode(token_bytes);

    sqlx::query(
        r#"INSERT INTO customers (id, external_id, name, email, phone, company, address, postal_code, city, country, vat_id, status, email_verified, password_hash, verification_token)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, 'active', false, $12, $13)"#
    )
    .bind(customer_id)
    .bind(external_id)
    .bind(payload.name.trim())
    .bind(payload.email.trim())
    .bind(payload.phone.as_deref().unwrap_or(""))
    .bind(payload.company.as_deref().unwrap_or(""))
    .bind(payload.address.trim())
    .bind(postal_code)
    .bind(payload.city.trim())
    .bind(payload.country.trim())
    .bind(payload.vat_id.as_deref().unwrap_or(""))
    .bind(password_hash)
    .bind(&verification_token)
    .execute(&state.db)
    .await?;

    send_verification_email(&state.config, payload.email.trim(), payload.name.trim(), &verification_token);

    Ok(Json(json!({
        "ok": true,
        "message": "Account created successfully. Please check your email and spam folder to verify your account."
    })))
}

pub fn send_verification_email(
    config: &crate::config::Config,
    to_email: &str,
    to_name: &str,
    token: &str,
) {
    let smtp_host = config.smtp_host.clone().unwrap_or_else(|| "mail.vaultscope.de".to_string());
    let smtp_user = config.smtp_username.clone().unwrap_or_else(|| "system@vaultscope.de".to_string());
    let smtp_pass = config.smtp_password.clone().unwrap_or_else(|| "".to_string());
    let smtp_from = config.smtp_from.clone().unwrap_or_else(|| "system@vaultscope.de".to_string());

    let verify_link = format!("http://localhost:5173/verify-email?token={}", token);
    tracing::info!("Verification link for {}: {}", to_email, verify_link);

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

    let email = match lettre::Message::builder()
        .from(from_mailbox)
        .to(to_mailbox)
        .subject("Verify your VaultScope email address")
        .body(body)
    {
        Ok(msg) => msg,
        Err(e) => {
            tracing::error!("Failed to build verification email: {:?}", e);
            return;
        }
    };

    let creds = lettre::transport::smtp::authentication::Credentials::new(smtp_user, smtp_pass);
    let mailer = match lettre::SmtpTransport::relay(&smtp_host) {
        Ok(b) => b.credentials(creds).build(),
        Err(e) => {
            tracing::error!("Failed to create SMTP relay transport: {:?}", e);
            return;
        }
    };

    if let Err(e) = lettre::Transport::send(&mailer, &email) {
        tracing::error!("Could not send verification email to {}: {:?}", to_email, e);
    } else {
        tracing::info!("Verification email sent successfully to {}", to_email);
    }
}

pub fn send_billing_email_verification(
    config: &crate::config::Config,
    to_email: &str,
    company_name: &str,
    token: &str,
) {
    let smtp_host = config.smtp_host.clone().unwrap_or_else(|| "mail.vaultscope.de".to_string());
    let smtp_user = config.smtp_username.clone().unwrap_or_else(|| "system@vaultscope.de".to_string());
    let smtp_pass = config.smtp_password.clone().unwrap_or_else(|| "".to_string());
    let smtp_from = config.smtp_from.clone().unwrap_or_else(|| "system@vaultscope.de".to_string());

    let verify_link = format!("http://localhost:5173/verify-email?token={}&type=billing", token);
    tracing::info!("Billing email verification link for {}: {}", to_email, verify_link);

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

    let email = match lettre::Message::builder()
        .from(from_mailbox)
        .to(to_mailbox)
        .subject(format!("Confirm Billing CC Email for {} - VaultScope", company_name))
        .body(body)
    {
        Ok(msg) => msg,
        Err(e) => {
            tracing::error!("Failed to build billing verification email: {:?}", e);
            return;
        }
    };

    let creds = lettre::transport::smtp::authentication::Credentials::new(smtp_user, smtp_pass);
    let mailer = match lettre::SmtpTransport::relay(&smtp_host) {
        Ok(b) => b.credentials(creds).build(),
        Err(e) => {
            tracing::error!("Failed to create SMTP relay transport: {:?}", e);
            return;
        }
    };

    if let Err(e) = lettre::Transport::send(&mailer, &email) {
        tracing::error!("Could not send billing verification email to {}: {:?}", to_email, e);
    } else {
        tracing::info!("Billing verification email sent successfully to {}", to_email);
    }
}

#[derive(Deserialize)]
struct VerifyEmailPayload {
    token: String,
}

async fn verify_email(
    State(state): State<AppState>,
    Json(payload): Json<VerifyEmailPayload>,
) -> Result<Json<Value>, ApiError> {
    validate_required_string(&payload.token, "Token")?;
    let token = payload.token.trim();

    // 1. Check primary customer verification token
    let customer: Option<(Uuid, String)> = sqlx::query_as(
        "SELECT id, email FROM customers WHERE verification_token = $1 LIMIT 1"
    )
    .bind(token)
    .fetch_optional(&state.db)
    .await?;

    if let Some((customer_id, email)) = customer {
        sqlx::query(
            "UPDATE customers SET email_verified = true, verification_token = NULL, updated_at = now() WHERE id = $1"
        )
        .bind(customer_id)
        .execute(&state.db)
        .await?;

        tracing::info!("Customer primary email verified successfully: {}", email);

        return Ok(Json(json!({
            "ok": true,
            "message": "Account email address verified successfully."
        })));
    }

    // 2. Check billing CC verification token
    let billing_email: Option<(Uuid, String)> = sqlx::query_as(
        "SELECT id, email FROM customer_billing_emails WHERE verification_token = $1 LIMIT 1"
    )
    .bind(token)
    .fetch_optional(&state.db)
    .await?;

    if let Some((id, email)) = billing_email {
        sqlx::query(
            "UPDATE customer_billing_emails SET verified = true, verification_token = NULL, verified_at = now() WHERE id = $1"
        )
        .bind(id)
        .execute(&state.db)
        .await?;

        tracing::info!("Customer billing CC email verified successfully: {}", email);

        return Ok(Json(json!({
            "ok": true,
            "message": "Billing CC email verified successfully. Invoices will now be sent to this address."
        })));
    }

    Err(ApiError::BadRequest("Invalid or expired verification token.".into()))
}

#[derive(Deserialize)]
struct ResendVerificationPayload {
    email: String,
}

async fn resend_verification(
    State(state): State<AppState>,
    Json(payload): Json<ResendVerificationPayload>,
) -> Result<Json<Value>, ApiError> {
    validate_required_string(&payload.email, "Email")?;
    validate_email(&payload.email)?;

    let customer: Option<(Uuid, String, String, bool)> = sqlx::query_as(
        "SELECT id, name, email, email_verified FROM customers WHERE email = $1 LIMIT 1"
    )
    .bind(payload.email.trim())
    .fetch_optional(&state.db)
    .await?;

    if let Some((customer_id, name, email, email_verified)) = customer {
        if email_verified {
            return Ok(Json(json!({
                "ok": true,
                "message": "Your email is already verified. You can log in directly."
            })));
        }

        let token_bytes: [u8; 32] = rand::random();
        let verification_token = hex::encode(token_bytes);

        sqlx::query(
            "UPDATE customers SET verification_token = $1, updated_at = now() WHERE id = $2"
        )
        .bind(&verification_token)
        .bind(customer_id)
        .execute(&state.db)
        .await?;

        send_verification_email(&state.config, &email, &name, &verification_token);
    }

    Ok(Json(json!({
        "ok": true,
        "message": "If your account exists and is unverified, a verification link has been sent. Please check your inbox and spam folder."
    })))
}

#[derive(Deserialize)]
struct DevLoginPayload {
    role: Option<String>,
    persona: Option<String>,
}

async fn dev_login(
    State(state): State<AppState>,
    Json(payload): Json<DevLoginPayload>,
) -> Result<Json<Value>, ApiError> {
    if !state.config.dev_mode {
        return Err(ApiError::NotFound("Not found".into()));
    }

    let persona = payload.persona.as_deref().or(payload.role.as_deref()).unwrap_or("Owner");

    if persona.eq_ignore_ascii_case("customer") || persona.eq_ignore_ascii_case("customer1") {
        let customer: Option<(Uuid, String, bool)> = sqlx::query_as(
            "SELECT id, email, email_verified FROM customers WHERE email = 'customer1@test.local' OR status = 'active' ORDER BY created_at ASC LIMIT 1"
        )
        .fetch_optional(&state.db)
        .await?;

        let (customer_id, email, email_verified) = customer.ok_or_else(|| {
            ApiError::Internal("No customer found in database. Run database seeding.".into())
        })?;

        let claims = Claims {
            sub: customer_id,
            email,
            role: "customer".to_string(),
            audience: Audience::Storefront,
            iat: Utc::now().timestamp(),
            exp: jwt::new_expiry(),
        };

        let token = jwt::sign(&claims, &state.config.jwt_secret)?;
        return Ok(Json(json!({ "token": token, "claims": claims, "email_verified": email_verified })));
    }

    let persona_lower = persona.to_lowercase();
    let query_role = match persona_lower.as_str() {
        "admin1" | "owner" => "Owner",
        "support1" | "support" => "Support",
        "billing1" | "billing" => "Billing",
        "tech1" | "technical" => "Technical",
        _ => persona,
    };

    let staff: Option<(Uuid, String, String)> = sqlx::query_as(
        "SELECT s.id, s.email, r.name FROM staff s JOIN roles r ON s.role_id = r.id WHERE r.name = $1 OR s.username = $2 LIMIT 1"
    )
    .bind(query_role)
    .bind(persona)
    .fetch_optional(&state.db)
    .await
    .map_err(ApiError::from)?;

    let (staff_id, email, role_name) = staff.ok_or_else(|| {
        ApiError::BadRequest(format!("Staff member for persona '{}' not found. Please run database seeding.", persona))
    })?;

    sqlx::query("UPDATE staff SET last_login = now() WHERE id = $1")
        .bind(staff_id)
        .execute(&state.db)
        .await?;

    let claims = Claims {
        sub: staff_id,
        email,
        role: role_name,
        audience: Audience::Admin,
        iat: Utc::now().timestamp(),
        exp: jwt::new_expiry(),
    };

    let token = jwt::sign(&claims, &state.config.jwt_secret)?;
    Ok(Json(json!({ "token": token, "claims": claims, "email_verified": true })))
}
