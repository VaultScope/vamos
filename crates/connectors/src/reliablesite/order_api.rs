use std::sync::Arc;

use chrono::{DateTime, Duration, Utc};
use reqwest::{Client, Method, StatusCode};
use tokio::sync::RwLock;

use crate::{ConnectorError, ConnectorResult};

use super::types::{
    CreateOrderRequest, CreateOrderResponse, LoginTokenResponse, PaymentMethodsResponse,
    ProductStockResponse, ResellerPromoCodesResponse, VerifyTokenResponse,
};

pub const DEFAULT_ORDER_API_URL: &str = "https://order-api.reliablesite.dev";

#[derive(Clone)]
struct CachedToken {
    bearer: String,
    expires_at: Option<DateTime<Utc>>,
}

/// Client for the ReliableSite Order API (`https://order-api.reliablesite.dev`).
#[derive(Clone)]
pub struct OrderApiClient {
    client: Client,
    base_url: String,
    api_key: String,
    cached_token: Arc<RwLock<Option<CachedToken>>>,
}

impl OrderApiClient {
    pub fn new(client: Client, base_url: String, api_key: String) -> Self {
        let cleaned = base_url.trim().trim_end_matches('/');
        let url = if cleaned.is_empty() {
            DEFAULT_ORDER_API_URL.to_string()
        } else {
            cleaned.to_string()
        };
        Self {
            client,
            base_url: url,
            api_key: api_key.trim().to_string(),
            cached_token: Arc::new(RwLock::new(None)),
        }
    }

    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    /// Exchanges the customer API key for a temporary Order API bearer token (`POST /v1/Login/Token`).
    pub async fn login_token(&self) -> ConnectorResult<String> {
        if self.api_key.is_empty() {
            return Err(ConnectorError::AuthFailed);
        }

        let url = format!("{}/v1/Login/Token", self.base_url);
        let resp = self
            .client
            .post(&url)
            .query(&[("apiKey", &self.api_key)])
            .header("User-Agent", "VaultScope/1.0")
            .timeout(std::time::Duration::from_secs(20))
            .send()
            .await
            .map_err(map_transport_err)?;

        let status = resp.status();
        if status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN {
            return Err(ConnectorError::AuthFailed);
        }

        let body_text = resp.text().await.unwrap_or_default();
        if !status.is_success() {
            return Err(ConnectorError::Api(format!(
                "ReliableSite Login/Token failed ({}): {}",
                status, body_text
            )));
        }

        let parsed: LoginTokenResponse = serde_json::from_str(&body_text).map_err(|e| {
            ConnectorError::Api(format!("Invalid Login/Token JSON ({}): {}", e, body_text))
        })?;

        if parsed.status == Some(false) {
            return Err(ConnectorError::AuthFailed);
        }

        let token = parsed
            .token
            .filter(|t| !t.trim().is_empty())
            .or(parsed.message.filter(|t| !t.trim().is_empty()))
            .ok_or_else(|| ConnectorError::Api("Missing token in Login/Token response".into()))?;

        // Conservative 10-minute default cache TTL; refreshed immediately ifVerifyToken reports earlier expiry or on 401
        let expires_at = Some(Utc::now() + Duration::minutes(10));
        *self.cached_token.write().await = Some(CachedToken {
            bearer: token.clone(),
            expires_at,
        });

        Ok(token)
    }

    /// Verifies an existing or freshly fetched bearer token (`GET /v1/Login/VerifyToken`).
    pub async fn verify_token(&self) -> ConnectorResult<VerifyTokenResponse> {
        let token = self.get_or_refresh_token().await?;
        let url = format!("{}/v1/Login/VerifyToken", self.base_url);
        let resp = self
            .client
            .get(&url)
            .bearer_auth(&token)
            .header("User-Agent", "VaultScope/1.0")
            .timeout(std::time::Duration::from_secs(20))
            .send()
            .await
            .map_err(map_transport_err)?;

        if resp.status() == StatusCode::UNAUTHORIZED {
            *self.cached_token.write().await = None;
            return Err(ConnectorError::AuthFailed);
        }

        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        if !status.is_success() {
            return Err(ConnectorError::Api(format!(
                "VerifyToken failed ({}): {}",
                status, text
            )));
        }

        let verified: VerifyTokenResponse = serde_json::from_str(&text)
            .map_err(|e| ConnectorError::Api(format!("Invalid VerifyToken JSON: {}", e)))?;

        if !verified.status {
            *self.cached_token.write().await = None;
            return Err(ConnectorError::AuthFailed);
        }

        if let Some(exp) = verified.token_expiration_timestamp {
            if let Some(guard) = self.cached_token.write().await.as_mut() {
                guard.expires_at = Some(exp);
            }
        }

        Ok(verified)
    }

    async fn get_or_refresh_token(&self) -> ConnectorResult<String> {
        {
            let guard = self.cached_token.read().await;
            if let Some(cached) = guard.as_ref() {
                let valid = match cached.expires_at {
                    Some(exp) => exp > Utc::now() + Duration::seconds(60),
                    None => true,
                };
                if valid {
                    return Ok(cached.bearer.clone());
                }
            }
        }
        self.login_token().await
    }

    async fn authed_request(
        &self,
        method: Method,
        path_and_query: &str,
        extra_headers: &[(&str, &str)],
        json_body: Option<&serde_json::Value>,
    ) -> ConnectorResult<(StatusCode, String)> {
        let mut attempt = 0;
        loop {
            attempt += 1;
            let token = self.get_or_refresh_token().await?;
            let url = format!("{}{}", self.base_url, path_and_query);
            let mut req = self
                .client
                .request(method.clone(), &url)
                .bearer_auth(&token)
                .header("User-Agent", "VaultScope/1.0");

            for (k, v) in extra_headers {
                req = req.header(*k, *v);
            }
            if let Some(body) = json_body {
                req = req.json(body);
            }

            let resp = req
                .timeout(std::time::Duration::from_secs(20))
                .send()
                .await
                .map_err(map_transport_err)?;
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();

            if status == StatusCode::UNAUTHORIZED && attempt == 1 {
                *self.cached_token.write().await = None;
                continue;
            }
            return Ok((status, text));
        }
    }

    pub async fn get_payment_methods(&self, product_id: i32) -> ConnectorResult<PaymentMethodsResponse> {
        let (status, text) = self
            .authed_request(
                Method::GET,
                &format!("/v1/PaymentMethods?productId={}", product_id),
                &[],
                None,
            )
            .await?;
        handle_status(status, &text, "PaymentMethods")?;
        serde_json::from_str(&text)
            .map_err(|e| ConnectorError::Api(format!("Invalid PaymentMethods JSON: {}", e)))
    }

    pub async fn get_product_stock(&self, product_id: i32) -> ConnectorResult<ProductStockResponse> {
        let (status, text) = self
            .authed_request(
                Method::GET,
                &format!("/v1/Products/{}/Stock", product_id),
                &[],
                None,
            )
            .await?;
        handle_status(status, &text, "ProductStock")?;
        serde_json::from_str(&text)
            .map_err(|e| ConnectorError::Api(format!("Invalid ProductStock JSON: {}", e)))
    }

    pub async fn get_reseller_promo_codes(
        &self,
        configuration_id: Option<i32>,
    ) -> ConnectorResult<ResellerPromoCodesResponse> {
        let path = match configuration_id {
            Some(cid) => format!("/v1/Reseller/PromoCodes?configurationId={}", cid),
            None => "/v1/Reseller/PromoCodes".to_string(),
        };
        let (status, text) = self.authed_request(Method::GET, &path, &[], None).await?;
        handle_status(status, &text, "ResellerPromoCodes")?;
        serde_json::from_str(&text)
            .map_err(|e| ConnectorError::Api(format!("Invalid ResellerPromoCodes JSON: {}", e)))
    }

    pub async fn create_order(
        &self,
        req: &CreateOrderRequest,
        idempotency_key: &str,
    ) -> ConnectorResult<CreateOrderResponse> {
        validate_create_order_request(req, idempotency_key)?;
        let body = serde_json::to_value(req)
            .map_err(|e| ConnectorError::Api(format!("Failed to serialize CreateOrderRequest: {}", e)))?;

        let (status, text) = self
            .authed_request(
                Method::POST,
                "/v1/Orders",
                &[
                    ("Idempotency-Key", idempotency_key.trim()),
                    ("Content-Type", "application/json"),
                ],
                Some(&body),
            )
            .await?;

        handle_status(status, &text, "CreateOrder")?;
        serde_json::from_str(&text)
            .map_err(|e| ConnectorError::Api(format!("Invalid CreateOrder JSON: {}", e)))
    }
}

fn map_transport_err(e: reqwest::Error) -> ConnectorError {
    tracing::warn!(
        status = ?e.status(),
        is_timeout = e.is_timeout(),
        "ReliableSite Order API transport error"
    );
    ConnectorError::Api("ReliableSite Order API request failed".into())
}

fn handle_status(status: StatusCode, body: &str, op: &str) -> ConnectorResult<()> {
    if status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN {
        return Err(ConnectorError::AuthFailed);
    }
    if status == StatusCode::TOO_MANY_REQUESTS {
        return Err(ConnectorError::RateLimited);
    }
    if !status.is_success() {
        return Err(ConnectorError::Api(format!(
            "ReliableSite {} failed ({}): {}",
            op, status, body
        )));
    }
    Ok(())
}

pub fn validate_create_order_request(
    req: &CreateOrderRequest,
    idempotency_key: &str,
) -> ConnectorResult<()> {
    if req.product_id <= 0 {
        return Err(ConnectorError::Api("productId must be positive".into()));
    }
    let key = idempotency_key.trim();
    if key.is_empty() || key.len() > 128 {
        return Err(ConnectorError::Api(
            "Idempotency-Key must be between 1 and 128 characters".into(),
        ));
    }
    let hostname = req.hostname.trim();
    if hostname.is_empty() || hostname.len() > 65535 {
        return Err(ConnectorError::Api(
            "hostname must be non-empty and at most 65,535 characters".into(),
        ));
    }
    if req.payment_method.trim().is_empty() {
        return Err(ConnectorError::Api("paymentMethod is required".into()));
    }
    if req.config_options.is_empty() {
        return Err(ConnectorError::Api(
            "configOptions must contain at least one Group_Id -> Addon_Id mapping".into(),
        ));
    }
    for (gid_str, addon_id) in &req.config_options {
        let gid: u32 = gid_str.parse().map_err(|_| {
            ConnectorError::Api(format!("Invalid numeric Group_Id key in configOptions: {}", gid_str))
        })?;
        if gid == 0 || *addon_id == 0 {
            return Err(ConnectorError::Api(
                "configOptions Group_Id and Addon_Id must be positive numbers".into(),
            ));
        }
    }
    if let Some(ref promo) = req.promo_code {
        if promo.len() > 64 {
            return Err(ConnectorError::Api(
                "promoCode exceeds maximum 64 characters".into(),
            ));
        }
    }
    if let Some(ref notes) = req.order_notes {
        if notes.len() > 2000 {
            return Err(ConnectorError::Api(
                "orderNotes exceeds maximum 2,000 characters".into(),
            ));
        }
    }
    Ok(())
}
