use axum::extract::State;
use axum::Json;
use serde::Serialize;
use serde_json::Value;
use utoipa::ToSchema;
use utoipa_axum::{router::OpenApiRouter, routes};

use crate::error::ApiError;
use crate::openapi::ErrorResponse;
use crate::state::AppState;
use vaultscope_db::models::Setting;

#[derive(Serialize, ToSchema)]
pub struct StorefrontConfig {
    pub stripe_public_key: Option<String>,
    pub custom_domain: Option<String>,
    pub tax_mode: String,
    pub tax_rate_percent: rust_decimal::Decimal,
    pub small_business_exempt: bool,
    pub tax_notice: Option<String>,
}

pub fn routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new().routes(routes!(get_config))
}

#[utoipa::path(
    get,
    path = "/",
    tag = "Storefront Config",
    responses(
        (status = 200, description = "Public storefront configuration", body = StorefrontConfig),
        (status = 500, description = "Internal error", body = ErrorResponse)
    )
)]
pub(crate) async fn get_config(State(state): State<AppState>) -> Result<Json<StorefrontConfig>, ApiError> {
    let settings: Vec<Setting> = sqlx::query_as(
        "SELECT * FROM settings WHERE key IN ('stripe_public_key', 'stripe_sandbox_public_key', 'stripe_test_mode_enabled', 'custom_domain')"
    )
    .fetch_all(&state.db)
    .await?;

    let mut stripe_public_key = None;
    let mut stripe_sandbox_public_key = None;
    let mut stripe_test_mode_enabled = false;
    let mut custom_domain = None;

    for s in settings {
        match s.key.as_str() {
            "stripe_public_key" => {
                if let Value::String(v) = s.value {
                    stripe_public_key = Some(v);
                }
            }
            "stripe_sandbox_public_key" => {
                if let Value::String(v) = s.value {
                    stripe_sandbox_public_key = Some(v);
                }
            }
            "stripe_test_mode_enabled" => {
                if let Value::Bool(v) = s.value {
                    stripe_test_mode_enabled = v;
                } else if let Value::String(v) = s.value {
                    stripe_test_mode_enabled = v == "true";
                }
            }
            "custom_domain" => {
                if let Value::String(v) = s.value {
                    custom_domain = Some(v);
                }
            }
            _ => {}
        }
    }

    let public_key = if stripe_test_mode_enabled {
        stripe_sandbox_public_key
    } else {
        stripe_public_key
    };

    let tax_policy = crate::reliablesite_catalog::load_tax_policy(&state.db, &state.config).await;

    Ok(Json(StorefrontConfig {
        stripe_public_key: public_key,
        custom_domain,
        tax_mode: tax_policy.tax_mode,
        tax_rate_percent: tax_policy.rate_percent,
        small_business_exempt: tax_policy.small_business_exempt,
        tax_notice: tax_policy.tax_notice,
    }))
}
