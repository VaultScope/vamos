use axum::extract::{Path, State};
use axum::routing::get;
use axum::{Json, Router};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use uuid::Uuid;

use crate::auth::AuthAdmin;
use crate::error::ApiError;
use crate::state::AppState;
use crate::validation::{
    validate_percentage, validate_positive, validate_required_string, validate_string_length,
    MAX_CODE_LENGTH,
};
use vaultscope_db::models::{Coupon, CouponDiscountType, CouponStatus};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/", get(list).post(create))
        .route("/{id}", get(get_one).put(update).delete(delete))
}

async fn list(
    _auth: AuthAdmin,
    State(state): State<AppState>,
) -> Result<Json<Vec<Coupon>>, ApiError> {
    let coupons: Vec<Coupon> = sqlx::query_as(
        "SELECT * FROM coupons ORDER BY created_at DESC"
    )
    .fetch_all(&state.db)
    .await?;
    Ok(Json(coupons))
}

async fn get_one(
    _auth: AuthAdmin,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<Coupon>, ApiError> {
    let coupon: Coupon = sqlx::query_as("SELECT * FROM coupons WHERE id = $1")
        .bind(id)
        .fetch_optional(&state.db)
        .await?
        .ok_or(ApiError::NotFound("coupon not found".into()))?;
    Ok(Json(coupon))
}

#[derive(Deserialize)]
struct CreateCoupon {
    code: String,
    discount_type: CouponDiscountType,
    discount_value: f64,
    usage_limit: Option<i32>,
    expires_at: Option<DateTime<Utc>>,
}

async fn create(
    _auth: AuthAdmin,
    State(state): State<AppState>,
    Json(payload): Json<CreateCoupon>,
) -> Result<Json<Coupon>, ApiError> {
    // Validate inputs
    validate_required_string(&payload.code, "Code")?;
    validate_string_length(&payload.code, "Code", MAX_CODE_LENGTH)?;

    validate_positive(payload.discount_value, "Discount value")?;
    match payload.discount_type {
        CouponDiscountType::Percentage => {
            validate_percentage(payload.discount_value, "Discount percentage")?;
        }
        CouponDiscountType::Fixed => {
            if payload.discount_value == 0.0 {
                return Err(ApiError::BadRequest("Fixed discount must be greater than zero".into()));
            }
        }
    }

    if let Some(limit) = payload.usage_limit {
        if limit <= 0 {
            return Err(ApiError::BadRequest("Usage limit must be greater than zero".into()));
        }
    }

    let coupon: Coupon = sqlx::query_as(
        r#"INSERT INTO coupons (code, discount_type, discount_value, usage_limit, expires_at)
           VALUES ($1, $2::coupon_discount_type, $3, $4, $5)
           RETURNING *"#
    )
    .bind(&payload.code.to_uppercase())
    .bind(payload.discount_type)
    .bind(rust_decimal::Decimal::from_f64_retain(payload.discount_value).unwrap_or_default())
    .bind(payload.usage_limit)
    .bind(payload.expires_at)
    .fetch_one(&state.db)
    .await?;

    Ok(Json(coupon))
}

#[derive(Deserialize)]
struct UpdateCoupon {
    discount_value: Option<f64>,
    usage_limit: Option<i32>,
    expires_at: Option<DateTime<Utc>>,
    status: Option<CouponStatus>,
}

async fn update(
    _auth: AuthAdmin,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(payload): Json<UpdateCoupon>,
) -> Result<Json<Coupon>, ApiError> {
    let coupon: Coupon = sqlx::query_as(
        r#"UPDATE coupons
           SET discount_value = COALESCE($1, discount_value),
               usage_limit = COALESCE($2, usage_limit),
               expires_at = COALESCE($3, expires_at),
               status = COALESCE($4::coupon_status, status),
               updated_at = now()
           WHERE id = $5
           RETURNING *"#
    )
    .bind(payload.discount_value.map(|v| rust_decimal::Decimal::from_f64_retain(v).unwrap_or_default()))
    .bind(payload.usage_limit)
    .bind(payload.expires_at)
    .bind(payload.status)
    .bind(id)
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound("coupon not found".into()))?;

    Ok(Json(coupon))
}

async fn delete(
    _auth: AuthAdmin,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<()>, ApiError> {
    let result = sqlx::query("DELETE FROM coupons WHERE id = $1")
        .bind(id)
        .execute(&state.db)
        .await?;

    if result.rows_affected() == 0 {
        return Err(ApiError::NotFound("coupon not found".into()));
    }

    Ok(Json(()))
}
