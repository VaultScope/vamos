use std::collections::{BTreeMap, HashMap};
use std::str::FromStr;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use tokio::sync::RwLock;
use utoipa::ToSchema;
use uuid::Uuid;
use vaultscope_connectors::reliablesite::{
    InventoryAddon, InventoryServer, OrderBillingCycle, ReliableSite,
};
use vaultscope_db::models::{
    Product, ReliableSiteAddon, ReliableSiteAddonGroup, ReliableSiteServer, Service,
};

use crate::config::Config;
use crate::error::ApiError;
use vaultscope_db::models::Setting;

/// Retail pricing policy applied on top of ReliableSite's wholesale USD rates.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct DedicatedPricingPolicy {
    pub usd_to_eur_rate: Decimal,
    pub markup_percent: Decimal,
}

impl Default for DedicatedPricingPolicy {
    fn default() -> Self {
        Self {
            usd_to_eur_rate: Decimal::new(91, 2),
            markup_percent: Decimal::new(15, 0),
        }
    }
}

/// Active VAT/tax policy loaded from platform settings (defaults safely to § 19 UStG 0%).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaxPolicy {
    pub tax_mode: String,
    pub rate_percent: Decimal,
    pub small_business_exempt: bool,
    pub tax_notice: Option<String>,
    pub seller_tax_id: Option<String>,
}

impl Default for TaxPolicy {
    fn default() -> Self {
        Self {
            tax_mode: "small_business".to_string(),
            rate_percent: Decimal::ZERO,
            small_business_exempt: true,
            tax_notice: Some("Gemäß § 19 UStG wird keine Umsatzsteuer berechnet.".to_string()),
            seller_tax_id: None,
        }
    }
}

fn parse_setting_decimal(value: &serde_json::Value) -> Option<Decimal> {
    if let Some(s) = value.as_str() {
        s.trim().parse::<Decimal>().ok()
    } else if let Some(n) = value.as_f64() {
        Decimal::try_from(n).ok()
    } else {
        value.as_i64().map(Decimal::from)
    }
}

/// Converts a ReliableSite wholesale USD price into retail EUR with VaultScope's markup,
/// preserving negative sentinel values (`-1.0000` for unavailable billing cycles).
pub fn convert_usd_to_retail_eur(amount_usd: Decimal, policy: DedicatedPricingPolicy) -> Decimal {
    if amount_usd < Decimal::ZERO {
        return amount_usd;
    }
    if amount_usd == Decimal::ZERO {
        return Decimal::ZERO;
    }
    let markup_multiplier = Decimal::ONE + (policy.markup_percent / Decimal::from(100));
    (amount_usd * policy.usd_to_eur_rate * markup_multiplier).round_dp(2)
}

struct LiveFxCacheEntry {
    rate: Option<Decimal>,
    last_success: Option<Instant>,
    last_attempt: Instant,
}

static LIVE_FX_CACHE: OnceLock<RwLock<Option<LiveFxCacheEntry>>> = OnceLock::new();
const LIVE_FX_TTL: Duration = Duration::from_secs(900);
const LIVE_FX_FAILURE_COOLDOWN: Duration = Duration::from_secs(60);

/// Parses `{"rates":{"EUR": ...}}` (Frankfurter / ECB & open.er-api formats) with sanity bounds.
pub fn parse_live_fx_rate_json(body: &serde_json::Value) -> Option<Decimal> {
    let eur_val = body.get("rates")?.get("EUR")?;
    let dec = if let Some(n) = eur_val.as_f64() {
        Decimal::try_from(n).ok()
    } else if let Some(s) = eur_val.as_str() {
        Decimal::from_str(s.trim()).ok()
    } else {
        None
    }?;

    // Guard against corrupted/outlier feeds (realistic USD/EUR band 0.2 .. 3.0)
    if dec > Decimal::new(2, 1) && dec < Decimal::new(30, 1) {
        Some(dec.round_dp(5))
    } else {
        None
    }
}

pub async fn fetch_live_usd_to_eur_from_urls(urls: &[&str]) -> Option<Decimal> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_millis(1500))
        .user_agent("VaultScope-Billing/1.0")
        .build()
        .ok()?;

    for url in urls {
        let Ok(resp) = client.get(*url).send().await else {
            continue;
        };
        if !resp.status().is_success() {
            continue;
        }
        let Ok(json) = resp.json::<serde_json::Value>().await else {
            continue;
        };
        if let Some(rate) = parse_live_fx_rate_json(&json) {
            return Some(rate);
        }
    }
    None
}

/// Resolves the live ECB USD->EUR rate (15 min cache, 60s retry cooldown when offline).
pub async fn resolve_live_usd_to_eur_rate() -> Option<Decimal> {
    const FX_URLS: [&str; 2] = [
        "https://api.frankfurter.app/latest?from=USD&to=EUR",
        "https://open.er-api.com/v6/latest/USD",
    ];
    let lock = LIVE_FX_CACHE.get_or_init(|| RwLock::new(None));
    let now = Instant::now();

    {
        let guard = lock.read().await;
        if let Some(entry) = guard.as_ref() {
            let fresh = entry
                .last_success
                .map(|t| now.duration_since(t) < LIVE_FX_TTL)
                .unwrap_or(false);
            if fresh || now.duration_since(entry.last_attempt) < LIVE_FX_FAILURE_COOLDOWN {
                return entry.rate;
            }
        }
    }

    let mut guard = lock.write().await;
    let now = Instant::now();
    if let Some(entry) = guard.as_ref() {
        let fresh = entry
            .last_success
            .map(|t| now.duration_since(t) < LIVE_FX_TTL)
            .unwrap_or(false);
        if fresh || now.duration_since(entry.last_attempt) < LIVE_FX_FAILURE_COOLDOWN {
            return entry.rate;
        }
    }

    if let Some(rate) = fetch_live_usd_to_eur_from_urls(&FX_URLS).await {
        tracing::info!(usd_to_eur_rate = %rate, "Refreshed live USD->EUR exchange rate");
        *guard = Some(LiveFxCacheEntry {
            rate: Some(rate),
            last_success: Some(now),
            last_attempt: now,
        });
        Some(rate)
    } else {
        let prev_rate = guard.as_ref().and_then(|e| e.rate);
        let prev_success = guard.as_ref().and_then(|e| e.last_success);
        *guard = Some(LiveFxCacheEntry {
            rate: prev_rate,
            last_success: prev_success,
            last_attempt: now,
        });
        prev_rate
    }
}

/// Loads the active ReliableSite USD->EUR FX rate & markup % from live ECB FX + `settings` (fallback to `Config`).
pub async fn load_pricing_policy(db: &PgPool, config: &Config) -> DedicatedPricingPolicy {
    let mut policy = DedicatedPricingPolicy {
        usd_to_eur_rate: config.reliablesite_usd_to_eur_rate,
        markup_percent: config.reliablesite_markup_percent,
    };
    let mut auto_fx_enabled = true;

    if let Ok(rows) = sqlx::query_as::<_, Setting>(
        "SELECT * FROM settings WHERE key IN ('reliablesite_auto_fx_enabled', 'reliablesite_usd_to_eur_rate', 'reliablesite_markup_percent')",
    )
    .fetch_all(db)
    .await
    {
        for s in rows {
            match s.key.as_str() {
                "reliablesite_auto_fx_enabled" => {
                    if let Some(b) = s.value.as_bool() {
                        auto_fx_enabled = b;
                    } else if let Some(str_val) = s.value.as_str() {
                        auto_fx_enabled = !matches!(str_val.trim().to_ascii_lowercase().as_str(), "false" | "0" | "no");
                    }
                }
                "reliablesite_usd_to_eur_rate" => {
                    if let Some(d) = parse_setting_decimal(&s.value).filter(|d| *d > Decimal::ZERO) {
                        policy.usd_to_eur_rate = d;
                    }
                }
                "reliablesite_markup_percent" => {
                    if let Some(d) = parse_setting_decimal(&s.value).filter(|d| *d >= Decimal::ZERO) {
                        policy.markup_percent = d;
                    }
                }
                _ => {}
            }
        }
    }

    if config.reliablesite_live_fx_enabled && auto_fx_enabled {
        if let Some(live_rate) = resolve_live_usd_to_eur_rate().await {
            policy.usd_to_eur_rate = live_rate;
        }
    }

    policy
}

/// Loads the active tax rate policy (`taxMode`, `defaultTaxRate`, `vatId`, `taxNumber`) from `settings`.
/// Safely falls back to Kleinunternehmerregelung (§ 19 UStG, 0% VAT) when unset or configured as `small_business`.
pub async fn load_tax_policy(db: &PgPool, config: &Config) -> TaxPolicy {
    let mut tax_mode: Option<String> = None;
    let mut rate_override: Option<Decimal> = None;
    let mut vat_id: Option<String> = None;
    let mut tax_number: Option<String> = None;
    let mut custom_notice: Option<String> = None;

    if let Ok(rows) = sqlx::query_as::<_, Setting>(
        "SELECT * FROM settings WHERE key IN ('taxMode', 'defaultTaxRate', 'tax_rate_percent', 'taxNotice', 'vatId', 'taxNumber')",
    )
    .fetch_all(db)
    .await
    {
        for s in rows {
            match s.key.as_str() {
                "taxMode" => {
                    if let Some(v) = s.value.as_str() {
                        let trimmed = v.trim();
                        if !trimmed.is_empty() {
                            tax_mode = Some(trimmed.to_string());
                        }
                    }
                }
                "defaultTaxRate" | "tax_rate_percent" => {
                    if let Some(d) = parse_setting_decimal(&s.value)
                        .filter(|d| *d >= Decimal::ZERO && *d <= Decimal::from(100))
                    {
                        rate_override = Some(d.round_dp(2));
                    }
                }
                "taxNotice" => {
                    if let Some(v) = s.value.as_str().map(str::trim).filter(|v| !v.is_empty()) {
                        custom_notice = Some(v.to_string());
                    }
                }
                "vatId" => {
                    if let Some(v) = s.value.as_str().map(str::trim).filter(|v| !v.is_empty()) {
                        vat_id = Some(v.to_string());
                    }
                }
                "taxNumber" => {
                    if let Some(v) = s.value.as_str().map(str::trim).filter(|v| !v.is_empty()) {
                        tax_number = Some(v.to_string());
                    }
                }
                _ => {}
            }
        }
    }

    let mode = tax_mode.unwrap_or_else(|| {
        if config.default_tax_rate_percent > Decimal::ZERO {
            "exclusive".to_string()
        } else {
            "small_business".to_string()
        }
    });

    let effective_rate = if mode.eq_ignore_ascii_case("small_business") {
        Decimal::ZERO
    } else {
        rate_override.unwrap_or(config.default_tax_rate_percent).round_dp(2)
    };

    let small_business_exempt =
        mode.eq_ignore_ascii_case("small_business") || effective_rate <= Decimal::ZERO;

    let tax_notice = if small_business_exempt {
        Some(
            custom_notice.unwrap_or_else(|| {
                "Gemäß § 19 UStG wird keine Umsatzsteuer berechnet.".to_string()
            }),
        )
    } else {
        custom_notice
    };

    let seller_tax_id = vat_id.or(tax_number);

    TaxPolicy {
        tax_mode: if small_business_exempt {
            "small_business".to_string()
        } else {
            mode
        },
        rate_percent: if small_business_exempt {
            Decimal::ZERO
        } else {
            effective_rate
        },
        small_business_exempt,
        tax_notice,
        seller_tax_id,
    }
}

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
    #[serde(default)]
    pub tax_rate_percent: Decimal,
    #[serde(default)]
    pub small_business_exempt: bool,
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
    #[serde(default)]
    pub tax_rate_percent: Decimal,
    #[serde(default)]
    pub small_business_exempt: bool,
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
            tax_rate_percent: self.tax_rate_percent,
            small_business_exempt: self.small_business_exempt,
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
    _product: &Product,
    srv: &ReliableSiteServer,
    cycle: OrderBillingCycle,
    upstream_recurring: Decimal,
    policy: DedicatedPricingPolicy,
) -> (Decimal, Decimal) {
    let upstream_setup = server_setup_for_cycle(srv, cycle);
    let converted_rec = convert_usd_to_retail_eur(upstream_recurring, policy);
    let converted_setup = convert_usd_to_retail_eur(upstream_setup, policy);
    (converted_rec, converted_setup)
}

#[allow(dead_code)]
pub async fn get_product_options(
    db: &PgPool,
    product: &Product,
    live_stock_override: Option<i32>,
) -> Result<ReliableSiteProductOptionsResponse, ApiError> {
    get_product_options_with_policy(
        db,
        product,
        live_stock_override,
        DedicatedPricingPolicy::default(),
    )
    .await
}

pub async fn get_product_options_with_policy(
    db: &PgPool,
    product: &Product,
    live_stock_override: Option<i32>,
    policy: DedicatedPricingPolicy,
) -> Result<ReliableSiteProductOptionsResponse, ApiError> {
    let mut server = load_server_for_product(db, product).await?;
    if let Some(s) = live_stock_override {
        server.stock = s;
    }

    let groups = load_server_groups_with_addons(db, server.product_id).await?;

    let mut available_cycles = Vec::new();
    for cycle in OrderBillingCycle::ALL {
        if let Some(upstream_rec) = server_recurring_for_cycle(&server, cycle) {
            let (recurring, setup_fee) =
                retail_base_for_cycle(product, &server, cycle, upstream_rec, policy);
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

#[allow(dead_code)]
pub async fn get_dedicated_options(
    db: &PgPool,
    product: &Product,
    live_stock_override: Option<i32>,
) -> Result<DedicatedProductOptionsResponse, ApiError> {
    get_dedicated_options_with_policy(
        db,
        product,
        live_stock_override,
        DedicatedPricingPolicy::default(),
    )
    .await
}

pub async fn get_dedicated_options_with_policy(
    db: &PgPool,
    product: &Product,
    live_stock_override: Option<i32>,
    policy: DedicatedPricingPolicy,
) -> Result<DedicatedProductOptionsResponse, ApiError> {
    let raw = get_product_options_with_policy(db, product, live_stock_override, policy).await?;
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
                        setup_1_month: convert_usd_to_retail_eur(o.setup_1_month, policy),
                        setup_3_month: convert_usd_to_retail_eur(o.setup_3_month, policy),
                        setup_6_month: convert_usd_to_retail_eur(o.setup_6_month, policy),
                        setup_12_month: convert_usd_to_retail_eur(o.setup_12_month, policy),
                        setup_24_month: convert_usd_to_retail_eur(o.setup_24_month, policy),
                        setup_36_month: convert_usd_to_retail_eur(o.setup_36_month, policy),
                        recurring_1_month: convert_usd_to_retail_eur(o.recurring_1_month, policy),
                        recurring_3_month: convert_usd_to_retail_eur(o.recurring_3_month, policy),
                        recurring_6_month: convert_usd_to_retail_eur(o.recurring_6_month, policy),
                        recurring_12_month: convert_usd_to_retail_eur(o.recurring_12_month, policy),
                        recurring_24_month: convert_usd_to_retail_eur(o.recurring_24_month, policy),
                        recurring_36_month: convert_usd_to_retail_eur(o.recurring_36_month, policy),
                    })
                    .collect(),
            })
            .collect(),
    })
}

#[allow(dead_code)]
pub fn sanitize_product_for_storefront(product: Product) -> Product {
    sanitize_product_for_storefront_with_policy(product, DedicatedPricingPolicy::default())
}

pub fn sanitize_product_for_storefront_with_policy(
    mut product: Product,
    policy: DedicatedPricingPolicy,
) -> Product {
    if product.provider == "reliablesite" {
        if product.cost > Decimal::ZERO {
            product.price = convert_usd_to_retail_eur(product.cost, policy);
        }
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
    product.cost = Decimal::ZERO;
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
        obj.remove("promo_code");
        obj.remove("promoCode");
        obj.remove("password");
        obj.remove("root_password");
        if let Some(raw_obj) = obj.get_mut("raw_response").and_then(|v| v.as_object_mut()) {
            raw_obj.remove("provider");
            raw_obj.remove("paymentMethod");
            raw_obj.remove("productId");
            raw_obj.remove("productIds");
            raw_obj.remove("orderId");
            raw_obj.remove("invoiceId");
            raw_obj.remove("promoCode");
            raw_obj.remove("promo_code");
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

#[allow(dead_code)]
pub async fn compute_quote(
    db: &PgPool,
    product: &Product,
    billing_cycle_raw: Option<&str>,
    supplied_options: Option<&BTreeMap<String, u32>>,
) -> Result<ReliableSiteQuote, ApiError> {
    compute_quote_with_policy(
        db,
        product,
        billing_cycle_raw,
        supplied_options,
        DedicatedPricingPolicy::default(),
        &TaxPolicy::default(),
    )
    .await
}

pub async fn compute_quote_with_policy(
    db: &PgPool,
    product: &Product,
    billing_cycle_raw: Option<&str>,
    supplied_options: Option<&BTreeMap<String, u32>>,
    policy: DedicatedPricingPolicy,
    tax_policy: &TaxPolicy,
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

    let (base_recurring, base_setup) =
        retail_base_for_cycle(product, &server, cycle, upstream_rec, policy);
    let groups = load_server_groups_with_addons(db, server.product_id).await?;

    let group_map: HashMap<i32, &ReliableSiteConfigGroupView> =
        groups.iter().map(|g| (g.group_id, g)).collect();

    let resolved_map: BTreeMap<String, u32> = match supplied_options {
        Some(map) => {
            let mut canonical_map = BTreeMap::new();
            for (gid_str, addon_id) in map {
                let trimmed = gid_str.trim();
                let gid: i32 = trimmed.parse().map_err(|_| {
                    ApiError::BadRequest(format!("Invalid configuration group '{}'", gid_str))
                })?;
                let canonical_key = gid.to_string();
                if gid_str != &canonical_key {
                    return Err(ApiError::BadRequest(format!(
                        "Non-canonical configuration group key '{}'",
                        gid_str
                    )));
                }
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
                if canonical_map.insert(canonical_key, *addon_id).is_some() {
                    return Err(ApiError::BadRequest(format!(
                        "Duplicate configuration group {}",
                        gid
                    )));
                }
            }
            // Verify all required groups are present when explicit config_options were provided
            for grp in &groups {
                if grp.required && !canonical_map.contains_key(&grp.group_id.to_string()) {
                    return Err(ApiError::BadRequest(format!(
                        "Missing required configuration option for group {} ({})",
                        grp.group_id, grp.description
                    )));
                }
            }
            canonical_map
        }
        None => {
            // Auto-fill baseline default option for each group
            let opts = get_product_options_with_policy(db, product, None, policy).await?;
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

        let add_rec_usd = addon_recurring_for_cycle(addon, cycle).ok_or_else(|| {
            ApiError::BadRequest(format!(
                "Addon '{}' is not available on billing cycle '{}'",
                addon.addon_description,
                cycle.as_str()
            ))
        })?;
        let add_setup_usd = addon_setup_for_cycle(addon, cycle);

        let add_rec = convert_usd_to_retail_eur(add_rec_usd, policy);
        let add_setup = convert_usd_to_retail_eur(add_setup_usd, policy);

        recurring_net += add_rec;
        setup_fee_net += add_setup;

        line_items.push(QuoteLineItem {
            group_id: Some(grp.group_id),
            group_name: grp.description.clone(),
            addon_id: Some(addon.addon_id),
            label: addon.addon_description.clone(),
            recurring: add_rec,
            setup_fee: add_setup,
        });
    }

    let subtotal_net = (recurring_net + setup_fee_net).round_dp(2);
    let tax_rate_pct = if tax_policy.small_business_exempt {
        Decimal::ZERO
    } else {
        tax_policy.rate_percent.max(Decimal::ZERO).round_dp(2)
    };
    let vat_amount = (subtotal_net * (tax_rate_pct / Decimal::from(100))).round_dp(2);
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
        tax_rate_percent: tax_rate_pct,
        small_business_exempt: tax_policy.small_business_exempt,
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

#[allow(dead_code)]
pub async fn sync_datacenter_to_db(
    db: &PgPool,
    rs: &ReliableSite,
    data_center: &str,
) -> Result<SyncSummary, ApiError> {
    sync_datacenter_to_db_with_policy(db, rs, data_center, DedicatedPricingPolicy::default()).await
}

pub async fn sync_datacenter_to_db_with_policy(
    db: &PgPool,
    rs: &ReliableSite,
    data_center: &str,
    policy: DedicatedPricingPolicy,
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
        let setup_1m_usd = srv.pricing.setup_1_month.max(Decimal::ZERO).round_dp(2);
        let retail_price_1m = convert_usd_to_retail_eur(cost_1m, policy);
        let retail_setup_1m = convert_usd_to_retail_eur(setup_1m_usd, policy);

        sqlx::query(
            r#"
            INSERT INTO products (id, name, category, provider, target, specs, cost, price, setup_fee, stock, user_limit, billing_cycle, hidden)
            VALUES ($1, $2, 'dedicated', 'reliablesite', $3, $4, $5, $6, $7, $8, 0, 'monthly', false)
            ON CONFLICT (id) DO UPDATE SET
                name = EXCLUDED.name,
                specs = EXCLUDED.specs,
                cost = EXCLUDED.cost,
                price = CASE
                    WHEN EXCLUDED.cost > 0 THEN EXCLUDED.price
                    ELSE products.price
                END,
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
        .bind(retail_price_1m)
        .bind(retail_setup_1m)
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
