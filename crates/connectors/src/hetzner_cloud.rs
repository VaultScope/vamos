use async_trait::async_trait;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use vaultscope_common::types::{PowerAction, ProvisionParams, ProvisionResult, ReinstallParams, ResourceStatus};

use crate::{ConnectorError, ConnectorResult, ProvisioningProvider};

const BASE_URL: &str = "https://api.hetzner.cloud/v1";

pub struct HetznerCloud {
    client: Client,
    token: String,
}

impl HetznerCloud {
    pub fn new(token: String) -> Self {
        Self {
            client: Client::new(),
            token,
        }
    }

    fn auth_header(&self) -> String {
        format!("Bearer {}", self.token)
    }

    async fn request<T: for<'de> Deserialize<'de>>(
        &self,
        method: reqwest::Method,
        path: &str,
        body: Option<&impl Serialize>,
    ) -> ConnectorResult<T> {
        let url = format!("{}{}", BASE_URL, path);
        let mut req = self.client.request(method, &url)
            .header("Authorization", self.auth_header());

        if let Some(b) = body {
            req = req.json(b);
        }

        let resp = req.send().await?;
        let status = resp.status();

        if status == reqwest::StatusCode::UNAUTHORIZED {
            return Err(ConnectorError::AuthFailed);
        }
        if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
            return Err(ConnectorError::RateLimited);
        }
        if status == reqwest::StatusCode::NOT_FOUND {
            return Err(ConnectorError::NotFound(path.to_string()));
        }
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(ConnectorError::Api(format!("{}: {}", status, text)));
        }

        resp.json().await.map_err(|e| ConnectorError::Api(e.to_string()))
    }
}

#[derive(Deserialize)]
struct ServersResponse {
    servers: Vec<ServerResponse>,
}

#[derive(Deserialize)]
struct CreateServerResponse {
    server: ServerResponse,
}

#[derive(Deserialize)]
struct ServerResponse {
    id: u64,
    name: String,
    status: String,
    public_net: PublicNet,
}

#[derive(Deserialize)]
struct PublicNet {
    ipv4: IpAddress,
}

#[derive(Deserialize)]
struct IpAddress {
    ip: String,
}

#[derive(Serialize)]
struct CreateServerRequest {
    name: String,
    server_type: String,
    location: String,
    image: String,
    ssh_keys: Vec<String>,
}

#[derive(Serialize)]
struct PowerActionRequest {
    #[serde(rename = "type")]
    action_type: String,
}

#[derive(Serialize)]
struct RebuildRequest {
    image: String,
}

#[async_trait]
impl ProvisioningProvider for HetznerCloud {
    fn name(&self) -> &'static str {
        "hetzner_cloud"
    }

    async fn test_connection(&self) -> ConnectorResult<()> {
        let _: ServersResponse = self.request(
            reqwest::Method::GET,
            "/servers?per_page=1",
            None::<&()>,
        ).await?;
        Ok(())
    }

    async fn provision(&self, params: &ProvisionParams) -> ConnectorResult<ProvisionResult> {
        let body = CreateServerRequest {
            name: params.name.clone(),
            server_type: params.server_type.clone(),
            location: params.location.clone(),
            image: params.image.clone(),
            ssh_keys: params.ssh_keys.clone(),
        };

        let resp: CreateServerResponse = self.request(
            reqwest::Method::POST,
            "/servers",
            Some(&body),
        ).await?;

        Ok(ProvisionResult {
            resource_id: resp.server.id.to_string(),
            ip: resp.server.public_net.ipv4.ip,
            hostname: resp.server.name,
            raw_response: serde_json::to_value(&resp.server.id).unwrap_or_default(),
        })
    }

    async fn power_action(&self, resource_id: &str, action: PowerAction) -> ConnectorResult<()> {
        let action_name = match action {
            PowerAction::Start => "poweron",
            PowerAction::Stop => "shutdown",
            PowerAction::Restart => "reboot",
            PowerAction::Reset => "reset",
        };

        let _: serde_json::Value = self.request(
            reqwest::Method::POST,
            &format!("/servers/{}/actions/{}", resource_id, action_name),
            None::<&()>,
        ).await?;
        Ok(())
    }

    async fn status(&self, resource_id: &str) -> ConnectorResult<ResourceStatus> {
        let resp: serde_json::Value = self.request(
            reqwest::Method::GET,
            &format!("/servers/{}", resource_id),
            None::<&()>,
        ).await?;

        let status_str = resp["server"]["status"].as_str().unwrap_or("unknown");
        Ok(match status_str {
            "running" => ResourceStatus::Running,
            "off" => ResourceStatus::Stopped,
            "starting" | "initializing" => ResourceStatus::Starting,
            "deleting" => ResourceStatus::Deleting,
            _ => ResourceStatus::Unknown,
        })
    }

    async fn delete(&self, resource_id: &str) -> ConnectorResult<()> {
        let _: serde_json::Value = self.request(
            reqwest::Method::DELETE,
            &format!("/servers/{}", resource_id),
            None::<&()>,
        ).await?;
        Ok(())
    }

    async fn reinstall(&self, resource_id: &str, params: &ReinstallParams) -> ConnectorResult<()> {
        let body = RebuildRequest {
            image: params.image.clone(),
        };

        let _: serde_json::Value = self.request(
            reqwest::Method::POST,
            &format!("/servers/{}/actions/rebuild", resource_id),
            Some(&body),
        ).await?;
        Ok(())
    }
}
