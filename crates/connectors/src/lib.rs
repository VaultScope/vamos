pub mod hetzner_cloud;

use async_trait::async_trait;
use thiserror::Error;
use vaultscope_common::types::{PowerAction, ProvisionParams, ProvisionResult, ReinstallParams, ResourceStatus};

#[derive(Debug, Error)]
pub enum ConnectorError {
    #[error("provider API error: {0}")]
    Api(String),
    #[error("authentication failed")]
    AuthFailed,
    #[error("resource not found: {0}")]
    NotFound(String),
    #[error("rate limited")]
    RateLimited,
    #[error("request failed: {0}")]
    Request(#[from] reqwest::Error),
    #[error("configuration missing or invalid")]
    BadConfig,
}

pub type ConnectorResult<T> = Result<T, ConnectorError>;

#[async_trait]
pub trait ProvisioningProvider: Send + Sync {
    fn name(&self) -> &'static str;

    async fn test_connection(&self) -> ConnectorResult<()>;

    async fn provision(&self, params: &ProvisionParams) -> ConnectorResult<ProvisionResult>;

    async fn power_action(&self, resource_id: &str, action: PowerAction) -> ConnectorResult<()>;

    async fn status(&self, resource_id: &str) -> ConnectorResult<ResourceStatus>;

    async fn delete(&self, resource_id: &str) -> ConnectorResult<()>;

    async fn reinstall(&self, resource_id: &str, params: &ReinstallParams) -> ConnectorResult<()>;
}
