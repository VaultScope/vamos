use reqwest::Client;
use serde::Deserialize;

use crate::config::Config;
use crate::error::ApiError;

#[derive(Debug, Deserialize)]
pub struct OidcTokenResponse {
    pub access_token: String,
    pub id_token: String,
    pub token_type: String,
    pub expires_in: u64,
}

#[derive(Debug, Deserialize)]
pub struct OidcUserInfo {
    pub sub: String,
    pub email: String,
    pub email_verified: Option<bool>,
    pub preferred_username: Option<String>,
    pub given_name: Option<String>,
    pub family_name: Option<String>,
    pub groups: Option<Vec<String>>,
}

pub async fn exchange_code(
    config: &Config,
    code: &str,
    redirect_uri: &str,
    is_admin: bool,
) -> Result<OidcTokenResponse, ApiError> {
    let (client_id, client_secret) = if is_admin {
        (&config.authentik_client_id_admin, &config.authentik_client_secret_admin)
    } else {
        (&config.authentik_client_id_storefront, &config.authentik_client_secret_storefront)
    };

    let token_url = format!(
        "{}/application/o/token/",
        config.authentik_issuer.trim_end_matches('/')
    );

    let resp = Client::new()
        .post(&token_url)
        .form(&[
            ("grant_type", "authorization_code"),
            ("code", code),
            ("redirect_uri", redirect_uri),
            ("client_id", client_id),
            ("client_secret", client_secret),
        ])
        .send()
        .await
        .map_err(|e| ApiError::Internal(format!("oidc token exchange: {}", e)))?;

    if !resp.status().is_success() {
        let _text = resp.text().await.unwrap_or_default();
        return Err(ApiError::Unauthorized);
    }

    resp.json()
        .await
        .map_err(|e| ApiError::Internal(format!("oidc parse: {}", e)))
}

pub async fn fetch_userinfo(
    config: &Config,
    access_token: &str,
) -> Result<OidcUserInfo, ApiError> {
    let userinfo_url = format!(
        "{}/application/o/userinfo/",
        config.authentik_issuer.trim_end_matches('/')
    );

    let resp = Client::new()
        .get(&userinfo_url)
        .bearer_auth(access_token)
        .send()
        .await
        .map_err(|e| ApiError::Internal(format!("oidc userinfo: {}", e)))?;

    if !resp.status().is_success() {
        return Err(ApiError::Unauthorized);
    }

    resp.json()
        .await
        .map_err(|e| ApiError::Internal(format!("oidc userinfo parse: {}", e)))
}
