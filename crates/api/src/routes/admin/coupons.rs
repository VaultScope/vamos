use axum::extract::{Path, State};
use axum::Json;
use chrono::{DateTime, Utc};
use serde::Deserialize;
use utoipa::ToSchema;
use utoipa_axum::{router::OpenApiRouter, routes};
use uuid::Uuid;

use crate::auth::AuthAdmin;
use crate::error::ApiError;
use crate::openapi::ErrorResponse;
use crate::state::AppState;
use crate::validation::{
    validate_percentage, validate_positive, validate_required_string, validate_string_length,
    MAX_CODE_LENGTH,
};
use serde_json::Value;
use vaultscope_db::models::{Coupon, CouponDiscountType, CouponStatus};

pub fn routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(list, create))
        .routes(routes!(get_one, update, delete))
}

#[utoipa::path(
    get,
    path = "/",
    tag = "Admin Coupons",
    security(("bearer_auth" = [])),
    responses((status = 200, description = "List coupons", body = Vec<Coupon>))
)]
pub(crate) async fn list(
    auth: AuthAdmin,
    State(state): State<AppState>,
) -> Result<Json<Vec<Coupon>>, ApiError> {
    crate::require_permission!(auth, state, "coupons.manage");
    let coupons: Vec<Coupon> = sqlx::query_as("SELECT * FROM coupons ORDER BY created_at DESC")
        .fetch_all(&state.db)
        .await?;
    Ok(Json(coupons))
}

#[utoipa::path(
    get,
    path = "/{id}",
    tag = "Admin Coupons",
    security(("bearer_auth" = [])),
    params(("id" = Uuid, Path, description = "Coupon ID")),
    responses(
        (status = 200, description = "Coupon details", body = Coupon),
        (status = 404, description = "Coupon not found", body = ErrorResponse)
    )
)]
pub(crate) async fn get_one(
    auth: AuthAdmin,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<Coupon>, ApiError> {
    crate::require_permission!(auth, state, "coupons.manage");
    let coupon: Coupon = sqlx::query_as("SELECT * FROM coupons WHERE id = $1")
        .bind(id)
        .fetch_optional(&state.db)
        .await?
        .ok_or(ApiError::NotFound("coupon not found".into()))?;
    Ok(Json(coupon))
}

#[derive(Deserialize, ToSchema)]
pub(crate) struct CreateCoupon {
    code: String,
    discount_type: CouponDiscountType,
    discount_value: f64,
    usage_limit: Option<i32>,
    expires_at: Option<DateTime<Utc>>,
}

async fn get_stripe_key(state: &AppState) -> Result<String, ApiError> {
    let is_test_mode_val = sqlx::query_scalar::<_, Value>(
        "SELECT value FROM settings WHERE key = 'stripe_test_mode_enabled'",
    )
    .fetch_optional(&state.db)
    .await?;

    let is_test_mode = is_test_mode_val.and_then(|v| v.as_bool()).unwrap_or(true);

    let key_name = if is_test_mode {
        "stripe_sandbox_secret_key"
    } else {
        "stripe_secret_key"
    };

    let stripe_key_val =
        sqlx::query_scalar::<_, Value>("SELECT value FROM settings WHERE key = $1")
            .bind(key_name)
            .fetch_optional(&state.db)
            .await?;

    let stripe_key = stripe_key_val
        .and_then(|v| v.as_str().map(|s| s.to_string()))
        .ok_or_else(|| ApiError::Internal("Stripe key not configured".into()))?;

    Ok(stripe_key)
}

pub fn validate_coupon_code(raw: &str) -> Result<String, ApiError> {
    validate_required_string(raw, "Code")?;
    let trimmed = raw.trim();
    validate_string_length(trimmed, "Code", MAX_CODE_LENGTH)?;
    if !trimmed
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
    {
        return Err(ApiError::BadRequest(
            "Coupon code may only contain ASCII letters, digits, underscores, and hyphens".into(),
        ));
    }
    Ok(trimmed.to_uppercase())
}

#[utoipa::path(
    post,
    path = "/",
    tag = "Admin Coupons",
    security(("bearer_auth" = []), ("csrf_token" = [])),
    request_body = CreateCoupon,
    responses(
        (status = 200, description = "Coupon created", body = Coupon),
        (status = 400, description = "Invalid input", body = ErrorResponse)
    )
)]
pub(crate) async fn create(
    auth: AuthAdmin,
    State(state): State<AppState>,
    Json(payload): Json<CreateCoupon>,
) -> Result<Json<Coupon>, ApiError> {
    crate::require_permission!(auth, state, "coupons.manage");
    // Validate inputs
    let uppercase_code = validate_coupon_code(&payload.code)?;

    validate_positive(payload.discount_value, "Discount value")?;
    match payload.discount_type {
        CouponDiscountType::Percentage => {
            validate_percentage(payload.discount_value, "Discount percentage")?;
        }
        CouponDiscountType::Fixed => {
            if payload.discount_value == 0.0 {
                return Err(ApiError::BadRequest(
                    "Fixed discount must be greater than zero".into(),
                ));
            }
        }
    }

    if let Some(limit) = payload.usage_limit {
        if limit <= 0 {
            return Err(ApiError::BadRequest(
                "Usage limit must be greater than zero".into(),
            ));
        }
    }

    // Create in Stripe first
    let stripe_key = get_stripe_key(&state).await?;
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(|e| {
            tracing::error!("Failed to build Stripe HTTP client: {}", e);
            ApiError::Internal("Payment provider unavailable".into())
        })?;

    let mut form = vec![("id", uppercase_code.clone())];

    let dv_string;
    if payload.discount_type == CouponDiscountType::Percentage {
        dv_string = payload.discount_value.to_string();
        form.push(("percent_off", dv_string.clone()));
    } else {
        // Stripe fixed amount is in euro cents
        dv_string = (payload.discount_value * 100.0).round().to_string();
        form.push(("amount_off", dv_string.clone()));
        form.push(("currency", "eur".to_string()));
    }

    let ul_string;
    if let Some(limit) = payload.usage_limit {
        ul_string = limit.to_string();
        form.push(("max_redemptions", ul_string.clone()));
    }

    let exp_string;
    if let Some(exp) = payload.expires_at {
        exp_string = exp.timestamp().to_string();
        form.push(("redeem_by", exp_string.clone()));
    }

    let res = client
        .post("https://api.stripe.com/v1/coupons")
        .basic_auth(&stripe_key, Some(""))
        .form(&form)
        .send()
        .await
        .map_err(|e| {
            tracing::error!("Stripe coupon create request error: {}", e);
            ApiError::Internal("Failed to communicate with payment provider".into())
        })?;

    if !res.status().is_success() {
        let status = res.status();
        let err_text = res.text().await.unwrap_or_default();
        tracing::error!(%status, stripe_body = %err_text, "Stripe coupon creation failed");
        return Err(ApiError::Internal("Stripe coupon creation failed".into()));
    }

    let discount_decimal = rust_decimal::Decimal::try_from(payload.discount_value)
        .map(|d| d.round_dp(2))
        .unwrap_or_default();

    let coupon: Coupon = sqlx::query_as(
        r#"INSERT INTO coupons (code, discount_type, discount_value, usage_limit, expires_at)
           VALUES ($1, $2::coupon_discount_type, $3, $4, $5)
           RETURNING *"#,
    )
    .bind(&uppercase_code)
    .bind(payload.discount_type)
    .bind(discount_decimal)
    .bind(payload.usage_limit)
    .bind(payload.expires_at)
    .fetch_one(&state.db)
    .await?;

    Ok(Json(coupon))
}

#[derive(Deserialize, ToSchema)]
pub(crate) struct UpdateCoupon {
    discount_value: Option<f64>,
    usage_limit: Option<i32>,
    expires_at: Option<DateTime<Utc>>,
    status: Option<CouponStatus>,
}

#[utoipa::path(
    put,
    path = "/{id}",
    tag = "Admin Coupons",
    security(("bearer_auth" = []), ("csrf_token" = [])),
    params(("id" = Uuid, Path, description = "Coupon ID")),
    request_body = UpdateCoupon,
    responses(
        (status = 200, description = "Coupon updated", body = Coupon),
        (status = 404, description = "Coupon not found", body = ErrorResponse)
    )
)]
pub(crate) async fn update(
    auth: AuthAdmin,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(payload): Json<UpdateCoupon>,
) -> Result<Json<Coupon>, ApiError> {
    crate::require_permission!(auth, state, "coupons.manage");

    // fetch existing coupon code
    let existing_coupon: Option<Coupon> = sqlx::query_as("SELECT * FROM coupons WHERE id = $1")
        .bind(id)
        .fetch_optional(&state.db)
        .await?;

    let existing = existing_coupon.ok_or(ApiError::NotFound("coupon not found".into()))?;
    let safe_code = validate_coupon_code(&existing.code)?;

    if let Some(val) = payload.discount_value {
        validate_positive(val, "Discount value")?;
        if existing.discount_type == CouponDiscountType::Percentage {
            validate_percentage(val, "Discount percentage")?;
        }
    }
    if let Some(limit) = payload.usage_limit {
        if limit <= 0 {
            return Err(ApiError::BadRequest(
                "Usage limit must be greater than zero".into(),
            ));
        }
    }

    // Update in Stripe
    let stripe_key = get_stripe_key(&state).await?;
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(|e| {
            tracing::error!("Failed to build Stripe HTTP client: {}", e);
            ApiError::Internal("Payment provider unavailable".into())
        })?;

    let stripe_payload: std::collections::HashMap<&str, &str> = std::collections::HashMap::new();

    let res = client
        .post(format!(
            "https://api.stripe.com/v1/coupons/{}",
            safe_code
        ))
        .basic_auth(&stripe_key, Some(""))
        .form(&stripe_payload)
        .send()
        .await
        .map_err(|e| {
            tracing::error!("Stripe coupon update request error: {}", e);
            ApiError::Internal("Failed to communicate with payment provider".into())
        })?;

    if !res.status().is_success() {
        let status = res.status();
        let err_text = res.text().await.unwrap_or_default();
        tracing::error!(%status, stripe_body = %err_text, "Stripe coupon update failed");
        return Err(ApiError::Internal("Stripe coupon update failed".into()));
    }

    let coupon: Coupon = sqlx::query_as(
        r#"UPDATE coupons
           SET discount_value = COALESCE($1, discount_value),
               usage_limit = COALESCE($2, usage_limit),
               expires_at = COALESCE($3, expires_at),
               status = COALESCE($4::coupon_status, status),
               updated_at = now()
           WHERE id = $5
           RETURNING *"#,
    )
    .bind(
        payload
            .discount_value
            .and_then(|v| rust_decimal::Decimal::try_from(v).ok())
            .map(|d| d.round_dp(2)),
    )
    .bind(payload.usage_limit)
    .bind(payload.expires_at)
    .bind(payload.status)
    .bind(id)
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound("coupon not found".into()))?;

    Ok(Json(coupon))
}

#[utoipa::path(
    delete,
    path = "/{id}",
    tag = "Admin Coupons",
    security(("bearer_auth" = []), ("csrf_token" = [])),
    params(("id" = Uuid, Path, description = "Coupon ID")),
    responses(
        (status = 200, description = "Coupon deleted"),
        (status = 404, description = "Coupon not found", body = ErrorResponse)
    )
)]
pub(crate) async fn delete(
    auth: AuthAdmin,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<()>, ApiError> {
    crate::require_permission!(auth, state, "coupons.manage");

    let existing_coupon: Option<Coupon> = sqlx::query_as("SELECT * FROM coupons WHERE id = $1")
        .bind(id)
        .fetch_optional(&state.db)
        .await?;

    let result = sqlx::query("DELETE FROM coupons WHERE id = $1")
        .bind(id)
        .execute(&state.db)
        .await?;

    if result.rows_affected() == 0 {
        return Err(ApiError::NotFound("coupon not found".into()));
    }

    // Also delete in stripe
    if let Some(coupon) = existing_coupon {
        if let Ok(safe_code) = validate_coupon_code(&coupon.code) {
            if let Ok(stripe_key) = get_stripe_key(&state).await {
                if let Ok(client) = reqwest::Client::builder()
                    .timeout(std::time::Duration::from_secs(15))
                    .build()
                {
                    let _ = client
                        .delete(format!("https://api.stripe.com/v1/coupons/{}", safe_code))
                        .basic_auth(&stripe_key, Some(""))
                        .send()
                        .await;
                }
            }
        }
    }

    Ok(Json(()))
}
