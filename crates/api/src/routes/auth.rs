use axum::extract::State;
use axum::routing::post;
use axum::{Json, Router};
use chrono::Utc;
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::auth::{jwt, oidc, Audience, Claims};
use crate::error::ApiError;
use crate::state::AppState;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/callback/admin", post(admin_callback))
        .route("/callback/storefront", post(storefront_callback))
}

#[derive(Deserialize)]
struct CallbackPayload {
    code: String,
    redirect_uri: String,
}

async fn admin_callback(
    State(state): State<AppState>,
    Json(payload): Json<CallbackPayload>,
) -> Result<Json<Value>, ApiError> {
    let tokens = oidc::exchange_code(&state.config, &payload.code, &payload.redirect_uri, true).await?;
    let userinfo = oidc::fetch_userinfo(&state.config, &tokens.access_token).await?;

    tracing::info!("admin login: sub={}, email={}", userinfo.sub, userinfo.email);

    let staff: Option<(Uuid, String)> = sqlx::query_as(
        "SELECT s.id, r.name FROM staff s JOIN roles r ON s.role_id = r.id WHERE s.external_id = $1"
    )
    .bind(&userinfo.sub)
    .fetch_optional(&state.db)
    .await
    .map_err(ApiError::from)?;

    let (staff_id, role_name) = staff.ok_or(ApiError::Forbidden)?;

    sqlx::query("UPDATE staff SET last_login = now() WHERE id = $1")
        .bind(staff_id)
        .execute(&state.db)
        .await?;

    let claims = Claims {
        sub: staff_id,
        email: userinfo.email,
        role: role_name,
        audience: Audience::Admin,
        iat: Utc::now().timestamp(),
        exp: jwt::new_expiry(),
    };

    let token = jwt::sign(&claims, &state.config.jwt_secret)?;
    Ok(Json(json!({ "token": token, "claims": claims })))
}

async fn storefront_callback(
    State(state): State<AppState>,
    Json(payload): Json<CallbackPayload>,
) -> Result<Json<Value>, ApiError> {
    let tokens = oidc::exchange_code(&state.config, &payload.code, &payload.redirect_uri, false).await?;
    let userinfo = oidc::fetch_userinfo(&state.config, &tokens.access_token).await?;

    let customer_id: Option<(Uuid,)> = sqlx::query_as(
        "SELECT id FROM customers WHERE external_id = $1"
    )
    .bind(&userinfo.sub)
    .fetch_optional(&state.db)
    .await?;

    let customer_id = match customer_id {
        Some((id,)) => {
            sqlx::query("UPDATE customers SET last_login = now() WHERE id = $1")
                .bind(id)
                .execute(&state.db)
                .await?;
            id
        }
        None => {
            let id = Uuid::now_v7();
            sqlx::query(
                "INSERT INTO customers (id, external_id, name, email) VALUES ($1, $2, $3, $4)"
            )
            .bind(id)
            .bind(&userinfo.sub)
            .bind(userinfo.preferred_username.as_deref().unwrap_or(&userinfo.email))
            .bind(&userinfo.email)
            .execute(&state.db)
            .await?;
            id
        }
    };

    let claims = Claims {
        sub: customer_id,
        email: userinfo.email,
        role: "customer".to_string(),
        audience: Audience::Storefront,
        iat: Utc::now().timestamp(),
        exp: jwt::new_expiry(),
    };

    let token = jwt::sign(&claims, &state.config.jwt_secret)?;
    Ok(Json(json!({ "token": token, "claims": claims })))
}
