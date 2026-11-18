pub mod admin;
pub mod auth;
pub mod health;
pub mod storefront;
pub mod webhooks;

use axum::{Json, Router};
use axum_csrf::CsrfToken;
use serde_json::{json, Value};
use utoipa::OpenApi;
use utoipa_axum::{router::OpenApiRouter, routes};
use utoipa_scalar::{Scalar, Servable};

use crate::middleware::csrf::csrf_middleware;
use crate::middleware::rate_limit::{rate_limit_middleware, RateLimiter};
use crate::openapi::CsrfTokenResponse;
use crate::state::AppState;

#[utoipa::path(
    get,
    path = "/api/csrf",
    tag = "Auth",
    responses(
        (status = 200, description = "CSRF authenticity token and cookie", body = CsrfTokenResponse)
    )
)]
pub(crate) async fn csrf_token(token: CsrfToken) -> (CsrfToken, Json<Value>) {
    (
        token.clone(),
        Json(json!({ "token": token.authenticity_token().unwrap_or_default() })),
    )
}

pub fn build_router(state: AppState) -> Router {
    let admin_rate_limiter = RateLimiter::new(2400, 60); // 2400 requests per 60 seconds
    let auth_rate_limiter = RateLimiter::new(300, 60); // 300 requests per 60 seconds for auth/me, 2fa, passkeys, login

    let protected = OpenApiRouter::new()
        .nest(
            "/admin",
            admin::routes()
                .layer(axum::middleware::from_fn(rate_limit_middleware))
                .layer(axum::extract::Extension(admin_rate_limiter)),
        )
        .nest("/storefront", storefront::routes())
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            csrf_middleware,
        ));

    let (router, api) = OpenApiRouter::with_openapi(crate::openapi::ApiDoc::openapi())
        .routes(routes!(csrf_token))
        .nest("/api/health", health::routes())
        .nest(
            "/api/auth",
            auth::routes(&state)
                .layer(axum::middleware::from_fn(rate_limit_middleware))
                .layer(axum::extract::Extension(auth_rate_limiter)),
        )
        .nest("/api/webhooks", webhooks::routes())
        .nest("/api", protected)
        .split_for_parts();

    let spec = api.clone();
    let uploads_path = crate::media::get_uploads_root();
    router
        .route(
            "/api/openapi.json",
            axum::routing::get(move || {
                let spec = spec.clone();
                async move { Json(spec) }
            }),
        )
        .merge(Scalar::with_url("/api/docs", api))
        .nest_service("/uploads", tower_http::services::ServeDir::new(&uploads_path))
        .nest_service("/api/uploads", tower_http::services::ServeDir::new(&uploads_path))
        .with_state(state)
}
