use std::sync::Arc;

use tracing::info;
use vaultscope_common::types::{PowerAction, ProvisionParams, ReinstallParams};
use vaultscope_connectors::{ConnectorError, ProvisioningProvider};
use vaultscope_db::models::Job;

use crate::runner::ConnectorRegistry;

pub async fn execute(
    job: &Job,
    connectors: &Arc<dyn ConnectorRegistry>,
) -> Result<serde_json::Value, ConnectorError> {
    match job.task.as_str() {
        "provision" => provision(job, connectors).await,
        "power_action" => power_action(job, connectors).await,
        "delete" => delete_resource(job, connectors).await,
        "reinstall" => reinstall(job, connectors).await,
        other => Err(ConnectorError::Api(format!("unknown task: {}", other))),
    }
}

async fn get_connector(
    job: &Job,
    connectors: &Arc<dyn ConnectorRegistry>,
) -> Result<Arc<dyn ProvisioningProvider>, ConnectorError> {
    let connector_id = job.connector_id.ok_or(ConnectorError::BadConfig)?;
    connectors
        .get(&connector_id)
        .ok_or(ConnectorError::NotFound(format!("connector {}", connector_id)))
}

async fn provision(
    job: &Job,
    connectors: &Arc<dyn ConnectorRegistry>,
) -> Result<serde_json::Value, ConnectorError> {
    let connector = get_connector(job, connectors).await?;
    let payload = job.request_payload.as_ref().ok_or(ConnectorError::BadConfig)?;
    let params: ProvisionParams = serde_json::from_value(payload.clone())
        .map_err(|e| ConnectorError::Api(e.to_string()))?;

    info!(connector = connector.name(), server = %params.name, "provisioning server");
    let result = connector.provision(&params).await?;
    serde_json::to_value(&result).map_err(|e| ConnectorError::Api(e.to_string()))
}

async fn power_action(
    job: &Job,
    connectors: &Arc<dyn ConnectorRegistry>,
) -> Result<serde_json::Value, ConnectorError> {
    let connector = get_connector(job, connectors).await?;
    let payload = job.request_payload.as_ref().ok_or(ConnectorError::BadConfig)?;
    let resource_id = payload["resource_id"].as_str().ok_or(ConnectorError::BadConfig)?;
    let action: PowerAction = serde_json::from_value(payload["action"].clone())
        .map_err(|e| ConnectorError::Api(e.to_string()))?;

    connector.power_action(resource_id, action).await?;
    Ok(serde_json::json!({"ok": true}))
}

async fn delete_resource(
    job: &Job,
    connectors: &Arc<dyn ConnectorRegistry>,
) -> Result<serde_json::Value, ConnectorError> {
    let connector = get_connector(job, connectors).await?;
    let payload = job.request_payload.as_ref().ok_or(ConnectorError::BadConfig)?;
    let resource_id = payload["resource_id"].as_str().ok_or(ConnectorError::BadConfig)?;

    connector.delete(resource_id).await?;
    Ok(serde_json::json!({"ok": true}))
}

async fn reinstall(
    job: &Job,
    connectors: &Arc<dyn ConnectorRegistry>,
) -> Result<serde_json::Value, ConnectorError> {
    let connector = get_connector(job, connectors).await?;
    let payload = job.request_payload.as_ref().ok_or(ConnectorError::BadConfig)?;
    let resource_id = payload["resource_id"].as_str().ok_or(ConnectorError::BadConfig)?;
    let params: ReinstallParams = serde_json::from_value(payload["params"].clone())
        .map_err(|e| ConnectorError::Api(e.to_string()))?;

    connector.reinstall(resource_id, &params).await?;
    Ok(serde_json::json!({"ok": true}))
}
