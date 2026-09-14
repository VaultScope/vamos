use axum::extract::{Path, State};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{json, Value};
use std::net::IpAddr;
use std::time::Duration;
use uuid::Uuid;

use crate::auth::AuthAdmin;
use crate::error::ApiError;
use crate::state::AppState;
use vaultscope_db::models::Connector;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/", get(list).post(create))
        .route("/{id}", get(get_one).put(update))
        .route("/{id}/test", post(test_connection))
        .route("/{id}/catalog", get(catalog))
}

/// SEC-004 FIX: Validate URL to prevent SSRF attacks
fn validate_connector_url(url: &str) -> Result<(), ApiError> {
    // Parse the URL (using reqwest's re-export of url crate)
    let parsed = reqwest::Url::parse(url)
        .map_err(|_| ApiError::BadRequest("Invalid URL format".into()))?;

    // Only allow HTTPS in production
    #[cfg(not(debug_assertions))]
    if parsed.scheme() != "https" {
        return Err(ApiError::BadRequest(
            "Only HTTPS URLs are allowed in production".into(),
        ));
    }

    // Allow HTTP and HTTPS in development
    #[cfg(debug_assertions)]
    if parsed.scheme() != "http" && parsed.scheme() != "https" {
        return Err(ApiError::BadRequest(
            "Only HTTP/HTTPS URLs are allowed".into(),
        ));
    }

    // Get the host
    let host = parsed
        .host_str()
        .ok_or_else(|| ApiError::BadRequest("URL must have a host".into()))?;

    // Block localhost and loopback
    if host == "localhost"
        || host == "127.0.0.1"
        || host == "::1"
        || host.starts_with("127.")
        || host == "0.0.0.0"
    {
        return Err(ApiError::BadRequest(
            "Cannot connect to localhost/loopback addresses".into(),
        ));
    }

    // Try to parse as IP address
    if let Ok(ip) = host.parse::<IpAddr>() {
        // Block private IP ranges (RFC 1918)
        if is_private_ip(&ip) {
            return Err(ApiError::BadRequest(
                "Cannot connect to private IP addresses".into(),
            ));
        }

        // Block link-local addresses (169.254.0.0/16 and fe80::/10)
        if is_link_local(&ip) {
            return Err(ApiError::BadRequest(
                "Cannot connect to link-local addresses".into(),
            ));
        }
    }

    // Block cloud metadata endpoints
    let metadata_hosts = [
        "169.254.169.254",         // AWS, Azure, GCP
        "metadata.google.internal", // GCP
        "169.254.170.2",           // AWS ECS
    ];
    for metadata in &metadata_hosts {
        if host.eq_ignore_ascii_case(metadata) {
            return Err(ApiError::BadRequest(
                "Cannot connect to cloud metadata endpoints".into(),
            ));
        }
    }

    Ok(())
}

fn is_private_ip(ip: &IpAddr) -> bool {
    match ip {
        IpAddr::V4(ipv4) => {
            let octets = ipv4.octets();
            // 10.0.0.0/8
            octets[0] == 10
                // 172.16.0.0/12
                || (octets[0] == 172 && octets[1] >= 16 && octets[1] <= 31)
                // 192.168.0.0/16
                || (octets[0] == 192 && octets[1] == 168)
        }
        IpAddr::V6(_) => {
            // IPv6 private addresses (fc00::/7)
            // Simplified check
            false
        }
    }
}

fn is_link_local(ip: &IpAddr) -> bool {
    match ip {
        IpAddr::V4(ipv4) => {
            let octets = ipv4.octets();
            // 169.254.0.0/16
            octets[0] == 169 && octets[1] == 254
        }
        IpAddr::V6(ipv6) => {
            // fe80::/10
            let segments = ipv6.segments();
            segments[0] & 0xffc0 == 0xfe80
        }
    }
}

async fn list(
    auth: AuthAdmin,
    State(state): State<AppState>,
) -> Result<Json<Vec<Connector>>, ApiError> {
    crate::require_permission!(auth, state, "connectors.manage");
    let connectors: Vec<Connector> = sqlx::query_as("SELECT * FROM connectors ORDER BY name")
        .fetch_all(&state.db)
        .await?;
    Ok(Json(connectors))
}

async fn get_one(
    auth: AuthAdmin,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<Connector>, ApiError> {
    crate::require_permission!(auth, state, "connectors.manage");
    let connector: Connector = sqlx::query_as("SELECT * FROM connectors WHERE id = $1")
        .bind(id)
        .fetch_optional(&state.db)
        .await?
        .ok_or(ApiError::NotFound("connector not found".into()))?;
    Ok(Json(connector))
}

#[derive(Deserialize)]
struct UpdateConnector {
    config: Value,
}

async fn update(
    auth: AuthAdmin,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(payload): Json<UpdateConnector>,
) -> Result<Json<Value>, ApiError> {
    crate::require_permission!(auth, state, "connectors.manage");

    // Get the existing connector
    let connector: Connector = sqlx::query_as("SELECT * FROM connectors WHERE id = $1")
        .bind(id)
        .fetch_optional(&state.db)
        .await?
        .ok_or(ApiError::NotFound("Connector not found".into()))?;

    // Check if the new config has any non-empty values
    let has_new_values = payload.config.as_object()
        .map(|obj| obj.values().any(|v| {
            v.as_str().map(|s| !s.is_empty()).unwrap_or(true)
        }))
        .unwrap_or(false);

    // If all fields are empty, keep the existing config
    let encrypted = if has_new_values {
        let config_bytes = serde_json::to_vec(&payload.config)
            .map_err(|e| ApiError::BadRequest(e.to_string()))?;
        let key = state.config.encryption_key_bytes();
        vaultscope_common::crypto::encrypt(&key, &config_bytes)
            .map_err(|e| ApiError::Internal(e.to_string()))?
    } else {
        // Keep existing encrypted config
        connector.config_encrypted.clone().unwrap_or_default()
    };

    let result = sqlx::query("UPDATE connectors SET config_encrypted = $1, status = 'connected', updated_at = now() WHERE id = $2")
        .bind(&encrypted)
        .bind(id)
        .execute(&state.db)
        .await?;

    if result.rows_affected() == 0 {
        return Err(ApiError::NotFound("Connector not found".into()));
    }

    Ok(Json(json!({
        "ok": true,
        "message": format!("Configuration for '{}' has been updated successfully", connector.name)
    })))
}

async fn test_connection(
    auth: AuthAdmin,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, ApiError> {
    crate::require_permission!(auth, state, "connectors.manage");

    let connector: Connector = sqlx::query_as("SELECT * FROM connectors WHERE id = $1")
        .bind(id)
        .fetch_optional(&state.db)
        .await?
        .ok_or(ApiError::NotFound("connector not found".into()))?;

    let key = state.config.encryption_key_bytes();
    let encrypted = connector.config_encrypted.as_deref().unwrap_or(&[]);

    tracing::info!("Testing connector: id={}, provider={}, encrypted_config_len={}", id, connector.provider, encrypted.len());

    let config_bytes = vaultscope_common::crypto::decrypt(&key, encrypted)
        .map_err(|e| {
            tracing::error!("Decryption failed: {}", e);
            ApiError::Internal(format!("Failed to decrypt connector config: {}", e))
        })?;

    tracing::info!("Decryption successful, config_bytes_len={}", config_bytes.len());

    let config: Value = serde_json::from_slice(&config_bytes)
        .map_err(|e| {
            tracing::error!("JSON parse failed: {}", e);
            ApiError::Internal(format!("Invalid connector config format: {}", e))
        })?;

    tracing::info!("Config parsed successfully");

    match connector.provider.as_str() {
        "hetzner_cloud" => {
            let token = config
                .get("token")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .trim();

            tracing::info!("Hetzner token extracted, length={}", token.len());

            if token.is_empty() {
                tracing::error!("No API token in config");
                return Err(ApiError::BadRequest("No API token configured".into()));
            }

            // SEC-004: Add timeout to HTTP client
            let client = reqwest::Client::builder()
                .timeout(Duration::from_secs(10))
                .build()
                .map_err(|e| {
                    tracing::error!("Failed to create HTTP client: {}", e);
                    ApiError::Internal(format!("Failed to create HTTP client: {}", e))
                })?;

            tracing::info!("Calling Hetzner API...");

            let res = client
                .get("https://api.hetzner.cloud/v1/locations")
                .bearer_auth(token)
                .send()
                .await
                .map_err(|e| {
                    tracing::error!("Hetzner API HTTP error: {}", e);
                    ApiError::Internal(format!("HTTP error: {}", e))
                })?;

            let status = res.status();
            tracing::info!("Hetzner API response status: {}", status);

            if status.is_success() {
                Ok(Json(
                    json!({ "ok": true, "message": "Hetzner Cloud connection successful" }),
                ))
            } else {
                let status_code = status.as_u16();
                let body = res.text().await.unwrap_or_default();
                tracing::error!("Hetzner API error {}: {}", status_code, body);
                Err(ApiError::Internal(format!(
                    "Hetzner API returned {}: {}",
                    status_code, body
                )))
            }
        }
        "hetzner_robot" => {
            let username = config
                .get("username")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let password = config
                .get("password")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            // SEC-004: Add timeout to HTTP client
            let client = reqwest::Client::builder()
                .timeout(Duration::from_secs(10))
                .build()
                .map_err(|e| ApiError::Internal(format!("Failed to create HTTP client: {}", e)))?;
            let res = client
                .get("https://robot-ws.your-server.de/server")
                .basic_auth(username, Some(password))
                .send()
                .await
                .map_err(|e| ApiError::Internal(format!("HTTP error: {}", e)))?;
            if res.status().is_success() {
                Ok(Json(
                    json!({ "ok": true, "message": "Hetzner Robot connection successful" }),
                ))
            } else {
                Err(ApiError::Internal(format!(
                    "Hetzner Robot returned HTTP {}",
                    res.status()
                )))
            }
        }
        "pterodactyl" => {
            let url = config.get("url").and_then(|v| v.as_str()).unwrap_or("");
            let api_key = config.get("api_key").and_then(|v| v.as_str()).unwrap_or("");

            // SEC-004: Validate URL to prevent SSRF
            validate_connector_url(url)?;

            // Create HTTP client with timeout
            let client = reqwest::Client::builder()
                .timeout(Duration::from_secs(10))
                .build()
                .map_err(|e| ApiError::Internal(format!("Failed to create HTTP client: {}", e)))?;

            let res = client
                .get(format!(
                    "{}/api/application/users",
                    url.trim_end_matches('/')
                ))
                .header("Authorization", format!("Bearer {}", api_key))
                .header("Accept", "application/json")
                .send()
                .await
                .map_err(|e| ApiError::Internal(format!("HTTP error: {}", e)))?;
            if res.status().is_success() {
                Ok(Json(
                    json!({ "ok": true, "message": "Pterodactyl connection successful" }),
                ))
            } else {
                Err(ApiError::Internal(format!(
                    "Pterodactyl returned HTTP {}",
                    res.status()
                )))
            }
        }
        _ => Ok(Json(
            json!({ "ok": true, "message": "Connection test not implemented for this provider" }),
        )),
    }
}

#[derive(Deserialize)]
struct CreateConnector {
    name: Option<String>,
    provider: String,
    config: Value,
}

async fn create(
    auth: AuthAdmin,
    State(state): State<AppState>,
    Json(payload): Json<CreateConnector>,
) -> Result<Json<Connector>, ApiError> {
    crate::require_permission!(auth, state, "connectors.manage");
    let config_bytes =
        serde_json::to_vec(&payload.config).map_err(|e| ApiError::BadRequest(e.to_string()))?;

    let key = state.config.encryption_key_bytes();
    let encrypted = vaultscope_common::crypto::encrypt(&key, &config_bytes)
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    let name = payload
        .name
        .filter(|n| !n.trim().is_empty())
        .unwrap_or_else(|| match payload.provider.as_str() {
            "stripe" => "Stripe Checkout".to_string(),
            "mailcow" => "Mailcow Core".to_string(),
            "hetzner" => "Hetzner Cloud".to_string(),
            "ovh" => "OVH Cloud".to_string(),
            _ => payload.provider.clone(),
        });

    // Check if a connector with this name already exists
    let existing: Option<(String,)> = sqlx::query_as("SELECT name FROM connectors WHERE name = $1")
        .bind(&name)
        .fetch_optional(&state.db)
        .await?;

    if existing.is_some() {
        return Err(ApiError::BadRequest(format!(
            "A connector named '{}' already exists. Please use a different name or update the existing connector.",
            name
        )));
    }

    let connector: Connector = sqlx::query_as(
        "INSERT INTO connectors (id, name, provider, config_encrypted, status, updated_at)
         VALUES ($1, $2, $3, $4, 'connected', now()) RETURNING *",
    )
    .bind(Uuid::now_v7())
    .bind(name)
    .bind(&payload.provider)
    .bind(&encrypted)
    .fetch_one(&state.db)
    .await?;

    Ok(Json(connector))
}

async fn catalog(
    auth: AuthAdmin,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, ApiError> {
    crate::require_permission!(auth, state, "connectors.manage");
    let connector: Connector = sqlx::query_as("SELECT * FROM connectors WHERE id = $1")
        .bind(id)
        .fetch_optional(&state.db)
        .await?
        .ok_or(ApiError::NotFound("connector not found".into()))?;

    let key = state.config.encryption_key_bytes();
    let encrypted = connector.config_encrypted.as_deref().unwrap_or(&[]);
    let config_bytes = vaultscope_common::crypto::decrypt(&key, encrypted)
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    let config: Value =
        serde_json::from_slice(&config_bytes).map_err(|e| ApiError::Internal(e.to_string()))?;

    if connector.provider == "hetzner_cloud" {
        let api_key = config
            .get("token")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim();

        let client = reqwest::Client::new();
        let res = client
            .get("https://api.hetzner.cloud/v1/server_types")
            .bearer_auth(api_key)
            .send()
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?;

        if !res.status().is_success() {
            return Err(ApiError::Internal(
                "failed to fetch catalog from provider".into(),
            ));
        }

        let data: Value = res
            .json()
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?;
        let mut products = vec![];
        if let Some(server_types) = data.get("server_types").and_then(|v| v.as_array()) {
            for st in server_types {
                products.push(json!({
                    "id": st.get("id"),
                    "name": st.get("name"),
                    "cores": st.get("cores"),
                    "ram": st.get("memory"),
                    "disk": st.get("disk"),
                }));
            }
        }
        return Ok(Json(json!(products)));
    }

    Ok(Json(json!([])))
}
