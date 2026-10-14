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
        Self {
            client: Client::new(),
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
        let timestamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let password = format!("{:x}", timestamp)[..16].to_string();
            
        let customer_id = params.extra.get("customer_id").and_then(|v| v.as_str()).unwrap_or("unknown");
        let username = format!("vs_{}", &customer_id.to_string().replace("-", "")[..8]);

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
        let _ = self
            .request::<serde_json::Value>(reqwest::Method::DELETE, &format!("/api/v2/users/{}", resource_id), None::<&()>)
            .await;
        Ok(())
    }

    async fn reinstall(&self, _resource_id: &str, _params: &ReinstallParams) -> ConnectorResult<()> {
        Ok(())
    }

    async fn modify(&self, resource_id: &str, params: &serde_json::Value) -> ConnectorResult<()> {
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
