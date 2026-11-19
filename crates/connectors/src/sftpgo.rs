use async_trait::async_trait;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::json;
use vaultscope_common::types::{
    PowerAction, ProvisionParams, ProvisionResult, ReinstallParams, ResourceStatus,
};

use crate::{ConnectorError, ConnectorResult, ProvisioningProvider};

pub struct SftpGo {
    client: Client,
    base_url: String,
    admin_user: String,
    admin_pass: String,
}

impl SftpGo {
    pub fn new(base_url: String, admin_user: String, admin_pass: String) -> Self {
        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(20))
            .build()
            .unwrap_or_else(|_| Client::new());
        Self {
            client,
            base_url,
            admin_user,
            admin_pass,
        }
    }

    async fn get_token(&self) -> ConnectorResult<String> {
        // Authenticate with SFTPGo to get a JWT token
        let url = format!("{}/api/v2/token", self.base_url.trim_end_matches('/'));
        let resp = self.client.get(&url).basic_auth(&self.admin_user, Some(&self.admin_pass)).send().await?;

        if !resp.status().is_success() {
            return Err(ConnectorError::AuthFailed);
        }

        #[derive(Deserialize)]
        struct TokenResp {
            access_token: String,
        }

        let token_resp: TokenResp = resp.json().await?;
        Ok(token_resp.access_token)
    }

    async fn request<T: for<'de> Deserialize<'de>>(
        &self,
        method: reqwest::Method,
        path: &str,
        body: Option<&impl Serialize>,
    ) -> ConnectorResult<T> {
        let token = self.get_token().await?;
        let url = format!("{}{}", self.base_url.trim_end_matches('/'), path);
        let mut req = self
            .client
            .request(method, &url)
            .header("Authorization", format!("Bearer {}", token));

        if let Some(b) = body {
            req = req.json(b);
        }

        let resp = req.send().await?;
        let status = resp.status();

        if status == reqwest::StatusCode::UNAUTHORIZED {
            return Err(ConnectorError::AuthFailed);
        }
        
        if status == reqwest::StatusCode::NOT_FOUND {
            return Err(ConnectorError::NotFound(path.to_string()));
        }

        if !status.is_success() {
            let err_text = resp.text().await.unwrap_or_default();
            return Err(ConnectorError::Api(format!("Status {}: {}", status, err_text)));
        }

        let json = resp.json::<T>().await?;
        Ok(json)
    }
}

/// Generates a collision-free SFTPGo username of the form `vs_<cust6>_<rand8>`
/// safely handling short, empty, `"unknown"`, or non-ASCII customer IDs.
pub fn generate_sftpgo_username(customer_id: Option<&str>) -> String {
    let clean: String = customer_id
        .unwrap_or("unknown")
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect();
    let tail: String = if clean.is_empty() {
        "unknwn".to_string()
    } else {
        let skip = clean.len().saturating_sub(6);
        clean.chars().skip(skip).collect::<String>().to_ascii_lowercase()
    };
    let rand_suffix = vaultscope_common::crypto::generate_random_secret(8).to_ascii_lowercase();
    format!("vs_{}_{}", tail, rand_suffix)
}

/// Validates an SFTPGo resource identifier (username) before embedding into URL paths.
pub fn validate_resource_id(resource_id: &str) -> ConnectorResult<&str> {
    if resource_id.is_empty()
        || resource_id.len() > 128
        || resource_id == "."
        || resource_id == ".."
        || !resource_id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b'-'))
    {
        return Err(ConnectorError::Api("invalid SFTPGo resource_id".into()));
    }
    Ok(resource_id)
}

#[async_trait]
impl ProvisioningProvider for SftpGo {
    fn name(&self) -> &'static str {
        "sftpgo"
    }

    async fn test_connection(&self) -> ConnectorResult<()> {
        let _ = self.get_token().await?;
        Ok(())
    }

    async fn provision(&self, params: &ProvisionParams) -> ConnectorResult<ProvisionResult> {
        let password = vaultscope_common::crypto::generate_random_secret(24);
        let customer_id = params.extra.get("customer_id").and_then(|v| v.as_str());
        let username = generate_sftpgo_username(customer_id);

        let is_trial = params.extra.get("is_trial").and_then(|v| v.as_bool()).unwrap_or(false);
        let mut quota_bytes: i64 = if is_trial {
            5 * 1024 * 1024 * 1024 // 5 GB trial storage
        } else {
            150 * 1024 * 1024 * 1024 // 150 GB default
        };
        
        if let Some(explicit_quota) = params.extra.get("quota_size").and_then(|v| v.as_i64().or_else(|| v.as_u64().map(|u| u as i64))) {
            quota_bytes = explicit_quota;
        } else if let Some(specs) = params.extra.get("specs") {
            if let Some(gb) = specs.get("storage_gb").and_then(|v| v.as_i64()) {
                if !is_trial {
                    quota_bytes = gb * 1024 * 1024 * 1024;
                }
            }
        }

        let payload = json!({
            "username": username,
            "password": password,
            "status": 1, // active
            "quota_size": quota_bytes,
            "permissions": {
                "/": ["*"]
            }
        });

        let resp: serde_json::Value = self
            .request(reqwest::Method::POST, "/api/v2/users", Some(&payload))
            .await?;

        let sftp_ip = std::env::var("SFTP_PUBLIC_IP").unwrap_or_else(|_| "176.9.0.171".to_string());
        let sftp_domain = std::env::var("SFTP_DOMAIN").unwrap_or_else(|_| "sftp.vaultscope.de".to_string());

        Ok(ProvisionResult {
            resource_id: username.clone(),
            ip: sftp_ip,
            hostname: sftp_domain,
            raw_response: json!({ "password": password, "sftpgo_user": resp }),
        })

    }

    async fn power_action(&self, resource_id: &str, action: PowerAction) -> ConnectorResult<()> {
        let resource_id = validate_resource_id(resource_id)?;
        let mut user: serde_json::Value = self
            .request(reqwest::Method::GET, &format!("/api/v2/users/{}", resource_id), None::<&()>)
            .await?;

        let status = match action {
            PowerAction::Start => 1,
            PowerAction::Stop => 0,
            PowerAction::Restart => return Ok(()),
            PowerAction::Reset => return Ok(()),
        };

        user["status"] = json!(status);

        let _: serde_json::Value = self
            .request(reqwest::Method::PUT, &format!("/api/v2/users/{}", resource_id), Some(&user))
            .await?;

        Ok(())
    }

    async fn status(&self, resource_id: &str) -> ConnectorResult<ResourceStatus> {
        let resource_id = validate_resource_id(resource_id)?;
        let user: serde_json::Value = self
            .request(reqwest::Method::GET, &format!("/api/v2/users/{}", resource_id), None::<&()>)
            .await?;
            
        let status = user["status"].as_i64().unwrap_or(0);
        if status == 1 {
            Ok(ResourceStatus::Running)
        } else {
            Ok(ResourceStatus::Stopped)
        }
    }

    async fn delete(&self, resource_id: &str) -> ConnectorResult<()> {
        let resource_id = validate_resource_id(resource_id)?;
        let _ = self
            .request::<serde_json::Value>(reqwest::Method::DELETE, &format!("/api/v2/users/{}", resource_id), None::<&()>)
            .await;
        Ok(())
    }

    async fn reinstall(&self, _resource_id: &str, _params: &ReinstallParams) -> ConnectorResult<()> {
        Ok(())
    }

    async fn modify(&self, resource_id: &str, params: &serde_json::Value) -> ConnectorResult<()> {
        let resource_id = validate_resource_id(resource_id)?;
        let mut user: serde_json::Value = self
            .request(reqwest::Method::GET, &format!("/api/v2/users/{}", resource_id), None::<&()>)
            .await?;

        if let Some(quota) = params.get("quota_size").and_then(|v| v.as_i64()) {
            user["quota_size"] = json!(quota);
        }

        let _: serde_json::Value = self
            .request(reqwest::Method::PUT, &format!("/api/v2/users/{}", resource_id), Some(&user))
            .await?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sftpgo_username_generation_safe_and_unique() {
        let u_unknown = generate_sftpgo_username(Some("unknown"));
        assert!(u_unknown.starts_with("vs_nknown_"));
        assert!(validate_resource_id(&u_unknown).is_ok());

        let u_none = generate_sftpgo_username(None);
        assert!(u_none.starts_with("vs_nknown_"));

        let u_short = generate_sftpgo_username(Some("ab"));
        assert!(u_short.starts_with("vs_ab_"));

        let u_unicode = generate_sftpgo_username(Some("äöüß"));
        assert!(u_unicode.starts_with("vs_unknwn_"));

        let cust = "01948a2b-1234-7000-8000-abcdef123456";
        let u1 = generate_sftpgo_username(Some(cust));
        let u2 = generate_sftpgo_username(Some(cust));
        assert!(u1.starts_with("vs_123456_"));
        assert!(u2.starts_with("vs_123456_"));
        assert_ne!(u1, u2);
    }

    #[test]
    fn test_sftpgo_validate_resource_id() {
        assert!(validate_resource_id("vs_123456_abcdefgh").is_ok());
        assert!(validate_resource_id("user.name-01").is_ok());

        for bad in ["", ".", "..", "../admin", "foo/bar", "foo?bar=1", "foo bar", "foo\nbar"] {
            assert!(
                validate_resource_id(bad).is_err(),
                "Expected validate_resource_id({bad:?}) to reject unsafe input"
            );
        }
    }
}
