use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Supported ReliableSite Order API billing cycle terms (`POST /v1/Orders`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum OrderBillingCycle {
    Monthly,
    Quarterly,
    Semiannually,
    Annually,
    Biennially,
    Triennially,
}

impl OrderBillingCycle {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Monthly => "monthly",
            Self::Quarterly => "quarterly",
            Self::Semiannually => "semiannually",
            Self::Annually => "annually",
            Self::Biennially => "biennially",
            Self::Triennially => "triennially",
        }
    }

    pub fn from_str_loose(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "monthly" | "1m" | "1_month" => Some(Self::Monthly),
            "quarterly" | "3m" | "3_month" => Some(Self::Quarterly),
            "semiannually" | "semi_annually" | "6m" | "6_month" => Some(Self::Semiannually),
            "annually" | "yearly" | "12m" | "12_month" => Some(Self::Annually),
            "biennially" | "24m" | "24_month" => Some(Self::Biennially),
            "triennially" | "36m" | "36_month" => Some(Self::Triennially),
            _ => None,
        }
    }

    /// Number of calendar days represented by this billing term for renewal scheduling.
    pub fn period_days(&self) -> i64 {
        match self {
            Self::Monthly => 30,
            Self::Quarterly => 90,
            Self::Semiannually => 180,
            Self::Annually => 365,
            Self::Biennially => 730,
            Self::Triennially => 1095,
        }
    }

    /// All billing cycles ordered from shortest to longest.
    pub const ALL: [Self; 6] = [
        Self::Monthly,
        Self::Quarterly,
        Self::Semiannually,
        Self::Annually,
        Self::Biennially,
        Self::Triennially,
    ];
}

/// Complete pricing schedule matching ReliableSite Inventory `Setup_*` / `Recurring_*` fields.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, ToSchema)]
pub struct InventoryPricing {
    pub setup_1_hour: Decimal,
    pub setup_1_month: Decimal,
    pub setup_3_month: Decimal,
    pub setup_6_month: Decimal,
    pub setup_12_month: Decimal,
    pub setup_24_month: Decimal,
    pub setup_36_month: Decimal,
    pub recurring_1_hour: Decimal,
    pub recurring_1_month: Decimal,
    pub recurring_3_month: Decimal,
    pub recurring_6_month: Decimal,
    pub recurring_12_month: Decimal,
    pub recurring_24_month: Decimal,
    pub recurring_36_month: Decimal,
}

impl InventoryPricing {
    /// Returns the recurring price for the billing cycle, or `None` if ReliableSite marks it unavailable (`< 0`, e.g. `-1.0000`).
    pub fn recurring_for(&self, cycle: OrderBillingCycle) -> Option<Decimal> {
        let val = match cycle {
            OrderBillingCycle::Monthly => self.recurring_1_month,
            OrderBillingCycle::Quarterly => self.recurring_3_month,
            OrderBillingCycle::Semiannually => self.recurring_6_month,
            OrderBillingCycle::Annually => self.recurring_12_month,
            OrderBillingCycle::Biennially => self.recurring_24_month,
            OrderBillingCycle::Triennially => self.recurring_36_month,
        };
        if val < Decimal::ZERO {
            None
        } else {
            Some(val)
        }
    }

    /// Returns the setup fee for the given billing cycle (`0.00` when negative sentinel is used).
    pub fn setup_for(&self, cycle: OrderBillingCycle) -> Decimal {
        let val = match cycle {
            OrderBillingCycle::Monthly => self.setup_1_month,
            OrderBillingCycle::Quarterly => self.setup_3_month,
            OrderBillingCycle::Semiannually => self.setup_6_month,
            OrderBillingCycle::Annually => self.setup_12_month,
            OrderBillingCycle::Biennially => self.setup_24_month,
            OrderBillingCycle::Triennially => self.setup_36_month,
        };
        if val < Decimal::ZERO {
            Decimal::ZERO
        } else {
            val
        }
    }
}

/// Real `Server_Details` record returned by ReliableSite SOAP `ServersList`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, ToSchema)]
pub struct InventoryServer {
    pub product_id: i32,
    pub description: String,
    pub data_center: String,
    pub detail: String,
    pub stock: i32,
    pub pricing: InventoryPricing,
}

/// Real `AddonGroups_Details` record returned by ReliableSite SOAP `AddonGroupsList`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, ToSchema)]
pub struct InventoryAddonGroup {
    pub group_id: i32,
    pub description: String,
    pub sort_order: i32,
}

/// Real `Addons_Details` record returned by ReliableSite SOAP `AddonsList`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, ToSchema)]
pub struct InventoryAddon {
    pub addon_id: i32,
    pub group_id: i32,
    pub addon_description: String,
    pub sort_order: i32,
    pub pricing: InventoryPricing,
}

/// Response from `POST /v1/Login/Token`.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct LoginTokenResponse {
    pub status: Option<bool>,
    pub message: Option<String>,
    pub token: Option<String>,
}

/// Response from `GET /v1/Login/VerifyToken`.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct VerifyTokenResponse {
    pub status: bool,
    pub message: String,
    pub token_expiration_timestamp: Option<DateTime<Utc>>,
    pub user_id: Option<i64>,
}

/// Payment method entry from `GET /v1/PaymentMethods`.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct PaymentMethodInfo {
    pub module: String,
    pub display_name: String,
}

/// Response from `GET /v1/PaymentMethods`.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct PaymentMethodsResponse {
    #[serde(default)]
    pub payment_methods: Vec<PaymentMethodInfo>,
}

/// Response from `GET /v1/Products/{productId}/Stock`.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ProductStockResponse {
    pub product_id: i32,
    pub stock: i32,
    pub stock_controlled: bool,
}

/// Reseller promo code entry from `GET /v1/Reseller/PromoCodes`.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ResellerPromoCode {
    pub configuration_id: i32,
    pub promo_code: String,
    pub regular_price: Decimal,
    pub sale_price: Decimal,
}

/// Response from `GET /v1/Reseller/PromoCodes`.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ResellerPromoCodesResponse {
    #[serde(default)]
    pub promo_codes: Vec<ResellerPromoCode>,
}

/// Request payload for `POST /v1/Orders`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct CreateOrderRequest {
    pub product_id: i32,
    pub billing_cycle: OrderBillingCycle,
    pub hostname: String,
    pub payment_method: String,
    /// Keys are Inventory API `Group_Id` strings, values are positive Inventory API `Addon_Id` integers.
    pub config_options: BTreeMap<String, u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub promo_code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub order_notes: Option<String>,
}

/// Response payload from `POST /v1/Orders`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct CreateOrderResponse {
    pub order_id: i64,
    pub invoice_id: Option<i64>,
    #[serde(default)]
    pub product_ids: String,
    pub payment_method: String,
    pub product_id: i32,
    #[serde(default)]
    pub replayed: bool,
}
