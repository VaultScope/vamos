use axum::extract::{Path, State};
use axum::Json;
use serde::Serialize;
use utoipa::ToSchema;
use utoipa_axum::{router::OpenApiRouter, routes};

use crate::error::ApiError;
use crate::openapi::ErrorResponse;
use crate::state::AppState;
use vaultscope_db::models::{Partner, Product};

pub fn routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new().routes(routes!(get_by_slug))
}

#[derive(Serialize, ToSchema)]
pub struct PartnerResponse {
    pub partner: PartnerInfo,
    pub product: Product,
}

#[derive(Serialize, ToSchema)]
pub struct PartnerInfo {
    pub id: uuid::Uuid,
    pub slug: String,
    pub name: String,
    pub headline: Option<String>,
    pub description: Option<String>,
    pub badge_text: Option<String>,
    pub trial_days: Option<i32>,
    pub trial_quota_gb: Option<i32>,
    pub quota_gb: Option<i32>,
    pub limit_one_per_user: bool,
}

#[utoipa::path(
    get,
    path = "/{slug}",
    tag = "Storefront Partners",
    params(
        ("slug" = String, Path, description = "Partner campaign slug")
    ),
    responses(
        (status = 200, description = "Partner campaign metadata and discounted/trial product", body = PartnerResponse),
        (status = 404, description = "Partner campaign not found", body = ErrorResponse)
    )
)]
pub(crate) async fn get_by_slug(
    State(state): State<AppState>,
    Path(slug): Path<String>,
) -> Result<Json<PartnerResponse>, ApiError> {
    let clean_slug = slug.trim().to_lowercase();

    // 1. Fetch partner
    let partner: Option<Partner> = sqlx::query_as(
        "SELECT * FROM partners WHERE LOWER(slug) = LOWER($1) AND active = true"
    )
    .bind(&clean_slug)
    .fetch_optional(&state.db)
    .await?;

    let partner = partner.ok_or_else(|| ApiError::NotFound("partner campaign not found".into()))?;

    // 2. Fetch linked visible product before counting click
    let mut product: Product = sqlx::query_as(
        "SELECT * FROM products WHERE id = $1 AND hidden = false"
    )
    .bind(partner.product_id)
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| ApiError::NotFound("campaign product not available".into()))?;

    // 3. Increment clicks
    let _ = sqlx::query("UPDATE partners SET clicks = clicks + 1 WHERE id = $1")
        .bind(partner.id)
        .execute(&state.db)
        .await;

    // 4. If partner has custom trial overrides, apply them
    let effective_trial_days = partner.trial_days_override.or(product.trial_days);
    let effective_trial_quota = partner.trial_quota_gb_override.or(product.trial_quota_gb);
    let effective_quota = partner.quota_gb_override.or(product.quota_gb);

    product.trial_days = effective_trial_days;
    product.trial_quota_gb = effective_trial_quota;
    product.quota_gb = effective_quota;

    let policy = crate::reliablesite_catalog::load_pricing_policy(&state.db, &state.config).await;
    let product =
        crate::reliablesite_catalog::sanitize_product_for_storefront_with_policy(product, policy);

    Ok(Json(PartnerResponse {
        partner: PartnerInfo {
            id: partner.id,
            slug: partner.slug,
            name: partner.name,
            headline: partner.headline,
            description: partner.description,
            badge_text: partner.badge_text,
            trial_days: effective_trial_days,
            trial_quota_gb: effective_trial_quota,
            quota_gb: effective_quota,
            limit_one_per_user: partner.limit_one_per_user,
        },
        product,
    }))
}
