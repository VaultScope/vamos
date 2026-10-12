use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum PowerAction {
    Start,
    Stop,
    Restart,
    Reset,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProvisionParams {
    #[serde(default)]
    pub server_type: String,
    #[serde(default)]
    pub location: String,
    #[serde(default)]
    pub image: String,
    #[serde(default)]
    pub ssh_keys: Vec<String>,
    pub name: String,
    pub extra: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProvisionResult {
    pub resource_id: String,
    pub ip: String,
    pub hostname: String,
    pub raw_response: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReinstallParams {
    pub image: String,
    pub ssh_keys: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResourceStatus {
    Running,
    Stopped,
    Starting,
    Deleting,
    Unknown,
}
