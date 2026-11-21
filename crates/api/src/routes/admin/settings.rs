use axum::extract::State;
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};
use utoipa::ToSchema;
use utoipa_axum::{router::OpenApiRouter, routes};

use crate::auth::AuthAdmin;
use crate::error::ApiError;
use crate::openapi::OkResponse;
use crate::state::AppState;
use vaultscope_db::models::Setting;

pub fn routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new().routes(routes!(list, update_batch))
}

const EXPLICIT_SECRET_KEYS: &[&str] = &[
    "smtpPass",
    "smtp_password",
    "oidcClientSecret",
    "stripe_live_secret_key",
    "stripe_secret_key",
    "stripe_webhook_secret",
    "stripe_sandbox_secret_key",
];

/// Returns `true` if `key` holds a sensitive secret or credential.
pub fn is_secret_setting_key(key: &str) -> bool {
    if EXPLICIT_SECRET_KEYS.iter().any(|k| k.eq_ignore_ascii_case(key)) {
        return true;
    }
    let lower = key.to_ascii_lowercase();
    lower.contains("secret")
        || lower.contains("password")
        || lower.contains("pass")
        || lower.contains("token")
        || lower.contains("api_key")
}

/// Returns `true` if a setting value is empty, non-string, or a masked placeholder.
pub fn is_empty_or_masked_secret(value: &Value) -> bool {
    let Some(raw) = value.as_str() else {
        return true;
    };
    let trimmed = raw.trim();
    if trimmed.is_empty()
        || trimmed == "__UNCHANGED__"
        || trimmed == "__CONFIGURED__"
        || trimmed.contains("••••")
    {
        return true;
    }
    trimmed.chars().all(|c| c == '•' || c == '*')
}

/// Masks secret setting values in `GET /admin/settings` responses (`""` + `<key>_configured: bool`)
/// while preserving all non-secret settings verbatim.
pub fn mask_settings_for_response(settings: Vec<Setting>) -> Vec<Setting> {
    let mut out = Vec::with_capacity(settings.len() + 8);
    let mut configured_entries = Vec::new();

    for setting in settings {
        if setting.key.ends_with("_configured") {
            continue;
        }
        if is_secret_setting_key(&setting.key) {
            let is_configured =
                !is_empty_or_masked_secret(&setting.value) || matches!(setting.value, Value::Bool(true));
            configured_entries.push(Setting {
                key: format!("{}_configured", setting.key),
                value: Value::Bool(is_configured),
                updated_at: setting.updated_at,
            });
            out.push(Setting {
                key: setting.key,
                value: Value::String(String::new()),
                updated_at: setting.updated_at,
            });
        } else {
            out.push(setting);
        }
    }

    out.extend(configured_entries);
    out
}

#[utoipa::path(
    get,
    path = "/",
    tag = "Admin Settings",
    security(("bearer_auth" = [])),
    responses((status = 200, description = "List platform settings", body = Vec<Setting>))
)]
pub(crate) async fn list(
    auth: AuthAdmin,
    State(state): State<AppState>,
) -> Result<Json<Vec<Setting>>, ApiError> {
    crate::require_permission!(auth, state, "settings.manage");
    let settings: Vec<Setting> = sqlx::query_as("SELECT * FROM settings ORDER BY key")
        .fetch_all(&state.db)
        .await?;
    Ok(Json(mask_settings_for_response(settings)))
}

#[derive(Deserialize, ToSchema)]
pub(crate) struct SettingEntry {
    key: String,
    value: Value,
}

#[utoipa::path(
    put,
    path = "/",
    tag = "Admin Settings",
    security(("bearer_auth" = []), ("csrf_token" = [])),
    request_body = Vec<SettingEntry>,
    responses((status = 200, description = "Settings updated", body = OkResponse))
)]
pub(crate) async fn update_batch(
    auth: AuthAdmin,
    State(state): State<AppState>,
    Json(entries): Json<Vec<SettingEntry>>,
) -> Result<Json<Value>, ApiError> {
    crate::require_permission!(auth, state, "settings.manage");
    let mut tx = state.db.begin().await?;

    for entry in &entries {
        let key = entry.key.trim();
        if key.is_empty() || key.ends_with("_configured") {
            continue;
        }
        if is_secret_setting_key(key) && is_empty_or_masked_secret(&entry.value) {
            continue;
        }

        sqlx::query(
            "INSERT INTO settings (key, value, updated_at) VALUES ($1, $2, now()) ON CONFLICT (key) DO UPDATE SET value = $2, updated_at = now()"
        )
        .bind(key)
        .bind(&entry.value)
        .execute(&mut *tx)
        .await?;
    }

    // If ReliableSite FX rate, auto-FX toggle, or markup changed, recompute retail EUR prices from wholesale cost
    if entries.iter().any(|e| {
        matches!(
            e.key.trim(),
            "reliablesite_auto_fx_enabled"
                | "reliablesite_usd_to_eur_rate"
                | "reliablesite_markup_percent"
        )
    }) {
        let settings_rows: Vec<Setting> = sqlx::query_as(
            "SELECT * FROM settings WHERE key IN ('reliablesite_auto_fx_enabled', 'reliablesite_usd_to_eur_rate', 'reliablesite_markup_percent')",
        )
        .fetch_all(&mut *tx)
        .await?;

        let mut auto_fx = true;
        let mut fx = state.config.reliablesite_usd_to_eur_rate;
        let mut markup = state.config.reliablesite_markup_percent;
        for row in settings_rows {
            if row.key == "reliablesite_auto_fx_enabled" {
                if let Some(b) = row.value.as_bool() {
                    auto_fx = b;
                }
                continue;
            }
            let parsed = if let Some(s) = row.value.as_str() {
                s.trim().parse::<rust_decimal::Decimal>().ok()
            } else if let Some(n) = row.value.as_f64() {
                rust_decimal::Decimal::try_from(n).ok()
            } else {
                row.value.as_i64().map(rust_decimal::Decimal::from)
            };
            if let Some(d) = parsed {
                if row.key == "reliablesite_usd_to_eur_rate" && d > rust_decimal::Decimal::ZERO {
                    fx = d;
                } else if row.key == "reliablesite_markup_percent" && d >= rust_decimal::Decimal::ZERO {
                    markup = d;
                }
            }
        }

        if state.config.reliablesite_live_fx_enabled && auto_fx {
            if let Some(live_fx) = crate::reliablesite_catalog::resolve_live_usd_to_eur_rate().await {
                fx = live_fx;
            }
        }

        let multiplier = fx * (rust_decimal::Decimal::ONE + (markup / rust_decimal::Decimal::from(100)));
        sqlx::query(
            "UPDATE products SET price = ROUND(cost * $1, 2), updated_at = now() WHERE provider = 'reliablesite' AND cost > 0",
        )
        .bind(multiplier)
        .execute(&mut *tx)
        .await?;
    }

    tx.commit().await?;

    Ok(Json(json!({ "ok": true })))
}
