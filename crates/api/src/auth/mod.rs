pub mod jwt;

use axum::extract::{FromRef, FromRequestParts};
use axum::http::header::AUTHORIZATION;
use axum::http::request::Parts;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::ApiError;
use crate::state::AppState;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Claims {
    pub sub: Uuid,
    pub email: String,
    pub role: String,
    pub audience: Audience,
    pub exp: i64,
    pub iat: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
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
        // SEC-003: Pass expected audience to JWT verification
        let claims = extract_claims(parts, &app_state, Audience::Admin)?;
        // Double-check audience (defense in depth)
        if claims.audience != Audience::Admin {
            return Err(ApiError::Forbidden);
        }
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
        // SEC-003: Pass expected audience to JWT verification
        let claims = extract_claims(parts, &app_state, Audience::Storefront)?;
        // Double-check audience (defense in depth)
        if claims.audience != Audience::Storefront {
            return Err(ApiError::Forbidden);
        }
        Ok(AuthCustomer(claims))
    }
}

fn extract_claims(
    parts: &Parts,
    state: &AppState,
    expected_audience: Audience,
) -> Result<Claims, ApiError> {
    let token = if let Some(header) = parts
        .headers
        .get(AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
    {
        header
            .strip_prefix("Bearer ")
            .ok_or(ApiError::Unauthorized)?
    } else if let Some(query) = parts.uri.query() {
        query
            .split('&')
            .find_map(|pair| {
                let mut parts = pair.splitn(2, '=');
                if parts.next()? == "token" {
                    parts.next()
                } else {
                    None
                }
            })
            .ok_or(ApiError::Unauthorized)?
    } else {
        return Err(ApiError::Unauthorized);
    };

    // SEC-003: Verify JWT with audience validation
    jwt::verify(token, &state.config.jwt_secret, expected_audience)
}
