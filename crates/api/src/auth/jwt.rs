use chrono::Utc;
use jsonwebtoken::{decode, encode, DecodingKey, EncodingKey, Header, Validation};

use super::{Audience, Claims};
use crate::error::ApiError;

const TOKEN_EXPIRY_HOURS: i64 = 24;

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
        return Err(ApiError::Unauthorized);
    }

    Ok(data.claims)
}

/// Deprecated: Use verify() with explicit audience instead
#[deprecated(note = "Use verify() with explicit audience parameter")]
pub fn verify_legacy(token: &str, secret: &str) -> Result<Claims, ApiError> {
    // For backwards compatibility during migration only
    let data = decode::<Claims>(
        token,
        &DecodingKey::from_secret(secret.as_bytes()),
        &Validation::default(),
    )
    .map_err(|_| ApiError::Unauthorized)?;

    Ok(data.claims)
}

pub fn new_expiry() -> i64 {
    (Utc::now() + chrono::Duration::hours(TOKEN_EXPIRY_HOURS)).timestamp()
}
