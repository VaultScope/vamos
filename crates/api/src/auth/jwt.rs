use chrono::Utc;
use jsonwebtoken::{decode, encode, DecodingKey, EncodingKey, Header, Validation};

use super::{Audience, Claims};
use crate::error::ApiError;

const DEFAULT_TOKEN_EXPIRY_SECS: i64 = 24 * 3600;
const MIN_TOKEN_EXPIRY_SECS: i64 = 60;
const MAX_TOKEN_EXPIRY_SECS: i64 = 30 * 24 * 3600;

pub fn sign(claims: &Claims, secret: &str) -> Result<String, ApiError> {
    encode(
        &Header::default(),
        claims,
        &EncodingKey::from_secret(secret.as_bytes()),
    )
    .map_err(|e| ApiError::Internal(format!("jwt sign: {}", e)))
}

/// SEC-003 FIX: Verify JWT token with audience validation
pub fn verify(token: &str, secret: &str, expected_audience: Audience) -> Result<Claims, ApiError> {
    let data = decode::<Claims>(
        token,
        &DecodingKey::from_secret(secret.as_bytes()),
        &Validation::default(),
    )
    .map_err(|_| ApiError::Unauthorized)?;

    // SEC-003: Enforce audience validation
    // Prevent cross-audience token reuse (e.g., admin token used for storefront)
    if data.claims.audience != expected_audience {
        return Err(ApiError::Forbidden);
    }

    Ok(data.claims)
}

/// Verify JWT token signature and expiration without restricting to a single audience.
pub fn verify_any_audience(token: &str, secret: &str) -> Result<Claims, ApiError> {
    let data = decode::<Claims>(
        token,
        &DecodingKey::from_secret(secret.as_bytes()),
        &Validation::default(),
    )
    .map_err(|_| ApiError::Unauthorized)?;

    Ok(data.claims)
}

pub fn token_expiry_secs() -> i64 {
    match std::env::var("JWT_EXPIRATION_SECS") {
        Ok(raw) => match raw.trim().parse::<i64>() {
            Ok(secs) if (MIN_TOKEN_EXPIRY_SECS..=MAX_TOKEN_EXPIRY_SECS).contains(&secs) => secs,
            Ok(secs) => {
                tracing::warn!(
                    "JWT_EXPIRATION_SECS={} out of bounds ({}..={}), falling back to {}s",
                    secs,
                    MIN_TOKEN_EXPIRY_SECS,
                    MAX_TOKEN_EXPIRY_SECS,
                    DEFAULT_TOKEN_EXPIRY_SECS
                );
                DEFAULT_TOKEN_EXPIRY_SECS
            }
            Err(_) => {
                tracing::warn!(
                    "Invalid JWT_EXPIRATION_SECS='{}', falling back to {}s",
                    raw,
                    DEFAULT_TOKEN_EXPIRY_SECS
                );
                DEFAULT_TOKEN_EXPIRY_SECS
            }
        },
        Err(_) => DEFAULT_TOKEN_EXPIRY_SECS,
    }
}

pub fn new_expiry() -> i64 {
    (Utc::now() + chrono::Duration::seconds(token_expiry_secs())).timestamp()
}
