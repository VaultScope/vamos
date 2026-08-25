use chrono::Utc;
use jsonwebtoken::{decode, encode, DecodingKey, EncodingKey, Header, Validation};

use super::Claims;
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

pub fn verify(token: &str, secret: &str) -> Result<Claims, ApiError> {
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
