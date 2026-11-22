use std::collections::HashMap;
use std::str::FromStr;

use reqwest::Client;
use roxmltree::{Document, Node};
use rust_decimal::Decimal;

use crate::{ConnectorError, ConnectorResult};

use super::types::{
    InventoryAddon, InventoryAddonGroup, InventoryPricing, InventoryServer,
};

pub const DEFAULT_INVENTORY_URL: &str = "https://api.reliablesite.net/Inventory.svc/Inventory";
const SOAP_ACTION_PREFIX: &str = "http://tempuri.org/IInventoryService";

/// Client for the ReliableSite WCF SOAP Inventory API.
#[derive(Clone)]
pub struct InventorySoapClient {
    client: Client,
    endpoint_url: String,
}

impl InventorySoapClient {
    pub fn new(client: Client, endpoint_url: String) -> Self {
        let cleaned = endpoint_url.trim();
        let url = if cleaned.is_empty() {
            DEFAULT_INVENTORY_URL.to_string()
        } else {
            cleaned.trim_end_matches('/').to_string()
        };
        Self {
            client,
            endpoint_url: url,
        }
    }

    pub fn endpoint_url(&self) -> &str {
        &self.endpoint_url
    }

    async fn call_soap(&self, operation: &str, body_xml: &str) -> ConnectorResult<String> {
        let envelope = format!(
            r#"<?xml version="1.0" encoding="utf-8"?><s:Envelope xmlns:s="http://schemas.xmlsoap.org/soap/envelope/"><s:Body>{}</s:Body></s:Envelope>"#,
            body_xml
        );
        let soap_action = format!("\"{}/{}\"", SOAP_ACTION_PREFIX, operation);

        let resp = self
            .client
            .post(&self.endpoint_url)
            .header("Content-Type", "text/xml; charset=utf-8")
            .header("SOAPAction", soap_action)
            .header("User-Agent", "VaultScope/1.0")
            .timeout(std::time::Duration::from_secs(20))
            .body(envelope)
            .send()
            .await
            .map_err(|e| {
                tracing::warn!(
                    status = ?e.status(),
                    is_timeout = e.is_timeout(),
                    "ReliableSite Inventory SOAP transport error"
                );
                ConnectorError::Api("ReliableSite Inventory SOAP request failed".into())
            })?;

        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        if !status.is_success() {
            return Err(ConnectorError::Api(format!(
                "ReliableSite Inventory SOAP {} failed ({}): {}",
                operation, status, text
            )));
        }

        Ok(text)
    }

    pub async fn servers_list(&self) -> ConnectorResult<Vec<InventoryServer>> {
        let xml = self
            .call_soap(
                "ServersList",
                r#"<ServersList xmlns="http://tempuri.org/" />"#,
            )
            .await?;
        parse_servers_list_xml(&xml)
    }

    pub async fn addon_groups_list(&self, product_id: i32) -> ConnectorResult<Vec<InventoryAddonGroup>> {
        let body = format!(
            r#"<AddonGroupsList xmlns="http://tempuri.org/"><pStrProductId>{}</pStrProductId></AddonGroupsList>"#,
            product_id
        );
        let xml = self.call_soap("AddonGroupsList", &body).await?;
        parse_addon_groups_list_xml(&xml)
    }

    pub async fn addons_list(&self, group_id: i32) -> ConnectorResult<Vec<InventoryAddon>> {
        let body = format!(
            r#"<AddonsList xmlns="http://tempuri.org/"><pStrGroupId>{}</pStrGroupId></AddonsList>"#,
            group_id
        );
        let xml = self.call_soap("AddonsList", &body).await?;
        parse_addons_list_xml(&xml)
    }
}

fn child_text_map<'a>(node: Node<'a, 'a>) -> HashMap<&'a str, &'a str> {
    let mut map = HashMap::new();
    for child in node.children().filter(|n| n.is_element()) {
        let text = child.text().unwrap_or("").trim();
        map.insert(child.tag_name().name(), text);
    }
    map
}

fn verify_soap_result(doc: &Document<'_>, operation: &str) -> ConnectorResult<()> {
    let mut result_flag: Option<bool> = None;
    let mut message: Option<&str> = None;
    let mut fault_string: Option<&str> = None;

    for node in doc.descendants().filter(|n| n.is_element()) {
        match node.tag_name().name() {
            "Fault" if fault_string.is_none() => {
                fault_string = Some("SOAP Fault");
            }
            "faultstring" => {
                fault_string = node.text().map(str::trim).or(Some("SOAP Fault"));
            }
            "Result" if result_flag.is_none() => {
                if let Some(txt) = node.text() {
                    result_flag = match txt.trim().to_ascii_lowercase().as_str() {
                        "true" => Some(true),
                        "false" => Some(false),
                        _ => None,
                    };
                }
            }
            "Message" if message.is_none() => {
                message = node.text().map(str::trim);
            }
            _ => {}
        }
    }

    if let Some(fault) = fault_string {
        return Err(ConnectorError::Api(format!(
            "ReliableSite {} SOAP Fault: {}",
            operation, fault
        )));
    }

    if result_flag == Some(false) {
        return Err(ConnectorError::Api(format!(
            "ReliableSite {} returned Result=false: {}",
            operation,
            message.unwrap_or("unknown error")
        )));
    }

    Ok(())
}

fn parse_decimal(map: &HashMap<&str, &str>, key: &str) -> Decimal {
    map.get(key)
        .and_then(|v| {
            let s = v.trim();
            if s.is_empty() {
                None
            } else {
                Decimal::from_str(s).ok()
            }
        })
        .unwrap_or(Decimal::ZERO)
}

fn parse_i32(map: &HashMap<&str, &str>, key: &str, default: i32) -> i32 {
    map.get(key)
        .and_then(|v| v.trim().parse::<i32>().ok())
        .unwrap_or(default)
}

fn parse_pricing(map: &HashMap<&str, &str>) -> InventoryPricing {
    InventoryPricing {
        setup_1_hour: parse_decimal(map, "Setup_1_Hour"),
        setup_1_month: parse_decimal(map, "Setup_1_Month"),
        setup_3_month: parse_decimal(map, "Setup_3_Month"),
        setup_6_month: parse_decimal(map, "Setup_6_Month"),
        setup_12_month: parse_decimal(map, "Setup_12_Month"),
        setup_24_month: parse_decimal(map, "Setup_24_Month"),
        setup_36_month: parse_decimal(map, "Setup_36_Month"),
        recurring_1_hour: parse_decimal(map, "Recurring_1_Hour"),
        recurring_1_month: parse_decimal(map, "Recurring_1_Month"),
        recurring_3_month: parse_decimal(map, "Recurring_3_Month"),
        recurring_6_month: parse_decimal(map, "Recurring_6_Month"),
        recurring_12_month: parse_decimal(map, "Recurring_12_Month"),
        recurring_24_month: parse_decimal(map, "Recurring_24_Month"),
        recurring_36_month: parse_decimal(map, "Recurring_36_Month"),
    }
}

pub fn parse_servers_list_xml(xml: &str) -> ConnectorResult<Vec<InventoryServer>> {
    let doc = Document::parse(xml)
        .map_err(|e| ConnectorError::Api(format!("Failed to parse ServersList XML: {}", e)))?;
    verify_soap_result(&doc, "ServersList")?;

    let mut out = Vec::new();
    for node in doc.descendants().filter(|n| n.is_element()) {
        if node.tag_name().name() != "Server_Details" {
            continue;
        }
        let map = child_text_map(node);
        let product_id = parse_i32(&map, "Product_Id", 0);
        if product_id <= 0 {
            continue;
        }
        out.push(InventoryServer {
            product_id,
            description: map.get("Description").copied().unwrap_or("").to_string(),
            data_center: map.get("Data_Center").copied().unwrap_or("").to_string(),
            detail: map.get("Detail").copied().unwrap_or("").to_string(),
            stock: parse_i32(&map, "Stock", 0),
            pricing: parse_pricing(&map),
        });
    }

    Ok(out)
}

pub fn parse_addon_groups_list_xml(xml: &str) -> ConnectorResult<Vec<InventoryAddonGroup>> {
    let doc = Document::parse(xml)
        .map_err(|e| ConnectorError::Api(format!("Failed to parse AddonGroupsList XML: {}", e)))?;
    verify_soap_result(&doc, "AddonGroupsList")?;

    let mut out = Vec::new();
    for node in doc.descendants().filter(|n| n.is_element()) {
        let name = node.tag_name().name();
        if name != "AddonGroups_Details" && name != "AddonGroup_Details" {
            continue;
        }
        let map = child_text_map(node);
        let group_id = parse_i32(&map, "Group_Id", 0);
        let description = map.get("Description").copied().unwrap_or("").to_string();
        // ReliableSite returns sentinel Group_Id=0 with empty description when none/trailing
        if group_id <= 0 || description.trim().is_empty() {
            continue;
        }
        out.push(InventoryAddonGroup {
            group_id,
            description,
            sort_order: parse_i32(&map, "Sort_Order", 0),
        });
    }

    out.sort_by_key(|g| (g.sort_order, g.group_id));
    Ok(out)
}

pub fn parse_addons_list_xml(xml: &str) -> ConnectorResult<Vec<InventoryAddon>> {
    let doc = Document::parse(xml)
        .map_err(|e| ConnectorError::Api(format!("Failed to parse AddonsList XML: {}", e)))?;
    verify_soap_result(&doc, "AddonsList")?;

    let mut out = Vec::new();
    for node in doc.descendants().filter(|n| n.is_element()) {
        let name = node.tag_name().name();
        if name != "Addons_Details" && name != "Addon_Details" {
            continue;
        }
        let map = child_text_map(node);
        let addon_id = parse_i32(&map, "Addon_Id", 0);
        let group_id = parse_i32(&map, "Group_Id", 0);
        if addon_id <= 0 || group_id <= 0 {
            continue;
        }
        out.push(InventoryAddon {
            addon_id,
            group_id,
            addon_description: map
                .get("AddonDescription")
                .or_else(|| map.get("Addon_Description"))
                .copied()
                .unwrap_or("")
                .to_string(),
            sort_order: parse_i32(&map, "Sort_Order", 0),
            pricing: parse_pricing(&map),
        });
    }

    out.sort_by_key(|a| (a.sort_order, a.addon_id));
    Ok(out)
}
