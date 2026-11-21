use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use utoipa::ToSchema;
use uuid::Uuid;

// --- Enum types (mirroring Postgres enums) ---

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::Type, PartialEq, ToSchema)]
#[sqlx(type_name = "customer_status", rename_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum CustomerStatus {
    Active,
    Suspended,
    Banned,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::Type, PartialEq, ToSchema)]
#[sqlx(type_name = "service_status", rename_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum ServiceStatus {
    Running,
    Suspended,
    Pending,
    Terminated,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::Type, PartialEq, ToSchema)]
#[sqlx(type_name = "invoice_status", rename_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum InvoiceStatus {
    Draft,
    Pending,
    Paid,
    Overdue,
    Void,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::Type, PartialEq, ToSchema)]
#[sqlx(type_name = "ticket_status", rename_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum TicketStatus {
    Open,
    InProgress,
    WaitingCustomer,
    Closed,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::Type, PartialEq, ToSchema)]
#[sqlx(type_name = "ticket_priority", rename_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum TicketPriority {
    Low,
    Normal,
    High,
    Critical,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::Type, PartialEq, ToSchema)]
#[sqlx(type_name = "ticket_category", rename_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum TicketCategory {
    Support,
    Abuse,
    Dmca,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::Type, PartialEq, ToSchema)]
#[sqlx(type_name = "job_status", rename_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum JobStatus {
    Queued,
    InProgress,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::Type, PartialEq, ToSchema)]
#[sqlx(type_name = "billing_cycle", rename_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum BillingCycle {
    Monthly,
    Yearly,
    OneTime,
}


#[derive(Debug, Clone, Serialize, Deserialize, sqlx::Type, PartialEq, ToSchema)]
#[sqlx(type_name = "actor_type", rename_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum ActorType {
    Staff,
    Customer,
    System,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::Type, PartialEq, ToSchema)]
#[sqlx(type_name = "email_delivery_status", rename_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum EmailDeliveryStatus {
    Delivered,
    Bounced,
    Failed,
    Queued,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::Type, PartialEq, ToSchema)]
#[sqlx(type_name = "coupon_discount_type", rename_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum CouponDiscountType {
    Percentage,
    Fixed,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::Type, PartialEq, ToSchema)]
#[sqlx(type_name = "coupon_status", rename_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum CouponStatus {
    Active,
    Exhausted,
    Expired,
    Disabled,
}

// --- Row structs ---

#[derive(Debug, Clone, Serialize, Deserialize, FromRow, ToSchema)]
pub struct Role {
    pub id: Uuid,
    pub name: String,
    pub permissions: Vec<String>,
    pub mapped_group: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow, ToSchema)]
pub struct Staff {
    pub id: Uuid,
    pub external_id: String,
    pub username: String,
    pub first_name: String,
    pub last_name: String,
    pub email: String,
    pub email_verified: bool,
    pub avatar_url: String,
    pub role_id: Uuid,
    pub mfa_enabled: bool,
    pub last_login: Option<DateTime<Utc>>,
    pub last_login_ip: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    #[serde(default, skip_serializing)]
    pub password_hash: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow, ToSchema)]
pub struct Customer {
    pub id: Uuid,
    pub external_id: String,
    pub name: String,
    pub email: String,
    pub phone: String,
    pub company: String,
    pub address: String,
    pub postal_code: String,
    pub city: String,
    pub country: String,
    pub vat_id: String,
    pub status: CustomerStatus,
    pub two_factor_enabled: bool,
    pub email_verified: bool,
    pub notes: String,
    pub stripe_customer_id: Option<String>,
    pub created_at: DateTime<Utc>,
    pub last_login: Option<DateTime<Utc>>,
    pub updated_at: DateTime<Utc>,
    #[serde(default, skip_serializing)]
    pub password_hash: Option<String>,
    #[serde(default, skip_serializing)]
    pub verification_token: Option<String>,
    #[serde(default, skip_serializing)]
    pub totp_secret: Option<String>,
    #[serde(default)]
    pub vat_verified: bool,
    #[serde(default)]
    #[sqlx(default)]
    pub username: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow, ToSchema)]
pub struct Product {
    pub id: Uuid,
    pub name: String,
    pub category: String,
    pub provider: String,
    pub target: String,
    pub specs: serde_json::Value,
    pub cost: rust_decimal::Decimal,
    pub price: rust_decimal::Decimal,
    pub setup_fee: rust_decimal::Decimal,
    pub stock: i32,
    pub user_limit: i32,
    pub billing_cycle: BillingCycle,
    pub hidden: bool,
    pub service_form_id: Option<Uuid>,
    #[serde(default)]
    pub trial_days: Option<i32>,
    #[serde(default)]
    pub trial_quota_gb: Option<i32>,
    #[serde(default)]
    pub quota_gb: Option<i32>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

fn default_order_state() -> String {
    "active".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow, ToSchema)]
pub struct Service {
    pub id: Uuid,
    pub customer_id: Uuid,
    pub product_id: Uuid,
    pub name: String,
    pub status: ServiceStatus,
    pub provider_resource_id: String,
    pub ip: String,
    pub hostname: String,
    pub config: serde_json::Value,
    pub price: rust_decimal::Decimal,
    pub next_due: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    #[serde(default)]
    pub cancel_at_period_end: bool,
    pub cancellation_reason: Option<String>,
    pub cancellation_feedback: Option<String>,
    pub stripe_payment_intent_id: Option<String>,
    #[serde(default)]
    pub is_trial: bool,
    #[serde(default)]
    pub trial_ends_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub partner_id: Option<Uuid>,
    #[serde(default)]
    pub quota_gb: Option<i32>,
    #[serde(default = "default_order_state")]
    pub order_state: String,
    #[serde(default)]
    pub order_state_note: Option<String>,
    #[serde(default)]
    pub order_state_updated_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default, ToSchema)]
pub struct InvoiceBillingSnapshot {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub company: String,
    #[serde(default)]
    pub address_line1: String,
    #[serde(default)]
    pub postal_code: String,
    #[serde(default)]
    pub city: String,
    #[serde(default)]
    pub country: String,
    #[serde(default)]
    pub vat_id: String,
    #[serde(default)]
    pub snapshot_complete: bool,
    #[serde(default)]
    pub missing_fields: Vec<String>,
    #[serde(default)]
    pub snapshot_source: String,
    #[serde(default)]
    pub captured_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow, ToSchema)]
pub struct Invoice {
    pub id: Uuid,
    pub invoice_number: String,
    pub customer_id: Option<Uuid>,
    pub status: InvoiceStatus,
    pub subtotal: rust_decimal::Decimal,
    pub tax_rate: rust_decimal::Decimal,
    pub tax_amount: rust_decimal::Decimal,
    pub total: rust_decimal::Decimal,
    pub issue_date: NaiveDate,
    pub due_date: NaiveDate,
    pub paid_at: Option<DateTime<Utc>>,
    pub stripe_payment_intent_id: Option<String>,
    pub notes: String,
    #[serde(default)]
    #[schema(value_type = Option<InvoiceBillingSnapshot>)]
    pub billing_snapshot: Option<sqlx::types::Json<InvoiceBillingSnapshot>>,
    #[serde(default)]
    pub tax_regime: String,
    #[serde(default)]
    pub retention_until: Option<NaiveDate>,
    #[serde(default)]
    pub retention_hold: bool,
    #[serde(default)]
    pub corrects_invoice_id: Option<Uuid>,
    #[serde(default)]
    pub retention_resolution_approved: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow, ToSchema)]
pub struct InvoiceLineItem {
    pub id: Uuid,
    pub invoice_id: Uuid,
    pub description: String,
    pub quantity: i32,
    pub unit_price: rust_decimal::Decimal,
    pub total: rust_decimal::Decimal,
    pub service_id: Option<Uuid>,
    pub sort_order: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow, ToSchema)]
pub struct Coupon {
    pub id: Uuid,
    pub code: String,
    pub discount_type: CouponDiscountType,
    pub discount_value: rust_decimal::Decimal,
    pub usage_limit: Option<i32>,
    pub usage_count: i32,
    pub expires_at: Option<DateTime<Utc>>,
    pub status: CouponStatus,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow, ToSchema)]
pub struct TaxRate {
    pub id: Uuid,
    pub name: String,
    pub country: String,
    pub rate: rust_decimal::Decimal,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow, ToSchema)]
pub struct Ticket {
    pub id: Uuid,
    pub ticket_number: String,
    pub customer_id: Uuid,
    pub category: TicketCategory,
    pub subject: String,
    pub status: TicketStatus,
    pub priority: TicketPriority,
    pub assignee_id: Option<Uuid>,
    pub mailbox: String,
    pub related_service_id: Option<Uuid>,
    pub ip: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow, ToSchema)]
pub struct TicketMessage {
    pub id: Uuid,
    pub ticket_id: Uuid,
    pub author_id: Uuid,
    pub author_type: ActorType,
    pub content: String,
    pub internal: bool,
    #[serde(default)]
    pub attachments: serde_json::Value,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow, ToSchema)]
pub struct Department {
    pub id: Uuid,
    pub name: String,
    pub mailbox: String,
    pub default_assignee_id: Option<Uuid>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow, ToSchema)]
pub struct Job {
    pub id: Uuid,
    pub task: String,
    pub target_api: String,
    pub customer_id: Option<Uuid>,
    pub service_id: Option<Uuid>,
    pub status: JobStatus,
    pub error: Option<String>,
    pub request_payload: Option<serde_json::Value>,
    pub response_payload: Option<serde_json::Value>,
    pub attempts: i32,
    pub max_attempts: i32,
    pub scheduled_at: DateTime<Utc>,
    pub started_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow, ToSchema)]
pub struct ActivityLog {
    pub id: Uuid,
    pub actor_id: Option<Uuid>,
    pub actor_type: ActorType,
    pub actor_name: String,
    pub category: String,
    pub action: String,
    pub target: String,
    pub detail: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow, ToSchema)]
pub struct Notification {
    pub id: Uuid,
    pub category: String,
    pub title: String,
    pub detail: String,
    pub severity: String,
    pub read: bool,
    pub resolved: bool,
    pub actions: serde_json::Value,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow, ToSchema)]
pub struct Mailbox {
    pub id: Uuid,
    pub name: String,
    pub host: String,
    pub port: i32,
    pub user_name: String,
    #[serde(default, skip_serializing)]
    pub password: String,
    pub is_default: bool,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow, ToSchema)]
pub struct EmailTemplate {
    pub id: Uuid,
    pub name: String,
    pub category: String,
    pub subject: String,
    pub body: String,
    pub enabled: bool,
    pub variables: serde_json::Value,
    pub mailbox_id: Option<Uuid>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow, ToSchema)]
pub struct EmailLog {
    pub id: Uuid,
    pub recipient: String,
    pub subject: String,
    pub template_id: Option<Uuid>,
    pub status: EmailDeliveryStatus,
    pub error: Option<String>,
    pub sent_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow, ToSchema)]
pub struct Setting {
    pub key: String,
    pub value: serde_json::Value,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow, ToSchema)]
pub struct ServiceForm {
    pub id: Uuid,
    pub name: String,
    pub description: String,
    pub fields: serde_json::Value,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow, ToSchema)]
pub struct InternalTool {
    pub id: Uuid,
    pub name: String,
    pub url: String,
    pub required_permission: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow, ToSchema)]
pub struct BlogPost {
    pub id: uuid::Uuid,
    pub title: String,
    pub slug: String,
    pub content: String,
    pub author_id: uuid::Uuid,
    pub status: String,
    pub published_at: Option<chrono::DateTime<chrono::Utc>>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
    pub excerpt: String,
    pub image_url: Option<String>,
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow, ToSchema)]
pub struct CustomerBillingEmail {
    pub id: Uuid,
    pub customer_id: Uuid,
    pub email: String,
    pub verified: bool,
    #[serde(default, skip_serializing)]
    pub verification_token: Option<String>,
    pub created_at: DateTime<Utc>,
    pub verified_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow, ToSchema)]
pub struct Partner {
    pub id: Uuid,
    pub slug: String,
    pub name: String,
    pub headline: Option<String>,
    pub description: Option<String>,
    pub badge_text: Option<String>,
    pub product_id: Uuid,
    pub trial_days_override: Option<i32>,
    pub trial_quota_gb_override: Option<i32>,
    pub quota_gb_override: Option<i32>,
    pub limit_one_per_user: bool,
    pub active: bool,
    pub clicks: i32,
    pub conversions: i32,
    pub commission_type: Option<String>,
    pub commission_rate: Option<rust_decimal::Decimal>,
    pub commission_fixed_eur: Option<rust_decimal::Decimal>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow, ToSchema)]
pub struct ReliableSiteServer {
    pub product_id: i32,
    pub description: String,
    pub data_center: String,
    pub detail: String,
    pub stock: i32,
    pub is_preorder: bool,
    pub setup_1_hour: rust_decimal::Decimal,
    pub setup_1_month: rust_decimal::Decimal,
    pub setup_3_month: rust_decimal::Decimal,
    pub setup_6_month: rust_decimal::Decimal,
    pub setup_12_month: rust_decimal::Decimal,
    pub setup_24_month: rust_decimal::Decimal,
    pub setup_36_month: rust_decimal::Decimal,
    pub recurring_1_hour: rust_decimal::Decimal,
    pub recurring_1_month: rust_decimal::Decimal,
    pub recurring_3_month: rust_decimal::Decimal,
    pub recurring_6_month: rust_decimal::Decimal,
    pub recurring_12_month: rust_decimal::Decimal,
    pub recurring_24_month: rust_decimal::Decimal,
    pub recurring_36_month: rust_decimal::Decimal,
    pub vaultscope_product_id: Option<Uuid>,
    pub reseller_promo_code: Option<String>,
    pub synced_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow, ToSchema)]
pub struct ReliableSiteAddonGroup {
    pub group_id: i32,
    pub description: String,
    pub sort_order: i32,
    pub required: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow, ToSchema)]
pub struct ReliableSiteAddon {
    pub addon_id: i32,
    pub group_id: i32,
    pub addon_description: String,
    pub sort_order: i32,
    pub setup_1_hour: rust_decimal::Decimal,
    pub setup_1_month: rust_decimal::Decimal,
    pub setup_3_month: rust_decimal::Decimal,
    pub setup_6_month: rust_decimal::Decimal,
    pub setup_12_month: rust_decimal::Decimal,
    pub setup_24_month: rust_decimal::Decimal,
    pub setup_36_month: rust_decimal::Decimal,
    pub recurring_1_hour: rust_decimal::Decimal,
    pub recurring_1_month: rust_decimal::Decimal,
    pub recurring_3_month: rust_decimal::Decimal,
    pub recurring_6_month: rust_decimal::Decimal,
    pub recurring_12_month: rust_decimal::Decimal,
    pub recurring_24_month: rust_decimal::Decimal,
    pub recurring_36_month: rust_decimal::Decimal,
}

