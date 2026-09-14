use axum::extract::State;
use axum::routing::get;
use axum::{Json, Router};
use serde::Serialize;
use serde_json::Value;

use crate::error::ApiError;
use crate::state::AppState;
use vaultscope_db::models::Setting;

#[derive(Serialize)]
pub struct StorefrontConfig {
    pub stripe_public_key: Option<String>,
    pub custom_domain: Option<String>,
}

pub fn routes() -> Router<AppState> {
    Router::new().route("/", get(get_config))
}

async fn get_config(State(state): State<AppState>) -> Result<Json<StorefrontConfig>, ApiError> {
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

    Ok(Json(StorefrontConfig {
        stripe_public_key: public_key,
        custom_domain,
    }))
}
