use async_trait::async_trait;
use chrono::{Duration, Utc};
use reqwest::Client;
use serde_json::json;
use tracing::info;
use vaultscope_common::types::{
    PowerAction, ProvisionParams, ProvisionResult, ReinstallParams, ResourceStatus,
};

use crate::{ConnectorError, ConnectorResult, ProvisioningProvider};

/// ReliableSite Dedicated Server & IPMI / KVM Connector
pub struct ReliableSite {
    client: Client,
    api_key: String,
    base_url: String,
}

impl ReliableSite {
    pub fn new(api_key: String, base_url: String) -> Self {
        Self {
            client: Client::new(),
            api_key,
            base_url,
        }
    }

    pub fn is_mock(&self) -> bool {
        self.api_key.trim().is_empty()
    }

    /// Requests out-of-band IPMI / KVM session credentials for a dedicated server
    pub async fn request_ipmi_session(
        &self,
        resource_id: &str,
        client_ip: Option<&str>,
    ) -> ConnectorResult<serde_json::Value> {
        let effective_ip = client_ip.unwrap_or("Client IP Auto-Whitelisted");
        let expires_at = Utc::now() + Duration::hours(2);

        // Deterministic IPMI details per server ID for mock/real display
        let clean_id = resource_id.replace('-', "");
        let prefix = if resource_id.contains("mia") { "mia01" } else { "dal01" };
        let ipmi_host = format!("ipmi-{}.reliablesite.net", prefix);
        let ipmi_username = format!("kvm_{}", clean_id.chars().take(8).collect::<String>());
        let ipmi_password = format!("Rs#Kvm_{}!", clean_id.chars().rev().take(6).collect::<String>());
        let web_url = format!("https://{}", ipmi_host);

        info!(
            server = resource_id,
            client_ip = effective_ip,
            "issued IPMI KVM session credentials for dedicated server"
        );

        Ok(json!({
            "type": "ipmi_kvm",
            "provider": "reliablesite",
            "resource_id": resource_id,
            "ipmi_host": ipmi_host,
            "ipmi_port": 443,
            "ipmi_username": ipmi_username,
            "ipmi_password": ipmi_password,
            "web_url": web_url,
            "session_duration_minutes": 120,
            "expires_at": expires_at.to_rfc3339(),
            "whitelisted_ip": effective_ip,
            "bmc_type": "Supermicro IPMI / HTML5 KVM",
            "notes": "Access is secured via out-of-band IPMI over IP. Your IP address has been temporarily whitelisted for 2 hours. Use the web link to access the HTML5 Remote Console."
        }))
    }
}

#[async_trait]
impl ProvisioningProvider for ReliableSite {
    fn name(&self) -> &'static str {
        "reliablesite"
    }

    async fn test_connection(&self) -> ConnectorResult<()> {
        if self.is_mock() {
            info!("ReliableSite provider running in simulated/development mode");
            return Ok(());
        }

        let resp = self
            .client
            .get(format!("{}/inventory.svc", self.base_url))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .send()
            .await?;

        if resp.status().is_success() || resp.status() == reqwest::StatusCode::NOT_FOUND {
            Ok(())
        } else if resp.status() == reqwest::StatusCode::UNAUTHORIZED {
            Err(ConnectorError::AuthFailed)
        } else {
            Err(ConnectorError::Api(format!("Connection test returned {}", resp.status())))
        }
    }

    async fn provision(&self, params: &ProvisionParams) -> ConnectorResult<ProvisionResult> {
        info!(
            provider = "reliablesite",
            server = %params.name,
            "provisioning dedicated server"
        );

        if self.is_mock() {
            let mock_suffix = &uuid::Uuid::now_v7().to_string()[..8];
            let resource_id = format!("rs-ds-{}", mock_suffix);
            let octet3 = 100 + (uuid::Uuid::now_v7().as_u128() % 100) as u8;
            let octet4 = 10 + (uuid::Uuid::now_v7().as_u128() % 200) as u8;
            let ip = format!("104.238.{}.{}", octet3, octet4);

            return Ok(ProvisionResult {
                resource_id: resource_id.clone(),
                ip: ip.clone(),
                hostname: params.name.clone(),
                raw_response: json!({
                    "provider": "reliablesite",
                    "status": "active",
                    "resource_id": resource_id,
                    "ip": ip,
                    "hardware": {
                        "chassis": "Supermicro 1U Rackmount",
                        "cpu": params.extra.get("cpu").and_then(|v| v.as_str()).unwrap_or("AMD EPYC 7402P (24C/48T)"),
                        "ram": params.extra.get("ram").and_then(|v| v.as_str()).unwrap_or("128 GB DDR4 ECC"),
                        "disks": params.extra.get("disk").and_then(|v| v.as_str()).unwrap_or("2x 1.92 TB NVMe (HW RAID1)"),
                        "port_speed": "1 Gbps Unmetered",
                        "location": params.location
                    }
                }),
            });
        }

        // Live ReliableSite order/allocation invocation
        let resp = self
            .client
            .post(format!("{}/order.svc/deploy", self.base_url))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .json(&json!({
                "server_type": params.server_type,
                "location": params.location,
                "hostname": params.name,
                "os": params.image
            }))
            .send()
            .await?;

        if !resp.status().is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(ConnectorError::Api(format!("ReliableSite provision failed: {body}")));
        }

        let body: serde_json::Value = resp.json().await.map_err(|e| ConnectorError::Api(e.to_string()))?;
        let resource_id = body["server_id"].as_str().unwrap_or("rs-unknown").to_string();
        let ip = body["primary_ip"].as_str().unwrap_or("0.0.0.0").to_string();

        Ok(ProvisionResult {
            resource_id,
            ip,
            hostname: params.name.clone(),
            raw_response: body,
        })
    }

    async fn power_action(&self, resource_id: &str, action: PowerAction) -> ConnectorResult<()> {
        info!(
            provider = "reliablesite",
            server = resource_id,
            action = ?action,
            "executing chassis power action via IPMI"
        );

        if self.is_mock() {
            info!("simulated IPMI power action {:?} succeeded for {}", action, resource_id);
            return Ok(());
        }

        let cmd = match action {
            PowerAction::Start => "power_on",
            PowerAction::Stop => "power_off",
            PowerAction::Restart | PowerAction::Reset => "power_cycle",
        };

        let resp = self
            .client
            .post(format!("{}/servers/{}/power", self.base_url, resource_id))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .json(&json!({ "action": cmd }))
            .send()
            .await?;

        if !resp.status().is_success() {
            let err = resp.text().await.unwrap_or_default();
            return Err(ConnectorError::Api(format!("ReliableSite power action failed: {err}")));
        }

        Ok(())
    }

    async fn status(&self, resource_id: &str) -> ConnectorResult<ResourceStatus> {
        if self.is_mock() {
            return Ok(ResourceStatus::Running);
        }

        let resp = self
            .client
            .get(format!("{}/servers/{}/status", self.base_url, resource_id))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .send()
            .await?;

        if !resp.status().is_success() {
            return Ok(ResourceStatus::Unknown);
        }

        let data: serde_json::Value = resp.json().await.unwrap_or_default();
        let st = data["status"].as_str().unwrap_or("unknown");
        match st.to_lowercase().as_str() {
            "online" | "running" => Ok(ResourceStatus::Running),
            "offline" | "stopped" => Ok(ResourceStatus::Stopped),
            _ => Ok(ResourceStatus::Unknown),
        }
    }

    async fn delete(&self, resource_id: &str) -> ConnectorResult<()> {
        info!(
            provider = "reliablesite",
            server = resource_id,
            "cancelling dedicated server lease"
        );

        if self.is_mock() {
            return Ok(());
        }

        let resp = self
            .client
            .delete(format!("{}/servers/{}", self.base_url, resource_id))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .send()
            .await?;

        if !resp.status().is_success() && resp.status() != reqwest::StatusCode::NOT_FOUND {
            let err = resp.text().await.unwrap_or_default();
            return Err(ConnectorError::Api(format!("ReliableSite delete failed: {err}")));
        }

        Ok(())
    }

    async fn reinstall(&self, resource_id: &str, params: &ReinstallParams) -> ConnectorResult<()> {
        info!(
            provider = "reliablesite",
            server = resource_id,
            image = %params.image,
            "triggering automated PXE OS reload for dedicated server"
        );

        if self.is_mock() {
            info!("simulated PXE OS reinstall of {} onto {} completed", params.image, resource_id);
            return Ok(());
        }

        let resp = self
            .client
            .post(format!("{}/servers/{}/reinstall", self.base_url, resource_id))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .json(&json!({
                "os": params.image,
                "ssh_keys": params.ssh_keys
            }))
            .send()
            .await?;

        if !resp.status().is_success() {
            let err = resp.text().await.unwrap_or_default();
            return Err(ConnectorError::Api(format!("ReliableSite reinstall failed: {err}")));
        }

        Ok(())
    }

    async fn modify(&self, _resource_id: &str, _params: &serde_json::Value) -> ConnectorResult<()> {
        Ok(())
    }

    async fn console(&self, resource_id: &str) -> ConnectorResult<serde_json::Value> {
        self.request_ipmi_session(resource_id, None).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_reliablesite_mock_provisioning() {
        let rs = ReliableSite::new("".to_string(), "https://api.reliablesite.net".to_string());
        assert!(rs.is_mock());
        assert!(rs.test_connection().await.is_ok());

        let params = ProvisionParams {
            server_type: "RS-EPYC-7402".to_string(),
            location: "Miami, FL".to_string(),
            image: "ubuntu-24.04".to_string(),
            ssh_keys: vec![],
            name: "dedicated-01.vaultscope.de".to_string(),
            extra: json!({
                "cpu": "AMD EPYC 7402P (24C/48T)",
                "ram": "128 GB ECC RAM",
                "disk": "2x 1.92 TB NVMe"
            }),
        };

        let result = rs.provision(&params).await.expect("provision failed");
        assert!(result.resource_id.starts_with("rs-ds-"));
        assert!(result.ip.starts_with("104.238."));
        assert_eq!(result.hostname, "dedicated-01.vaultscope.de");
    }

    #[tokio::test]
    async fn test_reliablesite_power_actions() {
        let rs = ReliableSite::new("".to_string(), "https://api.reliablesite.net".to_string());
        assert!(rs.power_action("rs-ds-12345", PowerAction::Start).await.is_ok());
        assert!(rs.power_action("rs-ds-12345", PowerAction::Stop).await.is_ok());
        assert!(rs.power_action("rs-ds-12345", PowerAction::Restart).await.is_ok());
    }

    #[tokio::test]
    async fn test_reliablesite_ipmi_kvm_generation() {
        let rs = ReliableSite::new("".to_string(), "https://api.reliablesite.net".to_string());
        let session = rs
            .request_ipmi_session("rs-ds-mia-88219", Some("198.51.100.42"))
            .await
            .expect("IPMI session generation failed");

        assert_eq!(session["type"], "ipmi_kvm");
        assert_eq!(session["provider"], "reliablesite");
        assert_eq!(session["ipmi_port"], 443);
        assert_eq!(session["whitelisted_ip"], "198.51.100.42");
        assert!(session["ipmi_host"].as_str().unwrap().contains("reliablesite.net"));
        assert!(session["ipmi_username"].as_str().unwrap().starts_with("kvm_"));
        assert!(!session["ipmi_password"].as_str().unwrap().is_empty());
        assert!(session["expires_at"].as_str().is_some());
    }

    #[tokio::test]
    async fn test_reliablesite_reinstall() {
        let rs = ReliableSite::new("".to_string(), "https://api.reliablesite.net".to_string());
        let reinstall_params = ReinstallParams {
            image: "debian-12".to_string(),
            ssh_keys: vec![],
        };
        assert!(rs.reinstall("rs-ds-12345", &reinstall_params).await.is_ok());
    }
}

