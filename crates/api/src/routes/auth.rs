use axum::extract::State;
use axum::routing::post;
use axum::{Json, Router};
use chrono::Utc;
use rand::Rng;
use redis::AsyncCommands;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use uuid::Uuid;

use crate::auth::{jwt, oidc, Audience, Claims};
use crate::error::ApiError;
use crate::state::AppState;
use crate::validation::{validate_email, validate_required_string, validate_string_length};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/init-login", post(init_login))
        .route("/callback/admin", post(admin_callback))
        .route("/callback/storefront", post(storefront_callback))
        .route("/register", post(register))
}

// SEC-001: OIDC state generation response
#[derive(Serialize)]
struct InitLoginResponse {
    state: String,
}

#[derive(Deserialize)]
struct CallbackPayload {
    code: String,
    redirect_uri: String,
    state: String,
}

// SEC-001: Generate and store OIDC state in Redis
async fn init_login(State(state): State<AppState>) -> Result<Json<InitLoginResponse>, ApiError> {
    // Generate 32-byte random state
    let state_bytes: [u8; 32] = rand::thread_rng().gen();
    let state_value = hex::encode(state_bytes);

    // Store in Redis with 10-minute TTL
    let key = format!("oidc:state:{}", state_value);
    let mut redis = state.redis.clone();
    redis
        .set_ex::<_, _, ()>(&key, "1", 600)
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to store state: {}", e)))?;

    tracing::info!("OIDC state generated: {}", state_value);

    Ok(Json(InitLoginResponse {
        state: state_value,
    }))
}

// SEC-001: Validate and consume OIDC state from Redis
async fn validate_and_consume_state(
    redis: &mut redis::aio::ConnectionManager,
    state: &str,
) -> Result<(), ApiError> {
    // Validate state format (64 hex characters from 32-byte random generation)
    if state.len() != 64 || !state.chars().all(|c| c.is_ascii_hexdigit()) {
        tracing::warn!("OIDC state validation failed: invalid format (expected 64 hex chars)");
        return Err(ApiError::Unauthorized);
    }

    let key = format!("oidc:state:{}", state);

    // Atomically get and delete state (prevents TOCTOU race condition)
    // Using GETDEL ensures only one concurrent request can consume the state
    let value: Option<String> = redis
        .get_del(&key)
        .await
        .map_err(|e| {
            tracing::error!("Redis error validating state: {}", e);
            ApiError::Internal(format!("Redis error: {}", e))
        })?;

    if value.is_none() {
        tracing::warn!("OIDC state validation failed: state not found or expired");
        return Err(ApiError::Unauthorized);
    }

    tracing::info!("OIDC state validated and consumed: {}", state);
    Ok(())
}

async fn admin_callback(
    State(state): State<AppState>,
    Json(payload): Json<CallbackPayload>,
) -> Result<Json<Value>, ApiError> {
    // SEC-001: Validate and consume OIDC state
    let mut redis = state.redis.clone();
    validate_and_consume_state(&mut redis, &payload.state).await?;

    let tokens = oidc::exchange_code(&state.config, &payload.code, &payload.redirect_uri, true).await?;
    let userinfo = oidc::fetch_userinfo(&state.config, &tokens.access_token).await?;

    tracing::info!("admin login: sub={}, email={}", userinfo.sub, userinfo.email);

    let staff: Option<(Uuid, String)> = sqlx::query_as(
        "SELECT s.id, r.name FROM staff s JOIN roles r ON s.role_id = r.id WHERE s.external_id = $1"
    )
    .bind(&userinfo.sub)
    .fetch_optional(&state.db)
    .await
    .map_err(ApiError::from)?;

    let (staff_id, role_name) = staff.ok_or_else(|| {
        tracing::warn!("admin login denied: no staff record for sub={}", userinfo.sub);
        ApiError::Forbidden
    })?;

    sqlx::query("UPDATE staff SET last_login = now() WHERE id = $1")
        .bind(staff_id)
        .execute(&state.db)
        .await?;

    let claims = Claims {
        sub: staff_id,
        email: userinfo.email,
        role: role_name,
        audience: Audience::Admin,
        iat: Utc::now().timestamp(),
        exp: jwt::new_expiry(),
    };

    let token = jwt::sign(&claims, &state.config.jwt_secret)?;
    Ok(Json(json!({ "token": token, "claims": claims })))
}

async fn storefront_callback(
    State(state): State<AppState>,
    Json(payload): Json<CallbackPayload>,
) -> Result<Json<Value>, ApiError> {
    // SEC-001: Validate and consume OIDC state
    let mut redis = state.redis.clone();
    validate_and_consume_state(&mut redis, &payload.state).await?;

    let tokens = oidc::exchange_code(&state.config, &payload.code, &payload.redirect_uri, false).await?;
    let userinfo = oidc::fetch_userinfo(&state.config, &tokens.access_token).await?;

    let customer_id: Option<(Uuid,)> = sqlx::query_as(
        "SELECT id FROM customers WHERE external_id = $1"
    )
    .bind(&userinfo.sub)
    .fetch_optional(&state.db)
    .await?;

    let customer_id = match customer_id {
        Some((id,)) => {
            sqlx::query("UPDATE customers SET last_login = now() WHERE id = $1")
                .bind(id)
                .execute(&state.db)
                .await?;
            id
        }
        None => {
            let id = Uuid::now_v7();
            sqlx::query(
                "INSERT INTO customers (id, external_id, name, email) VALUES ($1, $2, $3, $4)"
            )
            .bind(id)
            .bind(&userinfo.sub)
            .bind(userinfo.preferred_username.as_deref().unwrap_or(&userinfo.email))
            .bind(&userinfo.email)
            .execute(&state.db)
            .await?;
            id
        }
    };

    let claims = Claims {
        sub: customer_id,
        email: userinfo.email,
        role: "customer".to_string(),
        audience: Audience::Storefront,
        iat: Utc::now().timestamp(),
        exp: jwt::new_expiry(),
    };

    let token = jwt::sign(&claims, &state.config.jwt_secret)?;
    Ok(Json(json!({ "token": token, "claims": claims })))
}

// ─── Registration ────────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct RegisterPayload {
    username: String,
    email: String,
    password: String,
    name: String,
    company: Option<String>,
    address: String,
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

    if payload.password.len() < 8 {
        return Err(ApiError::BadRequest("Password must be at least 8 characters".into()));
    }

    let api_token = state.config.authentik_api_token.as_deref()
        .ok_or(ApiError::Internal("Registration not configured (missing AUTHENTIK_API_TOKEN)".into()))?;

    let existing: Option<(Uuid,)> = sqlx::query_as(
        "SELECT id FROM customers WHERE email = $1"
    )
    .bind(&payload.email)
    .fetch_optional(&state.db)
    .await?;

    if existing.is_some() {
        return Err(ApiError::BadRequest("An account with this email already exists".into()));
    }

    let authentik_url = state.config.authentik_issuer.trim_end_matches('/');
    let client = reqwest::Client::new();

    // Create user in Authentik, assigned to the vs-customer group
    let customer_group_id = "670f1bae-4b4c-4a4e-93ae-17318869c028";
    let create_resp = client
        .post(format!("{}/api/v3/core/users/", authentik_url))
        .bearer_auth(api_token)
        .json(&json!({
            "username": payload.username.trim(),
            "email": payload.email.trim(),
            "name": payload.name.trim(),
            "is_active": true,
            "groups": [customer_group_id],
        }))
        .send()
        .await
        .map_err(|e| ApiError::Internal(format!("Authentik API error: {}", e)))?;

    if !create_resp.status().is_success() {
        let status = create_resp.status().as_u16();
        let body = create_resp.text().await.unwrap_or_default();
        tracing::error!("Authentik user creation failed ({}): {}", status, body);
        if body.contains("already exists") || body.contains("unique") {
            return Err(ApiError::BadRequest("This username or email is already taken".into()));
        }
        return Err(ApiError::Internal(format!("Failed to create account ({})", status)));
    }

    let user_data: Value = create_resp.json().await
        .map_err(|e| ApiError::Internal(format!("Authentik response parse error: {}", e)))?;

    let authentik_user_id = user_data["pk"].as_i64()
        .ok_or(ApiError::Internal("Missing user ID from Authentik".into()))?;
    let fallback_uid = format!("{}", authentik_user_id);
    let authentik_uid = user_data["uid"].as_str()
        .unwrap_or(&fallback_uid);

    // Set password in Authentik
    let pw_resp = client
        .post(format!("{}/api/v3/core/users/{}/set_password/", authentik_url, authentik_user_id))
        .bearer_auth(api_token)
        .json(&json!({ "password": payload.password }))
        .send()
        .await
        .map_err(|e| ApiError::Internal(format!("Authentik password set error: {}", e)))?;

    if !pw_resp.status().is_success() {
        tracing::error!("Failed to set password for Authentik user {}", authentik_user_id);
    }

    // Create customer in VaultScope DB
    let customer_id = Uuid::now_v7();
    sqlx::query(
        r#"INSERT INTO customers (id, external_id, name, email, phone, company, address, city, country, vat_id, email_verified)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, false)"#
    )
    .bind(customer_id)
    .bind(authentik_uid)
    .bind(payload.name.trim())
    .bind(payload.email.trim())
    .bind(payload.phone.as_deref().unwrap_or(""))
    .bind(payload.company.as_deref().unwrap_or(""))
    .bind(payload.address.trim())
    .bind(payload.city.trim())
    .bind(payload.country.trim())
    .bind(payload.vat_id.as_deref().unwrap_or(""))
    .execute(&state.db)
    .await?;

    tracing::info!("registered customer: {} ({})", payload.username, payload.email);

    Ok(Json(json!({
        "ok": true,
        "message": "Account created successfully. You can now log in."
    })))
}
