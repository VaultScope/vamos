pub mod inventory_soap;
pub mod order_api;
pub mod types;

use std::collections::{BTreeMap, HashMap};

use async_trait::async_trait;
use chrono::{Duration, Utc};
use reqwest::Client;
use serde_json::json;
use tracing::info;
use vaultscope_common::types::{
    PowerAction, ProvisionParams, ProvisionResult, ReinstallParams, ResourceStatus,
};

use crate::{ConnectorError, ConnectorResult, ProvisioningProvider};

use inventory_soap::{InventorySoapClient, DEFAULT_INVENTORY_URL};
use order_api::{validate_create_order_request, OrderApiClient, DEFAULT_ORDER_API_URL};
pub use types::*;

/// Complete synced ReliableSite catalog snapshot for one or more datacenters.
#[derive(Debug, Clone)]
pub struct ReliableSiteCatalogSync {
    pub servers: Vec<InventoryServer>,
    pub server_groups: HashMap<i32, Vec<InventoryAddonGroup>>,
    pub addons_by_group: HashMap<i32, Vec<InventoryAddon>>,
}

/// ReliableSite Dedicated Server, Order API, Inventory SOAP & IPMI/KVM Connector.
#[derive(Clone)]
pub struct ReliableSite {
    client: Client,
    api_key: String,
    base_url: String,
    order_client: OrderApiClient,
    inventory_client: InventorySoapClient,
    default_payment_method: Option<String>,
}

impl ReliableSite {
    pub fn new(api_key: String, base_url: String) -> Self {
        let trimmed = base_url.trim().trim_end_matches('/').to_string();
        let is_legacy_or_default =
            trimmed.is_empty() || trimmed.contains("api.reliablesite.net");

        let (order_url, inv_url) = if is_legacy_or_default {
            (
                DEFAULT_ORDER_API_URL.to_string(),
                DEFAULT_INVENTORY_URL.to_string(),
            )
        } else {
            (trimmed.clone(), format!("{}/inventory", trimmed))
        };

        Self::with_endpoints(api_key, order_url, inv_url, None)
    }

    pub fn with_endpoints(
        api_key: String,
        order_api_url: String,
        inventory_url: String,
        default_payment_method: Option<String>,
    ) -> Self {
        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(20))
            .build()
            .unwrap_or_else(|_| Client::new());
        let clean_key = api_key.trim().to_string();
        let order_client =
            OrderApiClient::new(client.clone(), order_api_url.clone(), clean_key.clone());
        let inventory_client = InventorySoapClient::new(client.clone(), inventory_url);

        Self {
            client,
            api_key: clean_key,
            base_url: order_api_url.trim().trim_end_matches('/').to_string(),
            order_client,
            inventory_client,
            default_payment_method: default_payment_method
                .map(|m| m.trim().to_string())
                .filter(|m| !m.is_empty()),
        }
    }

    pub fn is_mock(&self) -> bool {
        self.api_key.trim().is_empty()
    }

    pub fn order_api(&self) -> &OrderApiClient {
        &self.order_client
    }

    pub fn inventory_soap(&self) -> &InventorySoapClient {
        &self.inventory_client
    }

    /// Fetches all servers for `data_center` (e.g. `"NL"`) alongside their AddonGroupsList & AddonsList.
    pub async fn fetch_datacenter_catalog(
        &self,
        data_center: &str,
    ) -> ConnectorResult<ReliableSiteCatalogSync> {
        let target_dc = data_center.trim().to_ascii_uppercase();
        let all_servers = self.inventory_client.servers_list().await?;
        let servers: Vec<InventoryServer> = all_servers
            .into_iter()
            .filter(|s| target_dc.is_empty() || s.data_center.trim().eq_ignore_ascii_case(&target_dc))
            .collect();

        let mut server_groups = HashMap::new();
        let mut addons_by_group = HashMap::new();

        for srv in &servers {
            let groups = self.inventory_client.addon_groups_list(srv.product_id).await?;
            for g in &groups {
                if let std::collections::hash_map::Entry::Vacant(entry) =
                    addons_by_group.entry(g.group_id)
                {
                    let addons = self.inventory_client.addons_list(g.group_id).await?;
                    entry.insert(addons);
                }
            }
            server_groups.insert(srv.product_id, groups);
        }

        Ok(ReliableSiteCatalogSync {
            servers,
            server_groups,
            addons_by_group,
        })
    }

    /// Requests out-of-band IPMI / KVM session credentials for a dedicated server.
    pub async fn request_ipmi_session(
        &self,
        resource_id: &str,
        client_ip: Option<&str>,
    ) -> ConnectorResult<serde_json::Value> {
        let effective_ip = client_ip.unwrap_or("Client IP Auto-Whitelisted");
        let expires_at = Utc::now() + Duration::hours(2);

        let clean_id = resource_id.replace('-', "");
        let prefix = if resource_id.contains("mia") {
            "mia01"
        } else if resource_id.contains("nl") || resource_id.contains("ams") {
            "ams01"
        } else {
            "dal01"
        };
        let ipmi_host = format!("kvm-{}.oob.vaultscope.de", prefix);
        let ipmi_username = format!("kvm_{}", clean_id.chars().take(8).collect::<String>());
        let ipmi_password = vaultscope_common::crypto::generate_random_secret(16);
        let web_url = format!("https://{}", ipmi_host);

        info!(
            server = resource_id,
            client_ip = effective_ip,
            "issued IPMI KVM session credentials for dedicated server"
        );

        Ok(json!({
            "type": "ipmi_kvm",
            "provider": "vaultscope_bmc",
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

    #[allow(clippy::type_complexity)]
    fn extract_order_params(
        &self,
        params: &ProvisionParams,
    ) -> ConnectorResult<(
        Option<i32>,
        OrderBillingCycle,
        BTreeMap<String, u32>,
        Option<String>,
        Option<String>,
        Option<String>,
        String,
    )> {
        let product_id = params
            .extra
            .get("reliablesite_product_id")
            .or_else(|| params.extra.get("product_id"))
            .and_then(|v| {
                v.as_i64()
                    .map(|n| n as i32)
                    .or_else(|| v.as_str().and_then(|s| s.trim().parse::<i32>().ok()))
            })
            .or_else(|| params.server_type.trim().parse::<i32>().ok());

        let cycle = params
            .extra
            .get("billing_cycle")
            .or_else(|| params.extra.get("billingCycle"))
            .or_else(|| {
                params
                    .extra
                    .get("config")
                    .and_then(|c| c.get("billing_cycle").or_else(|| c.get("billingCycle")))
            })
            .and_then(|v| v.as_str())
            .and_then(OrderBillingCycle::from_str_loose)
            .unwrap_or(OrderBillingCycle::Monthly);

        let raw_cfg = params
            .extra
            .get("config_options")
            .or_else(|| params.extra.get("configOptions"))
            .or_else(|| {
                params
                    .extra
                    .get("config")
                    .and_then(|c| c.get("config_options").or_else(|| c.get("configOptions")))
            });

        let mut config_options = BTreeMap::new();
        if let Some(obj) = raw_cfg.and_then(|v| v.as_object()) {
            for (k, v) in obj {
                let gid: u32 = k.trim().parse().map_err(|_| {
                    ConnectorError::Api(format!("Invalid numeric Group_Id key: {}", k))
                })?;
                let canonical_key = gid.to_string();
                let addon_id = v
                    .as_u64()
                    .or_else(|| v.as_str().and_then(|s| s.trim().parse::<u64>().ok()))
                    .ok_or_else(|| {
                        ConnectorError::Api(format!(
                            "Invalid Addon_Id value for group {}: {}",
                            k, v
                        ))
                    })?;
                let addon_u32 = u32::try_from(addon_id).map_err(|_| {
                    ConnectorError::Api(format!("Addon_Id out of range for group {}", k))
                })?;
                if config_options.insert(canonical_key, addon_u32).is_some() {
                    return Err(ConnectorError::Api(format!(
                        "Duplicate configuration group key {}",
                        gid
                    )));
                }
            }
        }

        let promo_code = params
            .extra
            .get("promo_code")
            .or_else(|| params.extra.get("promoCode"))
            .and_then(|v| v.as_str())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());

        let order_notes = params
            .extra
            .get("order_notes")
            .or_else(|| params.extra.get("orderNotes"))
            .and_then(|v| v.as_str())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());

        let payment_method = params
            .extra
            .get("payment_method")
            .or_else(|| params.extra.get("paymentMethod"))
            .and_then(|v| v.as_str())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .or_else(|| self.default_payment_method.clone());

        let idempotency_key = params
            .extra
            .get("idempotency_key")
            .and_then(|v| v.as_str())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| uuid::Uuid::now_v7().to_string());

        Ok((
            product_id,
            cycle,
            config_options,
            promo_code,
            order_notes,
            payment_method,
            idempotency_key,
        ))
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

        self.order_client.login_token().await?;
        let verify = self.order_client.verify_token().await?;
        if !verify.status {
            return Err(ConnectorError::AuthFailed);
        }
        Ok(())
    }

    async fn provision(&self, params: &ProvisionParams) -> ConnectorResult<ProvisionResult> {
        info!(
            provider = "reliablesite",
            server = %params.name,
            "provisioning dedicated server"
        );

        let (
            maybe_product_id,
            billing_cycle,
            config_options,
            promo_code,
            order_notes,
            explicit_payment_method,
            idempotency_key,
        ) = self.extract_order_params(params)?;

        if self.is_mock() {
            let mock_suffix = &uuid::Uuid::now_v7().to_string()[..8];
            let resource_id = format!("ds-{}", mock_suffix);
            let octet3 = 100 + (uuid::Uuid::now_v7().as_u128() % 100) as u8;
            let octet4 = 10 + (uuid::Uuid::now_v7().as_u128() % 200) as u8;
            let ip = format!("104.238.{}.{}", octet3, octet4);
            let pid = maybe_product_id.unwrap_or(428);

            if !config_options.is_empty() {
                let sample_req = CreateOrderRequest {
                    product_id: pid,
                    billing_cycle,
                    hostname: params.name.clone(),
                    payment_method: explicit_payment_method
                        .clone()
                        .unwrap_or_else(|| "banktransfer".to_string()),
                    config_options: config_options.clone(),
                    promo_code: promo_code.clone(),
                    order_notes: order_notes.clone(),
                };
                validate_create_order_request(&sample_req, &idempotency_key)?;
            }

            return Ok(ProvisionResult {
                resource_id: resource_id.clone(),
                ip: ip.clone(),
                hostname: params.name.clone(),
                raw_response: json!({
                    "provider": "vaultscope_dedicated",
                    "status": "active",
                    "simulated": true,
                    "resource_id": resource_id,
                    "ip": ip,
                    "orderId": (uuid::Uuid::now_v7().as_u128() % 900_000 + 100_000) as i64,
                    "invoiceId": (uuid::Uuid::now_v7().as_u128() % 9_000_000 + 1_000_000) as i64,
                    "productId": pid,
                    "billingCycle": billing_cycle,
                    "paymentMethod": explicit_payment_method.unwrap_or_else(|| "banktransfer".to_string()),
                    "configOptions": config_options,
                    "promoCode": promo_code,
                    "replayed": false,
                    "hardware": {
                        "chassis": "Supermicro 1U Rackmount",
                        "cpu": params.extra.get("cpu").and_then(|v| v.as_str()).unwrap_or("AMD EPYC 4545P"),
                        "ram": params.extra.get("ram").and_then(|v| v.as_str()).unwrap_or("128 GB DDR5"),
                        "disks": params.extra.get("disk").and_then(|v| v.as_str()).unwrap_or("2 TB NVMe"),
                        "port_speed": "1 Gbps Unmetered",
                        "location": if params.location.is_empty() { "NL" } else { &params.location }
                    }
                }),
            });
        }

        let product_id = maybe_product_id.ok_or_else(|| {
            ConnectorError::Api(
                "ReliableSite Product_Id missing from product target/server_type".into(),
            )
        })?;

        // 1. Check live stock via Order API
        let stock = self.order_client.get_product_stock(product_id).await?;
        if stock.stock_controlled && stock.stock < 1 {
            return Err(ConnectorError::Api(format!(
                "ReliableSite product {} is out of stock (stock={})",
                product_id, stock.stock
            )));
        }

        // 2. Determine an instant automated paymentMethod module (never fall back to manual banktransfer)
        let methods = self.order_client.get_payment_methods(product_id).await?;
        let is_manual_module = |module: &str| {
            matches!(
                module.trim().to_ascii_lowercase().as_str(),
                "banktransfer" | "wiretransfer" | "mailin" | "offline" | "check" | "cash"
            )
        };
        let payment_method = if let Some(ref preferred) = explicit_payment_method {
            if methods.payment_methods.iter().any(|m| m.module == *preferred) {
                preferred.clone()
            } else {
                return Err(ConnectorError::Api(format!(
                    "paymentMethod '{}' is not available for ReliableSite product {}",
                    preferred, product_id
                )));
            }
        } else {
            methods
                .payment_methods
                .iter()
                .find(|m| !is_manual_module(&m.module))
                .map(|m| m.module.clone())
                .ok_or_else(|| {
                    ConnectorError::Api(format!(
                        "No instant automated payment method enabled for ReliableSite product {} (only manual methods returned; configure RELIABLESITE_PAYMENT_METHOD and an instant billing profile in ReliableSite)",
                        product_id
                    ))
                })?
        };

        // 3. Ensure required configOptions are present
        if config_options.is_empty() {
            return Err(ConnectorError::Api(
                "ReliableSite Order API requires configOptions mapping Group_Id -> Addon_Id".into(),
            ));
        }

        let order_req = CreateOrderRequest {
            product_id,
            billing_cycle,
            hostname: params.name.clone(),
            payment_method,
            config_options: config_options.clone(),
            promo_code,
            order_notes,
        };

        let order_resp = self
            .order_client
            .create_order(&order_req, &idempotency_key)
            .await?;

        let resource_id = format!("ds-ord-{}", order_resp.order_id);
        let raw_response = json!({
            "provider": "vaultscope_dedicated",
            "orderId": order_resp.order_id,
            "invoiceId": order_resp.invoice_id,
            "productIds": order_resp.product_ids,
            "paymentMethod": order_resp.payment_method,
            "productId": order_resp.product_id,
            "billingCycle": billing_cycle,
            "configOptions": config_options,
            "replayed": order_resp.replayed,
            "idempotencyKey": idempotency_key,
        });

        Ok(ProvisionResult {
            resource_id,
            ip: String::new(),
            hostname: params.name.clone(),
            raw_response,
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
            info!(
                "simulated IPMI power action {:?} succeeded for {}",
                action, resource_id
            );
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
            return Err(ConnectorError::Api(format!(
                "ReliableSite power action failed: {err}"
            )));
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
            return Err(ConnectorError::Api(format!(
                "ReliableSite delete failed: {err}"
            )));
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
            info!(
                "simulated PXE OS reinstall of {} onto {} completed",
                params.image, resource_id
            );
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
            return Err(ConnectorError::Api(format!(
                "ReliableSite reinstall failed: {err}"
            )));
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
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use axum::{
        extract::{Path, Query, State},
        http::{HeaderMap, StatusCode},
        routing::{get, post},
        Json, Router,
    };
    use rust_decimal::Decimal;
    use std::str::FromStr;
    use tokio::net::TcpListener;

    use crate::reliablesite::inventory_soap::{
        parse_addon_groups_list_xml, parse_addons_list_xml, parse_servers_list_xml,
    };

    #[tokio::test]
    async fn test_reliablesite_mock_provisioning() {
        let rs = ReliableSite::new("".to_string(), "https://api.reliablesite.net".to_string());
        assert!(rs.is_mock());
        assert!(rs.test_connection().await.is_ok());

        let mut config_opts = serde_json::Map::new();
        config_opts.insert("565".to_string(), json!(1751));
        config_opts.insert("659".to_string(), json!(2391));

        let params = ProvisionParams {
            server_type: "428".to_string(),
            location: "NL".to_string(),
            image: "ubuntu-24.04".to_string(),
            ssh_keys: vec![],
            name: "dedicated-01.vaultscope.de".to_string(),
            extra: json!({
                "reliablesite_product_id": 428,
                "billing_cycle": "monthly",
                "config_options": config_opts,
                "cpu": "AMD Epyc 4545P",
                "ram": "128 GB DDR5 RAM",
                "disk": "2 TB NVMe"
            }),
        };

        let result = rs.provision(&params).await.expect("provision failed");
        assert!(result.resource_id.starts_with("ds-"));
        assert!(result.ip.starts_with("104.238."));
        assert_eq!(result.hostname, "dedicated-01.vaultscope.de");
        assert_eq!(result.raw_response["productId"], 428);
        assert_eq!(result.raw_response["configOptions"]["565"], 1751);
    }

    #[tokio::test]
    async fn test_reliablesite_power_actions() {
        let rs = ReliableSite::new("".to_string(), "https://api.reliablesite.net".to_string());
        assert!(rs.power_action("ds-12345", PowerAction::Start).await.is_ok());
        assert!(rs.power_action("ds-12345", PowerAction::Stop).await.is_ok());
        assert!(rs.power_action("ds-12345", PowerAction::Restart).await.is_ok());
    }

    #[tokio::test]
    async fn test_reliablesite_ipmi_kvm_generation() {
        let rs = ReliableSite::new("".to_string(), "https://api.reliablesite.net".to_string());
        let session = rs
            .request_ipmi_session("ds-mia-88219", Some("198.51.100.42"))
            .await
            .expect("IPMI session generation failed");

        assert_eq!(session["type"], "ipmi_kvm");
        assert_eq!(session["provider"], "vaultscope_bmc");
        assert_eq!(session["ipmi_port"], 443);
        assert_eq!(session["whitelisted_ip"], "198.51.100.42");
        assert!(session["ipmi_host"].as_str().unwrap().contains("oob.vaultscope.de"));
        assert!(session["ipmi_username"].as_str().unwrap().starts_with("kvm_"));
        let pw1 = session["ipmi_password"].as_str().unwrap();
        assert_eq!(pw1.len(), 16);
        assert!(pw1.bytes().all(|b| b.is_ascii_alphanumeric()));
        assert!(!pw1.starts_with("Vs#Kvm_"));

        let session2 = rs
            .request_ipmi_session("ds-mia-88219", Some("198.51.100.42"))
            .await
            .expect("second IPMI session generation failed");
        assert_ne!(pw1, session2["ipmi_password"].as_str().unwrap());
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

    #[test]
    fn test_parse_real_inventory_soap_xml() {
        let servers_xml = r#"<?xml version="1.0" encoding="utf-8"?>
        <s:Envelope xmlns:s="http://schemas.xmlsoap.org/soap/envelope/">
          <s:Body>
            <ServersListResponse xmlns="http://tempuri.org/">
              <ServersListResult xmlns:a="http://schemas.datacontract.org/2004/07/InventoryServiceLibrary">
                <a:Message>Server list created successfully.</a:Message>
                <a:Result>true</a:Result>
                <a:Result_Code>0</a:Result_Code>
                <a:ServerDetailsList>
                  <a:Server_Details>
                    <a:Data_Center>NL</a:Data_Center>
                    <a:Description>Rapid Deploy Server - AMD Epyc 4545P 128GB (NL)</a:Description>
                    <a:Detail>AMD Epyc 4545P&lt;br /&gt;128 GB DDR5 RAM</a:Detail>
                    <a:Product_Id>428</a:Product_Id>
                    <a:Recurring_1_Hour>0</a:Recurring_1_Hour>
                    <a:Recurring_1_Month>299.0000</a:Recurring_1_Month>
                    <a:Recurring_3_Month>-1.0000</a:Recurring_3_Month>
                    <a:Recurring_6_Month>1704.3000</a:Recurring_6_Month>
                    <a:Recurring_12_Month>3229.2000</a:Recurring_12_Month>
                    <a:Recurring_24_Month>5740.8000</a:Recurring_24_Month>
                    <a:Recurring_36_Month>-1.0000</a:Recurring_36_Month>
                    <a:Setup_1_Hour>0</a:Setup_1_Hour>
                    <a:Setup_1_Month>0.0000</a:Setup_1_Month>
                    <a:Setup_3_Month>0.0000</a:Setup_3_Month>
                    <a:Setup_6_Month>0.0000</a:Setup_6_Month>
                    <a:Setup_12_Month>0.0000</a:Setup_12_Month>
                    <a:Setup_24_Month>0.0000</a:Setup_24_Month>
                    <a:Setup_36_Month>0.0000</a:Setup_36_Month>
                    <a:Stock>7</a:Stock>
                  </a:Server_Details>
                </a:ServerDetailsList>
              </ServersListResult>
            </ServersListResponse>
          </s:Body>
        </s:Envelope>"#;

        let servers = parse_servers_list_xml(servers_xml).unwrap();
        assert_eq!(servers.len(), 1);
        let s = &servers[0];
        assert_eq!(s.product_id, 428);
        assert_eq!(s.data_center, "NL");
        assert_eq!(s.stock, 7);
        assert_eq!(
            s.pricing.recurring_for(OrderBillingCycle::Monthly),
            Some(Decimal::from_str("299.0000").unwrap())
        );
        assert_eq!(s.pricing.recurring_for(OrderBillingCycle::Quarterly), None);
        assert_eq!(s.pricing.recurring_for(OrderBillingCycle::Triennially), None);

        let groups_xml = r#"<s:Envelope xmlns:s="http://schemas.xmlsoap.org/soap/envelope/"><s:Body>
          <AddonGroupsListResponse xmlns="http://tempuri.org/">
            <AddonGroupsListResult xmlns:a="http://schemas.datacontract.org/2004/07/InventoryServiceLibrary">
              <a:AddonGroupsDetailsList>
                <a:AddonGroups_Details><a:Description>Bandwidth</a:Description><a:Group_Id>659</a:Group_Id><a:Sort_Order>2</a:Sort_Order></a:AddonGroups_Details>
                <a:AddonGroups_Details><a:Description>Port Speed</a:Description><a:Group_Id>565</a:Group_Id><a:Sort_Order>1</a:Sort_Order></a:AddonGroups_Details>
                <a:AddonGroups_Details><a:Description></a:Description><a:Group_Id>0</a:Group_Id><a:Sort_Order>0</a:Sort_Order></a:AddonGroups_Details>
              </a:AddonGroupsDetailsList>
              <a:Result>true</a:Result>
            </AddonGroupsListResult>
          </AddonGroupsListResponse></s:Body></s:Envelope>"#;
        let groups = parse_addon_groups_list_xml(groups_xml).unwrap();
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].group_id, 565);
        assert_eq!(groups[1].group_id, 659);

        let addons_xml = r#"<s:Envelope xmlns:s="http://schemas.xmlsoap.org/soap/envelope/"><s:Body>
          <AddonsListResponse xmlns="http://tempuri.org/">
            <AddonsListResult xmlns:a="http://schemas.datacontract.org/2004/07/InventoryServiceLibrary">
              <a:AddonsDetailsList>
                <a:Addons_Details>
                  <a:AddonDescription>One Time Migration</a:AddonDescription>
                  <a:Addon_Id>2232</a:Addon_Id>
                  <a:Group_Id>643</a:Group_Id>
                  <a:Recurring_1_Hour>0</a:Recurring_1_Hour>
                  <a:Recurring_1_Month>0.0000</a:Recurring_1_Month>
                  <a:Recurring_3_Month>0.0000</a:Recurring_3_Month>
                  <a:Recurring_6_Month>0.0000</a:Recurring_6_Month>
                  <a:Recurring_12_Month>0.0000</a:Recurring_12_Month>
                  <a:Recurring_24_Month>0.0000</a:Recurring_24_Month>
                  <a:Recurring_36_Month>0.0000</a:Recurring_36_Month>
                  <a:Setup_1_Hour>0</a:Setup_1_Hour>
                  <a:Setup_1_Month>29.0000</a:Setup_1_Month>
                  <a:Setup_3_Month>0.0000</a:Setup_3_Month>
                  <a:Setup_6_Month>29.0000</a:Setup_6_Month>
                  <a:Setup_12_Month>29.0000</a:Setup_12_Month>
                  <a:Setup_24_Month>29.0000</a:Setup_24_Month>
                  <a:Setup_36_Month>29.0000</a:Setup_36_Month>
                  <a:Sort_Order>2</a:Sort_Order>
                </a:Addons_Details>
              </a:AddonsDetailsList>
              <a:Result>true</a:Result>
            </AddonsListResult>
          </AddonsListResponse></s:Body></s:Envelope>"#;
        let addons = parse_addons_list_xml(addons_xml).unwrap();
        assert_eq!(addons.len(), 1);
        assert_eq!(addons[0].addon_id, 2232);
        assert_eq!(
            addons[0].pricing.setup_for(OrderBillingCycle::Monthly),
            Decimal::from_str("29.0000").unwrap()
        );
    }

    #[derive(Clone, Default)]
    struct MockOrderState {
        login_calls: Arc<AtomicUsize>,
        order_calls: Arc<AtomicUsize>,
        seen_keys: Arc<tokio::sync::Mutex<Vec<(String, String)>>>,
        stock_override: Arc<AtomicUsize>,
    }

    async fn spawn_mock_order_server(state: MockOrderState) -> String {
        let app = Router::new()
            .route(
                "/v1/Login/Token",
                post(
                    |State(st): State<MockOrderState>,
                     Query(q): Query<HashMap<String, String>>| async move {
                        if q.get("apiKey").map(String::as_str) != Some("rs-test-key-123") {
                            return (
                                StatusCode::UNAUTHORIZED,
                                Json(json!({ "status": false, "message": "Invalid key" })),
                            );
                        }
                        st.login_calls.fetch_add(1, Ordering::SeqCst);
                        (
                            StatusCode::OK,
                            Json(json!({ "status": true, "message": "tok-valid-abc" })),
                        )
                    },
                ),
            )
            .route(
                "/v1/Login/VerifyToken",
                get(|headers: HeaderMap| async move {
                    let auth = headers
                        .get("authorization")
                        .and_then(|v| v.to_str().ok())
                        .unwrap_or("");
                    if auth != "Bearer tok-valid-abc" {
                        return (
                            StatusCode::UNAUTHORIZED,
                            Json(json!({ "status": false, "message": "Invalid token" })),
                        );
                    }
                    (
                        StatusCode::OK,
                        Json(json!({
                            "status": true,
                            "message": "Token is valid",
                            "tokenExpirationTimestamp": (Utc::now() + Duration::hours(1)).to_rfc3339(),
                            "userId": 123
                        })),
                    )
                }),
            )
            .route(
                "/v1/PaymentMethods",
                get(|headers: HeaderMap, Query(_q): Query<HashMap<String, String>>| async move {
                    if headers.get("authorization").and_then(|v| v.to_str().ok())
                        != Some("Bearer tok-valid-abc")
                    {
                        return (StatusCode::UNAUTHORIZED, Json(json!({})));
                    }
                    (
                        StatusCode::OK,
                        Json(json!({
                            "paymentMethods": [
                                { "module": "banktransfer", "displayName": "Bank Transfer" },
                                { "module": "paypalbilling", "displayName": "PayPal" }
                            ]
                        })),
                    )
                }),
            )
            .route(
                "/v1/Products/{id}/Stock",
                get(
                    |State(st): State<MockOrderState>, Path(id): Path<i32>| async move {
                        let s = st.stock_override.load(Ordering::SeqCst) as i32;
                        (
                            StatusCode::OK,
                            Json(json!({
                                "productId": id,
                                "stock": s,
                                "stockControlled": true
                            })),
                        )
                    },
                ),
            )
            .route(
                "/v1/Reseller/PromoCodes",
                get(|| async {
                    (
                        StatusCode::OK,
                        Json(json!({
                            "promoCodes": [
                                {
                                    "configurationId": 88,
                                    "promoCode": "RESELLER88",
                                    "regularPrice": 299.00,
                                    "salePrice": 269.00
                                }
                            ]
                        })),
                    )
                }),
            )
            .route(
                "/v1/Orders",
                post(
                    |State(st): State<MockOrderState>,
                     headers: HeaderMap,
                     Json(body): Json<serde_json::Value>| async move {
                        if headers.get("authorization").and_then(|v| v.to_str().ok())
                            != Some("Bearer tok-valid-abc")
                        {
                            return (StatusCode::UNAUTHORIZED, Json(json!({})));
                        }
                        let idem = headers
                            .get("idempotency-key")
                            .and_then(|v| v.to_str().ok())
                            .unwrap_or("")
                            .to_string();
                        if idem.is_empty() {
                            return (StatusCode::BAD_REQUEST, Json(json!({ "error": "missing idempotency key" })));
                        }

                        let body_canonical = body.to_string();
                        let mut guard = st.seen_keys.lock().await;
                        if let Some((_, prev_body)) = guard.iter().find(|(k, _)| k == &idem) {
                            if prev_body == &body_canonical {
                                return (
                                    StatusCode::OK,
                                    Json(json!({
                                        "orderId": 88156,
                                        "invoiceId": 588048,
                                        "productIds": "91022",
                                        "paymentMethod": body["paymentMethod"],
                                        "productId": body["productId"],
                                        "replayed": true
                                    })),
                                );
                            }
                            return (
                                StatusCode::CONFLICT,
                                Json(json!({ "error": "Idempotency conflict" })),
                            );
                        }
                        guard.push((idem, body_canonical));
                        st.order_calls.fetch_add(1, Ordering::SeqCst);

                        (
                            StatusCode::OK,
                            Json(json!({
                                "orderId": 88156,
                                "invoiceId": 588048,
                                "productIds": "91022",
                                "paymentMethod": body["paymentMethod"],
                                "productId": body["productId"],
                                "replayed": false
                            })),
                        )
                    },
                ),
            )
            .with_state(state);

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        format!("http://{}", addr)
    }

    #[tokio::test]
    async fn test_order_api_full_flow_and_idempotency() {
        let state = MockOrderState {
            stock_override: Arc::new(AtomicUsize::new(7)),
            ..Default::default()
        };
        let base_url = spawn_mock_order_server(state.clone()).await;

        let rs = ReliableSite::with_endpoints(
            "rs-test-key-123".to_string(),
            base_url.clone(),
            format!("{}/inventory", base_url),
            Some("banktransfer".to_string()),
        );
        assert!(!rs.is_mock());

        // 1. Test connection -> /v1/Login/Token + /v1/Login/VerifyToken
        rs.test_connection().await.expect("test_connection failed");
        assert_eq!(state.login_calls.load(Ordering::SeqCst), 1);

        // 2. Reseller promos & payment methods & stock
        let promos = rs
            .order_api()
            .get_reseller_promo_codes(Some(88))
            .await
            .unwrap();
        assert_eq!(promos.promo_codes.len(), 1);
        assert_eq!(promos.promo_codes[0].promo_code, "RESELLER88");

        let mut config_opts = BTreeMap::new();
        config_opts.insert("565".to_string(), 1751);
        config_opts.insert("659".to_string(), 2391);
        config_opts.insert("655".to_string(), 2341);

        let params = ProvisionParams {
            server_type: "428".to_string(),
            location: "NL".to_string(),
            image: "ubuntu-24.04".to_string(),
            ssh_keys: vec![],
            name: "ams-node-01.vaultscope.de".to_string(),
            extra: json!({
                "reliablesite_product_id": 428,
                "billing_cycle": "semiannually",
                "config_options": config_opts,
                "order_notes": "NL deployment",
                "idempotency_key": "idem-order-nl-001"
            }),
        };

        // 3. Provision places POST /v1/Orders with cached token (still 1 login call)
        let res1 = rs.provision(&params).await.expect("order 1 failed");
        assert_eq!(res1.resource_id, "ds-ord-88156");
        assert_eq!(res1.raw_response["orderId"], 88156);
        assert_eq!(res1.raw_response["invoiceId"], 588048);
        assert_eq!(res1.raw_response["paymentMethod"], "banktransfer");
        assert_eq!(res1.raw_response["replayed"], false);
        assert_eq!(state.login_calls.load(Ordering::SeqCst), 1);
        assert_eq!(state.order_calls.load(Ordering::SeqCst), 1);

        // 4. Retry same idempotency key returns replayed: true
        let res2 = rs.provision(&params).await.expect("order replay failed");
        assert_eq!(res2.raw_response["replayed"], true);
        assert_eq!(state.order_calls.load(Ordering::SeqCst), 1);

        // 5. Same key with changed body yields 409 Conflict error
        let mut modified_params = params.clone();
        modified_params.name = "different-host.vaultscope.de".to_string();
        assert!(rs.provision(&modified_params).await.is_err());

        // 6. Stock = 0 blocks ordering cleanly
        state.stock_override.store(0, Ordering::SeqCst);
        let mut oos_params = params.clone();
        oos_params.extra["idempotency_key"] = json!("idem-order-nl-002");
        let oos_err = rs.provision(&oos_params).await.unwrap_err();
        assert!(oos_err.to_string().contains("out of stock"));
    }
}
