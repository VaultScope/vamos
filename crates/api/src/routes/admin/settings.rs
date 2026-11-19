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

    for entry in entries {
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

    tx.commit().await?;

    Ok(Json(json!({ "ok": true })))
}
