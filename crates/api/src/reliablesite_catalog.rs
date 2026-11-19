use std::collections::{BTreeMap, HashMap};

use rust_decimal::prelude::ToPrimitive;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use utoipa::ToSchema;
use uuid::Uuid;
use vaultscope_connectors::reliablesite::{
    InventoryAddon, InventoryServer, OrderBillingCycle, ReliableSite,
};
use vaultscope_db::models::{
    Product, ReliableSiteAddon, ReliableSiteAddonGroup, ReliableSiteServer, Service,
};

use crate::error::ApiError;

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct AvailableCyclePrice {
    pub billing_cycle: String,
    pub label: String,
    pub period_days: i64,
    pub recurring: Decimal,
    pub setup_fee: Decimal,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ReliableSiteConfigGroupView {
    pub group_id: i32,
    pub description: String,
    pub sort_order: i32,
    pub required: bool,
    pub options: Vec<ReliableSiteAddon>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ReliableSiteProductOptionsResponse {
    pub vaultscope_product_id: Uuid,
    pub server: ReliableSiteServer,
    pub available_cycles: Vec<AvailableCyclePrice>,
    pub default_config_options: BTreeMap<String, u32>,
    pub groups: Vec<ReliableSiteConfigGroupView>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct DedicatedServerSummary {
    pub data_center: String,
    pub description: String,
    pub detail: String,
    pub stock: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct DedicatedAddonOption {
    pub addon_id: i32,
    pub group_id: i32,
    pub addon_description: String,
    pub sort_order: i32,
    pub setup_1_month: Decimal,
    pub setup_3_month: Decimal,
    pub setup_6_month: Decimal,
    pub setup_12_month: Decimal,
    pub setup_24_month: Decimal,
    pub setup_36_month: Decimal,
    pub recurring_1_month: Decimal,
    pub recurring_3_month: Decimal,
    pub recurring_6_month: Decimal,
    pub recurring_12_month: Decimal,
    pub recurring_24_month: Decimal,
    pub recurring_36_month: Decimal,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct DedicatedAddonGroupView {
    pub group_id: i32,
    pub description: String,
    pub sort_order: i32,
    pub required: bool,
    pub options: Vec<DedicatedAddonOption>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct DedicatedProductOptionsResponse {
    pub product_id: Uuid,
    pub server: DedicatedServerSummary,
    pub available_cycles: Vec<AvailableCyclePrice>,
    pub default_config_options: BTreeMap<String, u32>,
    pub groups: Vec<DedicatedAddonGroupView>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct QuoteLineItem {
    pub group_id: Option<i32>,
    pub group_name: String,
    pub addon_id: Option<i32>,
    pub label: String,
    pub recurring: Decimal,
    pub setup_fee: Decimal,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct DedicatedQuote {
    pub product_id: Uuid,
    pub billing_cycle: String,
    pub period_days: i64,
    pub recurring_net: Decimal,
    pub setup_fee_net: Decimal,
    pub subtotal_net: Decimal,
    pub vat_amount: Decimal,
    pub total_gross: Decimal,
    pub stock: i32,
    pub validated_config_options: BTreeMap<String, u32>,
    pub line_items: Vec<QuoteLineItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ReliableSiteQuote {
    pub vaultscope_product_id: Uuid,
    pub reliablesite_product_id: i32,
    pub billing_cycle: String,
    pub period_days: i64,
    pub recurring_net: Decimal,
    pub setup_fee_net: Decimal,
    pub subtotal_net: Decimal,
    pub vat_amount: Decimal,
    pub total_gross: Decimal,
    pub stock: i32,
    pub validated_config_options: BTreeMap<String, u32>,
    pub line_items: Vec<QuoteLineItem>,
}

impl ReliableSiteQuote {
    pub fn into_white_label(self) -> DedicatedQuote {
        DedicatedQuote {
            product_id: self.vaultscope_product_id,
            billing_cycle: self.billing_cycle,
            period_days: self.period_days,
            recurring_net: self.recurring_net,
            setup_fee_net: self.setup_fee_net,
            subtotal_net: self.subtotal_net,
            vat_amount: self.vat_amount,
            total_gross: self.total_gross,
            stock: self.stock,
            validated_config_options: self.validated_config_options,
            line_items: self.line_items,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct SyncSummary {
    pub data_center: String,
    pub servers_synced: usize,
    pub groups_synced: usize,
    pub product_group_links_synced: usize,
    pub addons_synced: usize,
    pub servers: Vec<ReliableSiteServer>,
}

pub fn cycle_label(cycle: OrderBillingCycle) -> &'static str {
    match cycle {
        OrderBillingCycle::Monthly => "Monthly (1 Month)",
        OrderBillingCycle::Quarterly => "Quarterly (3 Months)",
        OrderBillingCycle::Semiannually => "Semiannually (6 Months)",
        OrderBillingCycle::Annually => "Annually (12 Months)",
        OrderBillingCycle::Biennially => "Biennially (24 Months)",
        OrderBillingCycle::Triennially => "Triennially (36 Months)",
    }
}

pub fn server_recurring_for_cycle(
    srv: &ReliableSiteServer,
    cycle: OrderBillingCycle,
) -> Option<Decimal> {
    let v = match cycle {
        OrderBillingCycle::Monthly => srv.recurring_1_month,
        OrderBillingCycle::Quarterly => srv.recurring_3_month,
        OrderBillingCycle::Semiannually => srv.recurring_6_month,
        OrderBillingCycle::Annually => srv.recurring_12_month,
        OrderBillingCycle::Biennially => srv.recurring_24_month,
        OrderBillingCycle::Triennially => srv.recurring_36_month,
    };
    if v < Decimal::ZERO {
        None
    } else {
        Some(v)
    }
}

pub fn server_setup_for_cycle(srv: &ReliableSiteServer, cycle: OrderBillingCycle) -> Decimal {
    let v = match cycle {
        OrderBillingCycle::Monthly => srv.setup_1_month,
        OrderBillingCycle::Quarterly => srv.setup_3_month,
        OrderBillingCycle::Semiannually => srv.setup_6_month,
        OrderBillingCycle::Annually => srv.setup_12_month,
        OrderBillingCycle::Biennially => srv.setup_24_month,
        OrderBillingCycle::Triennially => srv.setup_36_month,
    };
    if v < Decimal::ZERO {
        Decimal::ZERO
    } else {
        v
    }
}

pub fn addon_recurring_for_cycle(
    addon: &ReliableSiteAddon,
    cycle: OrderBillingCycle,
) -> Option<Decimal> {
    let v = match cycle {
        OrderBillingCycle::Monthly => addon.recurring_1_month,
        OrderBillingCycle::Quarterly => addon.recurring_3_month,
        OrderBillingCycle::Semiannually => addon.recurring_6_month,
        OrderBillingCycle::Annually => addon.recurring_12_month,
        OrderBillingCycle::Biennially => addon.recurring_24_month,
        OrderBillingCycle::Triennially => addon.recurring_36_month,
    };
    if v < Decimal::ZERO {
        None
    } else {
        Some(v)
    }
}

pub fn addon_setup_for_cycle(addon: &ReliableSiteAddon, cycle: OrderBillingCycle) -> Decimal {
    let v = match cycle {
        OrderBillingCycle::Monthly => addon.setup_1_month,
        OrderBillingCycle::Quarterly => addon.setup_3_month,
        OrderBillingCycle::Semiannually => addon.setup_6_month,
        OrderBillingCycle::Annually => addon.setup_12_month,
        OrderBillingCycle::Biennially => addon.setup_24_month,
        OrderBillingCycle::Triennially => addon.setup_36_month,
    };
    if v < Decimal::ZERO {
        Decimal::ZERO
    } else {
        v
    }
}

pub async fn load_server_for_product(
    db: &PgPool,
    product: &Product,
) -> Result<ReliableSiteServer, ApiError> {
    if let Some(srv) = sqlx::query_as::<_, ReliableSiteServer>(
        "SELECT * FROM reliablesite_servers WHERE vaultscope_product_id = $1",
    )
    .bind(product.id)
    .fetch_optional(db)
    .await?
    {
        return Ok(srv);
    }

    if let Ok(rs_id) = product.target.trim().parse::<i32>() {
        if let Some(srv) = sqlx::query_as::<_, ReliableSiteServer>(
            "SELECT * FROM reliablesite_servers WHERE product_id = $1",
        )
        .bind(rs_id)
        .fetch_optional(db)
        .await?
        {
            return Ok(srv);
        }
    }

    Err(ApiError::NotFound(format!(
        "Dedicated server configuration not found for product '{}'",
        product.name
    )))
}

pub async fn load_server_groups_with_addons(
    db: &PgPool,
    rs_product_id: i32,
) -> Result<Vec<ReliableSiteConfigGroupView>, ApiError> {
    let groups: Vec<ReliableSiteAddonGroup> = sqlx::query_as(
        r#"
        SELECT g.group_id, g.description, pag.sort_order, g.required
        FROM reliablesite_product_addon_groups pag
        JOIN reliablesite_addon_groups g ON pag.group_id = g.group_id
        WHERE pag.product_id = $1
        ORDER BY pag.sort_order ASC, g.group_id ASC
        "#,
    )
    .bind(rs_product_id)
    .fetch_all(db)
    .await?;

    if groups.is_empty() {
        return Ok(Vec::new());
    }

    let group_ids: Vec<i32> = groups.iter().map(|g| g.group_id).collect();
    let addons: Vec<ReliableSiteAddon> = sqlx::query_as(
        r#"
        SELECT * FROM reliablesite_addons
        WHERE group_id = ANY($1)
        ORDER BY sort_order ASC, addon_id ASC
        "#,
    )
    .bind(&group_ids)
    .fetch_all(db)
    .await?;

    let mut by_group: HashMap<i32, Vec<ReliableSiteAddon>> = HashMap::new();
    for addon in addons {
        by_group.entry(addon.group_id).or_default().push(addon);
    }

    let views = groups
        .into_iter()
        .map(|g| {
            let options = by_group.remove(&g.group_id).unwrap_or_default();
            ReliableSiteConfigGroupView {
                group_id: g.group_id,
                description: g.description,
                sort_order: g.sort_order,
                required: g.required,
                options,
            }
        })
        .collect();

    Ok(views)
}

fn retail_base_for_cycle(
    product: &Product,
    srv: &ReliableSiteServer,
    cycle: OrderBillingCycle,
    upstream_recurring: Decimal,
) -> (Decimal, Decimal) {
    let upstream_setup = server_setup_for_cycle(srv, cycle);
    let base_recurring = if cycle == OrderBillingCycle::Monthly && product.price > Decimal::ZERO {
        product.price
    } else if srv.recurring_1_month > Decimal::ZERO
        && product.price > Decimal::ZERO
        && product.price != srv.recurring_1_month
    {
        // Preserve admin retail margin proportionally when scaling across multi-month terms
        ((upstream_recurring * product.price) / srv.recurring_1_month).round_dp(2)
    } else {
        upstream_recurring.round_dp(2)
    };

    let base_setup = if cycle == OrderBillingCycle::Monthly && product.setup_fee > upstream_setup {
        product.setup_fee
    } else {
        upstream_setup.round_dp(2)
    };

    (base_recurring, base_setup)
}

pub async fn get_product_options(
    db: &PgPool,
    product: &Product,
    live_stock_override: Option<i32>,
) -> Result<ReliableSiteProductOptionsResponse, ApiError> {
    let mut server = load_server_for_product(db, product).await?;
    if let Some(s) = live_stock_override {
        server.stock = s;
    }

    let groups = load_server_groups_with_addons(db, server.product_id).await?;

    let mut available_cycles = Vec::new();
    for cycle in OrderBillingCycle::ALL {
        if let Some(upstream_rec) = server_recurring_for_cycle(&server, cycle) {
            let (recurring, setup_fee) = retail_base_for_cycle(product, &server, cycle, upstream_rec);
            available_cycles.push(AvailableCyclePrice {
                billing_cycle: cycle.as_str().to_string(),
                label: cycle_label(cycle).to_string(),
                period_days: cycle.period_days(),
                recurring,
                setup_fee,
            });
        }
    }

    let mut default_config_options = BTreeMap::new();
    for g in &groups {
        // Prefer the lowest-cost 1m option (typically $0.00 included baseline), breaking ties by sort_order
        if let Some(best) = g
            .options
            .iter()
            .filter(|o| o.recurring_1_month >= Decimal::ZERO)
            .min_by_key(|o| {
                (
                    o.recurring_1_month,
                    o.setup_1_month,
                    o.sort_order,
                    o.addon_id,
                )
            })
        {
            if best.addon_id > 0 {
                default_config_options.insert(g.group_id.to_string(), best.addon_id as u32);
            }
        }
    }

    Ok(ReliableSiteProductOptionsResponse {
        vaultscope_product_id: product.id,
        server,
        available_cycles,
        default_config_options,
        groups,
    })
}

pub async fn get_dedicated_options(
    db: &PgPool,
    product: &Product,
    live_stock_override: Option<i32>,
) -> Result<DedicatedProductOptionsResponse, ApiError> {
    let raw = get_product_options(db, product, live_stock_override).await?;
    Ok(DedicatedProductOptionsResponse {
        product_id: raw.vaultscope_product_id,
        server: DedicatedServerSummary {
            data_center: raw.server.data_center,
            description: raw.server.description,
            detail: raw.server.detail,
            stock: raw.server.stock,
        },
        available_cycles: raw.available_cycles,
        default_config_options: raw.default_config_options,
        groups: raw
            .groups
            .into_iter()
            .map(|g| DedicatedAddonGroupView {
                group_id: g.group_id,
                description: g.description,
                sort_order: g.sort_order,
                required: g.required,
                options: g
                    .options
                    .into_iter()
                    .map(|o| DedicatedAddonOption {
                        addon_id: o.addon_id,
                        group_id: o.group_id,
                        addon_description: o.addon_description,
                        sort_order: o.sort_order,
                        setup_1_month: o.setup_1_month,
                        setup_3_month: o.setup_3_month,
                        setup_6_month: o.setup_6_month,
                        setup_12_month: o.setup_12_month,
                        setup_24_month: o.setup_24_month,
                        setup_36_month: o.setup_36_month,
                        recurring_1_month: o.recurring_1_month,
                        recurring_3_month: o.recurring_3_month,
                        recurring_6_month: o.recurring_6_month,
                        recurring_12_month: o.recurring_12_month,
                        recurring_24_month: o.recurring_24_month,
                        recurring_36_month: o.recurring_36_month,
                    })
                    .collect(),
            })
            .collect(),
    })
}

pub fn sanitize_product_for_storefront(mut product: Product) -> Product {
    product.cost = Decimal::ZERO;
    if product.provider == "reliablesite" {
        product.provider = "vaultscope_dedicated".to_string();
        product.target = "dedicated".to_string();
        if let Some(obj) = product.specs.as_object_mut() {
            obj.remove("reliablesite_product_id");
            obj.insert(
                "configurable_options".to_string(),
                serde_json::Value::Bool(true),
            );
        }
    }
    product
}

pub fn sanitize_service_for_storefront(mut service: Service) -> Service {
    if service.provider_resource_id.starts_with("rs-order-") {
        service.provider_resource_id =
            service.provider_resource_id.replacen("rs-order-", "ds-ord-", 1);
    } else if service.provider_resource_id.starts_with("rs-ds-") {
        service.provider_resource_id = service.provider_resource_id.replacen("rs-ds-", "ds-", 1);
    }

    if let Some(obj) = service.config.as_object_mut() {
        obj.remove("provider");
        obj.remove("reliablesite_product_id");
        obj.remove("password");
        obj.remove("root_password");
        if let Some(raw_obj) = obj.get_mut("raw_response").and_then(|v| v.as_object_mut()) {
            raw_obj.remove("provider");
            raw_obj.remove("paymentMethod");
            raw_obj.remove("productId");
            raw_obj.remove("replayed");
            raw_obj.remove("idempotencyKey");
            raw_obj.remove("password");
            raw_obj.remove("root_password");
            if let Some(sftp_obj) = raw_obj
                .get_mut("sftpgo_user")
                .and_then(|v| v.as_object_mut())
            {
                sftp_obj.remove("password");
            }
            if let Some(rid) = raw_obj
                .get("resource_id")
                .and_then(|v| v.as_str())
                .map(str::to_string)
            {
                if rid.starts_with("rs-order-") {
                    raw_obj.insert(
                        "resource_id".to_string(),
                        serde_json::Value::String(rid.replacen("rs-order-", "ds-ord-", 1)),
                    );
                } else if rid.starts_with("rs-ds-") {
                    raw_obj.insert(
                        "resource_id".to_string(),
                        serde_json::Value::String(rid.replacen("rs-ds-", "ds-", 1)),
                    );
                }
            }
        }
    }
    service
}

pub async fn compute_quote(
    db: &PgPool,
    product: &Product,
    billing_cycle_raw: Option<&str>,
    supplied_options: Option<&BTreeMap<String, u32>>,
) -> Result<ReliableSiteQuote, ApiError> {
    let cycle = match billing_cycle_raw.map(str::trim).filter(|s| !s.is_empty()) {
        Some(raw) => OrderBillingCycle::from_str_loose(raw).ok_or_else(|| {
            ApiError::BadRequest(format!(
                "Unsupported billing_cycle '{}'. Supported: monthly, quarterly, semiannually, annually, biennially, triennially",
                raw
            ))
        })?,
        None => OrderBillingCycle::Monthly,
    };

    let server = load_server_for_product(db, product).await?;
    let upstream_rec = server_recurring_for_cycle(&server, cycle).ok_or_else(|| {
        ApiError::BadRequest(format!(
            "Billing cycle '{}' is not available for '{}'",
            cycle.as_str(),
            server.description
        ))
    })?;

    let (base_recurring, base_setup) = retail_base_for_cycle(product, &server, cycle, upstream_rec);
    let groups = load_server_groups_with_addons(db, server.product_id).await?;

    let group_map: HashMap<i32, &ReliableSiteConfigGroupView> =
        groups.iter().map(|g| (g.group_id, g)).collect();

    let resolved_map: BTreeMap<String, u32> = match supplied_options {
        Some(map) => {
            for (gid_str, addon_id) in map {
                let gid: i32 = gid_str.trim().parse().map_err(|_| {
                    ApiError::BadRequest(format!("Invalid configuration group '{}'", gid_str))
                })?;
                let grp = group_map.get(&gid).ok_or_else(|| {
                    ApiError::BadRequest(format!(
                        "Configuration group {} is not valid for '{}'",
                        gid, product.name
                    ))
                })?;
                if *addon_id == 0
                    || !grp
                        .options
                        .iter()
                        .any(|opt| opt.addon_id as u32 == *addon_id)
                {
                    return Err(ApiError::BadRequest(format!(
                        "Addon_Id {} does not belong to Group_Id {} ({})",
                        addon_id, gid, grp.description
                    )));
                }
            }
            // Verify all required groups are present when explicit config_options were provided
            for grp in &groups {
                if grp.required && !map.contains_key(&grp.group_id.to_string()) {
                    return Err(ApiError::BadRequest(format!(
                        "Missing required configuration option for group {} ({})",
                        grp.group_id, grp.description
                    )));
                }
            }
            map.clone()
        }
        None => {
            // Auto-fill baseline default option for each group
            let opts = get_product_options(db, product, None).await?;
            opts.default_config_options
        }
    };

    let mut recurring_net = base_recurring;
    let mut setup_fee_net = base_setup;
    let mut line_items = vec![QuoteLineItem {
        group_id: None,
        group_name: "Base Server".to_string(),
        addon_id: None,
        label: server.description.clone(),
        recurring: base_recurring,
        setup_fee: base_setup,
    }];

    for grp in &groups {
        let key = grp.group_id.to_string();
        let Some(&selected_addon_id) = resolved_map.get(&key) else {
            continue;
        };
        let addon = grp
            .options
            .iter()
            .find(|o| o.addon_id as u32 == selected_addon_id)
            .ok_or_else(|| {
                ApiError::BadRequest(format!(
                    "Invalid Addon_Id {} for group {}",
                    selected_addon_id, grp.group_id
                ))
            })?;

        let add_rec = addon_recurring_for_cycle(addon, cycle).ok_or_else(|| {
            ApiError::BadRequest(format!(
                "Addon '{}' is not available on billing cycle '{}'",
                addon.addon_description,
                cycle.as_str()
            ))
        })?;
        let add_setup = addon_setup_for_cycle(addon, cycle);

        recurring_net += add_rec.round_dp(2);
        setup_fee_net += add_setup.round_dp(2);

        line_items.push(QuoteLineItem {
            group_id: Some(grp.group_id),
            group_name: grp.description.clone(),
            addon_id: Some(addon.addon_id),
            label: addon.addon_description.clone(),
            recurring: add_rec.round_dp(2),
            setup_fee: add_setup.round_dp(2),
        });
    }

    let subtotal_net = (recurring_net + setup_fee_net).round_dp(2);
    let subtotal_cents = (subtotal_net * Decimal::from(100)).to_i64().unwrap_or(0);
    let vat_cents = subtotal_cents * 19 / 100;
    let vat_amount = Decimal::new(vat_cents, 2);
    let total_gross = (subtotal_net + vat_amount).round_dp(2);

    Ok(ReliableSiteQuote {
        vaultscope_product_id: product.id,
        reliablesite_product_id: server.product_id,
        billing_cycle: cycle.as_str().to_string(),
        period_days: cycle.period_days(),
        recurring_net: recurring_net.round_dp(2),
        setup_fee_net: setup_fee_net.round_dp(2),
        subtotal_net,
        vat_amount,
        total_gross,
        stock: server.stock,
        validated_config_options: resolved_map,
        line_items,
    })
}

fn deterministic_product_uuid(rs_product_id: i32) -> Uuid {
    let hex_str = format!("10000000-0000-4000-8000-{rs_product_id:012}");
    Uuid::parse_str(&hex_str).unwrap_or_else(|_| Uuid::now_v7())
}

fn build_specs_from_server(srv: &InventoryServer) -> serde_json::Value {
    let cleaned = srv.detail.replace("<br />", "\n").replace("<br/>", "\n");
    let lines: Vec<&str> = cleaned.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
    let cpu = lines.first().copied().unwrap_or(&srv.description);
    let ram = lines.get(1).copied().unwrap_or("");
    let disk = lines.get(2).copied().unwrap_or("");
    let gpu = lines
        .iter()
        .skip(3)
        .find(|l| {
            let low = l.to_ascii_lowercase();
            low.contains("gpu") || low.contains("rtx") || low.contains("nvidia")
        })
        .copied();

    let mut obj = serde_json::json!({
        "reliablesite_product_id": srv.product_id,
        "datacenter": srv.data_center,
        "location": if srv.data_center.eq_ignore_ascii_case("NL") { "Amsterdam, NL" } else { &srv.data_center },
        "server_type": srv.product_id.to_string(),
        "cpu": cpu,
        "ram": ram,
        "disk": disk,
        "uplink": "1 Gbps Unmetered",
        "traffic": "Unmetered",
        "ipmi": "Dedicated KVM Access with Virtual Media",
        "preorder": srv.description.contains("Pre-Order"),
        "detail": srv.detail,
    });
    if let Some(g) = gpu {
        obj["gpu"] = serde_json::Value::String(g.to_string());
    }
    obj
}

pub async fn sync_datacenter_to_db(
    db: &PgPool,
    rs: &ReliableSite,
    data_center: &str,
) -> Result<SyncSummary, ApiError> {
    let sync_data = rs
        .fetch_datacenter_catalog(data_center)
        .await
        .map_err(|e| ApiError::Internal(format!("ReliableSite Inventory sync failed: {e}")))?;

    let mut tx = db.begin().await?;
    let mut group_ids_seen = std::collections::HashSet::new();
    let mut links_count = 0usize;
    let mut addons_count = 0usize;

    for srv in &sync_data.servers {
        let existing_pid: Option<Uuid> = sqlx::query_scalar(
            "SELECT vaultscope_product_id FROM reliablesite_servers WHERE product_id = $1",
        )
        .bind(srv.product_id)
        .fetch_optional(&mut *tx)
        .await?
        .flatten();

        let fallback_pid = deterministic_product_uuid(srv.product_id);
        let prod_id = if let Some(pid) = existing_pid {
            pid
        } else {
            let found_by_target: Option<Uuid> = sqlx::query_scalar(
                "SELECT id FROM products WHERE provider = 'reliablesite' AND target = $1 LIMIT 1",
            )
            .bind(srv.product_id.to_string())
            .fetch_optional(&mut *tx)
            .await?;
            found_by_target.unwrap_or(fallback_pid)
        };

        let specs = build_specs_from_server(srv);
        let cost_1m = srv.pricing.recurring_1_month.max(Decimal::ZERO).round_dp(2);
        let setup_1m = srv.pricing.setup_1_month.max(Decimal::ZERO).round_dp(2);

        sqlx::query(
            r#"
            INSERT INTO products (id, name, category, provider, target, specs, cost, price, setup_fee, stock, user_limit, billing_cycle, hidden)
            VALUES ($1, $2, 'dedicated', 'reliablesite', $3, $4, $5, $6, $7, $8, 0, 'monthly', false)
            ON CONFLICT (id) DO UPDATE SET
                name = EXCLUDED.name,
                specs = EXCLUDED.specs,
                cost = EXCLUDED.cost,
                setup_fee = EXCLUDED.setup_fee,
                stock = EXCLUDED.stock,
                updated_at = now()
            "#,
        )
        .bind(prod_id)
        .bind(&srv.description)
        .bind(srv.product_id.to_string())
        .bind(&specs)
        .bind(cost_1m)
        .bind(cost_1m)
        .bind(setup_1m)
        .bind(srv.stock)
        .execute(&mut *tx)
        .await?;

        sqlx::query(
            r#"
            INSERT INTO reliablesite_servers (
                product_id, description, data_center, detail, stock, is_preorder,
                setup_1_hour, setup_1_month, setup_3_month, setup_6_month, setup_12_month, setup_24_month, setup_36_month,
                recurring_1_hour, recurring_1_month, recurring_3_month, recurring_6_month, recurring_12_month, recurring_24_month, recurring_36_month,
                vaultscope_product_id, synced_at
            ) VALUES (
                $1, $2, $3, $4, $5, $6,
                $7, $8, $9, $10, $11, $12, $13,
                $14, $15, $16, $17, $18, $19, $20,
                $21, now()
            )
            ON CONFLICT (product_id) DO UPDATE SET
                description = EXCLUDED.description,
                data_center = EXCLUDED.data_center,
                detail = EXCLUDED.detail,
                stock = EXCLUDED.stock,
                is_preorder = EXCLUDED.is_preorder,
                setup_1_hour = EXCLUDED.setup_1_hour,
                setup_1_month = EXCLUDED.setup_1_month,
                setup_3_month = EXCLUDED.setup_3_month,
                setup_6_month = EXCLUDED.setup_6_month,
                setup_12_month = EXCLUDED.setup_12_month,
                setup_24_month = EXCLUDED.setup_24_month,
                setup_36_month = EXCLUDED.setup_36_month,
                recurring_1_hour = EXCLUDED.recurring_1_hour,
                recurring_1_month = EXCLUDED.recurring_1_month,
                recurring_3_month = EXCLUDED.recurring_3_month,
                recurring_6_month = EXCLUDED.recurring_6_month,
                recurring_12_month = EXCLUDED.recurring_12_month,
                recurring_24_month = EXCLUDED.recurring_24_month,
                recurring_36_month = EXCLUDED.recurring_36_month,
                vaultscope_product_id = COALESCE(reliablesite_servers.vaultscope_product_id, EXCLUDED.vaultscope_product_id),
                synced_at = now()
            "#,
        )
        .bind(srv.product_id)
        .bind(&srv.description)
        .bind(&srv.data_center)
        .bind(&srv.detail)
        .bind(srv.stock)
        .bind(srv.description.contains("Pre-Order"))
        .bind(srv.pricing.setup_1_hour)
        .bind(srv.pricing.setup_1_month)
        .bind(srv.pricing.setup_3_month)
        .bind(srv.pricing.setup_6_month)
        .bind(srv.pricing.setup_12_month)
        .bind(srv.pricing.setup_24_month)
        .bind(srv.pricing.setup_36_month)
        .bind(srv.pricing.recurring_1_hour)
        .bind(srv.pricing.recurring_1_month)
        .bind(srv.pricing.recurring_3_month)
        .bind(srv.pricing.recurring_6_month)
        .bind(srv.pricing.recurring_12_month)
        .bind(srv.pricing.recurring_24_month)
        .bind(srv.pricing.recurring_36_month)
        .bind(prod_id)
        .execute(&mut *tx)
        .await?;

        if let Some(groups) = sync_data.server_groups.get(&srv.product_id) {
            sqlx::query("DELETE FROM reliablesite_product_addon_groups WHERE product_id = $1")
                .bind(srv.product_id)
                .execute(&mut *tx)
                .await?;

            for g in groups {
                group_ids_seen.insert(g.group_id);
                sqlx::query(
                    r#"
                    INSERT INTO reliablesite_addon_groups (group_id, description, sort_order, required)
                    VALUES ($1, $2, $3, true)
                    ON CONFLICT (group_id) DO UPDATE SET
                        description = EXCLUDED.description,
                        sort_order = EXCLUDED.sort_order
                    "#,
                )
                .bind(g.group_id)
                .bind(&g.description)
                .bind(g.sort_order)
                .execute(&mut *tx)
                .await?;

                sqlx::query(
                    r#"
                    INSERT INTO reliablesite_product_addon_groups (product_id, group_id, sort_order)
                    VALUES ($1, $2, $3)
                    ON CONFLICT (product_id, group_id) DO UPDATE SET sort_order = EXCLUDED.sort_order
                    "#,
                )
                .bind(srv.product_id)
                .bind(g.group_id)
                .bind(g.sort_order)
                .execute(&mut *tx)
                .await?;
                links_count += 1;
            }
        }
    }

    for addons in sync_data.addons_by_group.values() {
        for a in addons {
            upsert_addon_tx(&mut tx, a).await?;
            addons_count += 1;
        }
    }

    tx.commit().await?;

    let saved_servers: Vec<ReliableSiteServer> = sqlx::query_as(
        "SELECT * FROM reliablesite_servers WHERE UPPER(data_center) = UPPER($1) ORDER BY product_id ASC",
    )
    .bind(data_center)
    .fetch_all(db)
    .await?;

    Ok(SyncSummary {
        data_center: data_center.to_uppercase(),
        servers_synced: sync_data.servers.len(),
        groups_synced: group_ids_seen.len(),
        product_group_links_synced: links_count,
        addons_synced: addons_count,
        servers: saved_servers,
    })
}

async fn upsert_addon_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    a: &InventoryAddon,
) -> Result<(), ApiError> {
    sqlx::query(
        r#"
        INSERT INTO reliablesite_addons (
            addon_id, group_id, addon_description, sort_order,
            setup_1_hour, setup_1_month, setup_3_month, setup_6_month, setup_12_month, setup_24_month, setup_36_month,
            recurring_1_hour, recurring_1_month, recurring_3_month, recurring_6_month, recurring_12_month, recurring_24_month, recurring_36_month
        ) VALUES (
            $1, $2, $3, $4,
            $5, $6, $7, $8, $9, $10, $11,
            $12, $13, $14, $15, $16, $17, $18
        )
        ON CONFLICT (addon_id) DO UPDATE SET
            group_id = EXCLUDED.group_id,
            addon_description = EXCLUDED.addon_description,
            sort_order = EXCLUDED.sort_order,
            setup_1_hour = EXCLUDED.setup_1_hour,
            setup_1_month = EXCLUDED.setup_1_month,
            setup_3_month = EXCLUDED.setup_3_month,
            setup_6_month = EXCLUDED.setup_6_month,
            setup_12_month = EXCLUDED.setup_12_month,
            setup_24_month = EXCLUDED.setup_24_month,
            setup_36_month = EXCLUDED.setup_36_month,
            recurring_1_hour = EXCLUDED.recurring_1_hour,
            recurring_1_month = EXCLUDED.recurring_1_month,
            recurring_3_month = EXCLUDED.recurring_3_month,
            recurring_6_month = EXCLUDED.recurring_6_month,
            recurring_12_month = EXCLUDED.recurring_12_month,
            recurring_24_month = EXCLUDED.recurring_24_month,
            recurring_36_month = EXCLUDED.recurring_36_month
        "#,
    )
    .bind(a.addon_id)
    .bind(a.group_id)
    .bind(&a.addon_description)
    .bind(a.sort_order)
    .bind(a.pricing.setup_1_hour)
    .bind(a.pricing.setup_1_month)
    .bind(a.pricing.setup_3_month)
    .bind(a.pricing.setup_6_month)
    .bind(a.pricing.setup_12_month)
    .bind(a.pricing.setup_24_month)
    .bind(a.pricing.setup_36_month)
    .bind(a.pricing.recurring_1_hour)
    .bind(a.pricing.recurring_1_month)
    .bind(a.pricing.recurring_3_month)
    .bind(a.pricing.recurring_6_month)
    .bind(a.pricing.recurring_12_month)
    .bind(a.pricing.recurring_24_month)
    .bind(a.pricing.recurring_36_month)
    .execute(&mut **tx)
    .await?;
    Ok(())
}
