use axum::extract::State;
use axum::routing::get;
use axum::{Json, Router};
use serde::Deserialize;

use crate::auth::AuthCustomer;
use crate::error::ApiError;
use crate::state::AppState;
use vaultscope_db::models::Customer;

pub fn routes() -> Router<AppState> {
    Router::new().route("/", get(get_profile).put(update_profile))
}

async fn get_profile(
    auth: AuthCustomer,
    State(state): State<AppState>,
) -> Result<Json<ProfileResponse>, ApiError> {
    let customer = sqlx::query_as::<_, Customer>("SELECT * FROM customers WHERE id = $1")
        .bind(auth.0.sub)
        .fetch_optional(&state.db)
        .await?
        .ok_or(ApiError::NotFound("customer not found".into()))?;

    Ok(Json(ProfileResponse {
        id: customer.id,
        name: customer.name,
        email: customer.email,
        phone: customer.phone,
        company: customer.company,
        address: customer.address,
        city: customer.city,
        country: customer.country,
        vat_id: customer.vat_id,
        two_factor_enabled: customer.two_factor_enabled,
        email_verified: customer.email_verified,
        created_at: customer.created_at,
        updated_at: customer.updated_at,
        last_login: customer.last_login,
    }))
}

/// A filtered response that does NOT expose internal admin fields
/// like notes, stripe_customer_id, or status.
#[derive(serde::Serialize)]
struct ProfileResponse {
    id: uuid::Uuid,
    name: String,
    email: String,
    phone: String,
    company: String,
    address: String,
    city: String,
    country: String,
    vat_id: String,
    two_factor_enabled: bool,
    email_verified: bool,
    created_at: chrono::DateTime<chrono::Utc>,
    updated_at: chrono::DateTime<chrono::Utc>,
    last_login: Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(Deserialize)]
struct UpdateProfile {
    name: Option<String>,
    phone: Option<String>,
    company: Option<String>,
    address: Option<String>,
    city: Option<String>,
    country: Option<String>,
    vat_id: Option<String>,
}

async fn update_profile(
    auth: AuthCustomer,
    State(state): State<AppState>,
    Json(payload): Json<UpdateProfile>,
) -> Result<Json<ProfileResponse>, ApiError> {
    // Validate name if provided
    if let Some(ref name) = payload.name {
        if name.trim().is_empty() {
            return Err(ApiError::BadRequest("Name cannot be empty".into()));
        }
        if name.len() > 255 {
            return Err(ApiError::BadRequest(
                "Name exceeds maximum length of 255 characters".into(),
            ));
        }
    }

    let customer: Customer = sqlx::query_as(
        r#"UPDATE customers
           SET name = COALESCE($1, name),
               phone = COALESCE($2, phone),
               company = COALESCE($3, company),
               address = COALESCE($4, address),
               city = COALESCE($5, city),
               country = COALESCE($6, country),
               vat_id = COALESCE($7, vat_id),
               updated_at = now()
           WHERE id = $8
           RETURNING *"#,
    )
    .bind(payload.name)
    .bind(payload.phone)
    .bind(payload.company)
    .bind(payload.address)
    .bind(payload.city)
    .bind(payload.country)
    .bind(payload.vat_id)
    .bind(auth.0.sub)
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound("customer not found".into()))?;

    Ok(Json(ProfileResponse {
        id: customer.id,
        name: customer.name,
        email: customer.email,
        phone: customer.phone,
        company: customer.company,
        address: customer.address,
        city: customer.city,
        country: customer.country,
        vat_id: customer.vat_id,
        two_factor_enabled: customer.two_factor_enabled,
        email_verified: customer.email_verified,
        created_at: customer.created_at,
        updated_at: customer.updated_at,
        last_login: customer.last_login,
    }))
}
