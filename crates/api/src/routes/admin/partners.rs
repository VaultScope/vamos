use axum::extract::{Path, State};
use axum::routing::get;
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::auth::AuthAdmin;
use crate::error::ApiError;
use crate::state::AppState;
use vaultscope_db::models::Partner;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/", get(list).post(create))
        .route("/{id}", get(get_one).put(update).delete(delete))
}

#[derive(Serialize, sqlx::FromRow)]
pub struct PartnerWithProduct {
    pub id: Uuid,
    pub slug: String,
    pub name: String,
    pub headline: Option<String>,
    pub description: Option<String>,
    pub badge_text: Option<String>,
    pub product_id: Uuid,
    pub product_name: String,
    pub product_price: rust_decimal::Decimal,
    pub product_trial_days: Option<i32>,
    pub trial_days_override: Option<i32>,
    pub trial_quota_gb_override: Option<i32>,
    pub quota_gb_override: Option<i32>,
    pub product_trial_quota_gb: Option<i32>,
    pub product_quota_gb: Option<i32>,
    pub limit_one_per_user: bool,
    pub active: bool,
    pub clicks: i32,
    pub conversions: i32,
    pub commission_type: Option<String>,
    pub commission_rate: Option<rust_decimal::Decimal>,
    pub commission_fixed_eur: Option<rust_decimal::Decimal>,
    pub per_sale_commission_eur: rust_decimal::Decimal,
    pub earned_commission_eur: rust_decimal::Decimal,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

async fn list(
    auth: AuthAdmin,
    State(state): State<AppState>,
) -> Result<Json<Vec<PartnerWithProduct>>, ApiError> {
    crate::require_permission!(auth, state, "products.view");

    let partners = sqlx::query_as::<_, PartnerWithProduct>(
        r#"
        SELECT 
            p.id, p.slug, p.name, p.headline, p.description, p.badge_text,
            p.product_id, prod.name as product_name, prod.price as product_price,
            prod.trial_days as product_trial_days, p.trial_days_override,
            p.trial_quota_gb_override, p.quota_gb_override,
            prod.trial_quota_gb as product_trial_quota_gb, prod.quota_gb as product_quota_gb,
            p.limit_one_per_user, p.active, p.clicks, p.conversions,
            p.commission_type, p.commission_rate, p.commission_fixed_eur,
            CASE 
                WHEN p.commission_type = 'percent' AND p.commission_rate IS NOT NULL THEN
                    ROUND(prod.price * (p.commission_rate / 100.0), 2)
                WHEN p.commission_type = 'fixed' AND p.commission_fixed_eur IS NOT NULL THEN
                    p.commission_fixed_eur
                ELSE 0.00
            END AS per_sale_commission_eur,
            CASE 
                WHEN p.commission_type = 'percent' AND p.commission_rate IS NOT NULL THEN
                    ROUND(prod.price * (p.commission_rate / 100.0) * p.conversions, 2)
                WHEN p.commission_type = 'fixed' AND p.commission_fixed_eur IS NOT NULL THEN
                    ROUND(p.commission_fixed_eur * p.conversions, 2)
                ELSE 0.00
            END AS earned_commission_eur,
            p.created_at, p.updated_at
        FROM partners p
        JOIN products prod ON p.product_id = prod.id
        ORDER BY p.created_at DESC
        "#,
    )
    .fetch_all(&state.db)
    .await?;

    Ok(Json(partners))
}

async fn get_one(
    auth: AuthAdmin,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<Partner>, ApiError> {
    crate::require_permission!(auth, state, "products.view");
    let partner: Partner = sqlx::query_as("SELECT * FROM partners WHERE id = $1")
        .bind(id)
        .fetch_optional(&state.db)
        .await?
        .ok_or(ApiError::NotFound("partner not found".into()))?;
    Ok(Json(partner))
}

#[derive(Deserialize)]
pub struct CreatePartner {
    pub slug: String,
    pub name: String,
    pub headline: Option<String>,
    pub description: Option<String>,
    pub badge_text: Option<String>,
    pub product_id: Uuid,
    pub trial_days_override: Option<i32>,
    pub trial_quota_gb_override: Option<i32>,
    pub quota_gb_override: Option<i32>,
    pub limit_one_per_user: Option<bool>,
    pub active: Option<bool>,
    pub commission_type: Option<String>,
    pub commission_rate: Option<rust_decimal::Decimal>,
    pub commission_fixed_eur: Option<rust_decimal::Decimal>,
}

async fn create(
    auth: AuthAdmin,
    State(state): State<AppState>,
    Json(payload): Json<CreatePartner>,
) -> Result<Json<Partner>, ApiError> {
    crate::require_permission!(auth, state, "products.edit");

    let slug = payload.slug.trim().to_lowercase().replace(' ', "-");
    let id = Uuid::now_v7();

    let partner: Partner = sqlx::query_as(
        r#"
        INSERT INTO partners (
            id, slug, name, headline, description, badge_text, product_id,
            trial_days_override, trial_quota_gb_override, quota_gb_override,
            limit_one_per_user, active,
            commission_type, commission_rate, commission_fixed_eur
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15)
        RETURNING *
        "#
    )
    .bind(id)
    .bind(slug)
    .bind(&payload.name)
    .bind(payload.headline)
    .bind(payload.description)
    .bind(payload.badge_text)
    .bind(payload.product_id)
    .bind(payload.trial_days_override)
    .bind(payload.trial_quota_gb_override)
    .bind(payload.quota_gb_override)
    .bind(payload.limit_one_per_user.unwrap_or(true))
    .bind(payload.active.unwrap_or(true))
    .bind(payload.commission_type.as_deref().unwrap_or("none"))
    .bind(payload.commission_rate)
    .bind(payload.commission_fixed_eur)
    .fetch_one(&state.db)
    .await?;

    Ok(Json(partner))
}

async fn update(
    auth: AuthAdmin,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(payload): Json<CreatePartner>,
) -> Result<Json<Partner>, ApiError> {
    crate::require_permission!(auth, state, "products.edit");

    let slug = payload.slug.trim().to_lowercase().replace(' ', "-");

    let partner: Partner = sqlx::query_as(
        r#"
        UPDATE partners
        SET slug = $1, name = $2, headline = $3, description = $4, badge_text = $5,
            product_id = $6, trial_days_override = $7, trial_quota_gb_override = $8,
            quota_gb_override = $9, limit_one_per_user = $10, active = $11,
            commission_type = $12, commission_rate = $13, commission_fixed_eur = $14,
            updated_at = now()
        WHERE id = $15
        RETURNING *
        "#
    )
    .bind(slug)
    .bind(&payload.name)
    .bind(payload.headline)
    .bind(payload.description)
    .bind(payload.badge_text)
    .bind(payload.product_id)
    .bind(payload.trial_days_override)
    .bind(payload.trial_quota_gb_override)
    .bind(payload.quota_gb_override)
    .bind(payload.limit_one_per_user.unwrap_or(true))
    .bind(payload.active.unwrap_or(true))
    .bind(payload.commission_type.as_deref().unwrap_or("none"))
    .bind(payload.commission_rate)
    .bind(payload.commission_fixed_eur)
    .bind(id)
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound("partner not found".into()))?;

    Ok(Json(partner))
}

async fn delete(
    auth: AuthAdmin,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, ApiError> {
    crate::require_permission!(auth, state, "products.edit");

    let rows = sqlx::query("DELETE FROM partners WHERE id = $1")
        .bind(id)
        .execute(&state.db)
        .await?
        .rows_affected();

    if rows == 0 {
        return Err(ApiError::NotFound("partner not found".into()));
    }

    Ok(Json(serde_json::json!({ "deleted": true })))
}
