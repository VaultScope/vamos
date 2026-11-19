pub mod admin;
pub mod auth;
pub mod health;
pub mod storefront;
pub mod webhooks;

use axum::{
    body::Body,
    extract::{Path, State},
    http::{
        header::{CACHE_CONTROL, CONTENT_DISPOSITION, CONTENT_TYPE, X_CONTENT_TYPE_OPTIONS},
        HeaderMap, HeaderValue, StatusCode,
    },
    response::Response,
    Json, Router,
};
use axum_csrf::CsrfToken;
use serde_json::{json, Value};
use utoipa::OpenApi;
use utoipa_axum::{router::OpenApiRouter, routes};
use utoipa_scalar::{Scalar, Servable};

use crate::auth::{authenticate_any_bearer, Audience, AuthAdmin};
use crate::error::ApiError;
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

fn is_safe_ticket_filename(filename: &str) -> bool {
    !filename.is_empty()
        && filename.len() <= 128
        && !filename.starts_with('.')
        && !filename.contains("..")
        && filename
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_' || b == b'.')
}

fn attachment_content_type(filename: &str) -> &'static str {
    let lower = filename.to_ascii_lowercase();
    if lower.ends_with(".webp") {
        "image/webp"
    } else if lower.ends_with(".png") {
        "image/png"
    } else if lower.ends_with(".jpg") || lower.ends_with(".jpeg") {
        "image/jpeg"
    } else {
        "application/octet-stream"
    }
}

pub(crate) async fn download_ticket_attachment(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(filename): Path<String>,
) -> Result<Response, ApiError> {
    if !is_safe_ticket_filename(&filename) {
        return Err(ApiError::BadRequest("Invalid attachment filename".into()));
    }

    let claims = authenticate_any_bearer(&headers, &state).await?;
    match claims.audience {
        Audience::Admin => {
            let admin = AuthAdmin(claims);
            crate::middleware::rbac::check_permission(&admin, &state, "tickets.view").await?;
        }
        Audience::Storefront => {
            let legacy_url = format!("/uploads/tickets/{}", filename);
            let api_url = format!("/api/uploads/tickets/{}", filename);
            let owns_attachment: bool = sqlx::query_scalar(
                r#"SELECT EXISTS (
                       SELECT 1
                       FROM ticket_messages tm
                       JOIN tickets t ON t.id = tm.ticket_id
                       WHERE t.customer_id = $1
                         AND tm.internal = false
                         AND EXISTS (
                             SELECT 1
                             FROM jsonb_array_elements(
                                 CASE
                                     WHEN jsonb_typeof(tm.attachments) = 'array' THEN tm.attachments
                                     ELSE '[]'::jsonb
                                 END
                             ) AS att
                             WHERE att->>'url' IN ($2, $3)
                         )
                   )"#,
            )
            .bind(claims.sub)
            .bind(&legacy_url)
            .bind(&api_url)
            .fetch_one(&state.db)
            .await?;

            if !owns_attachment {
                return Err(ApiError::Forbidden);
            }
        }
    }

    let file_path = crate::media::get_uploads_root()
        .join("tickets")
        .join(&filename);
    let bytes = match tokio::fs::read(&file_path).await {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err(ApiError::NotFound("Attachment not found".into()));
        }
        Err(e) => {
            return Err(ApiError::Internal(format!(
                "Failed to read ticket attachment: {}",
                e
            )));
        }
    };

    let content_type = attachment_content_type(&filename);
    let disposition = format!("attachment; filename=\"{}\"", filename);

    let mut response = Response::new(Body::from(bytes));
    *response.status_mut() = StatusCode::OK;
    let headers = response.headers_mut();
    headers.insert(CONTENT_TYPE, HeaderValue::from_static(content_type));
    if let Ok(disp_val) = HeaderValue::from_str(&disposition) {
        headers.insert(CONTENT_DISPOSITION, disp_val);
    }
    headers.insert(CACHE_CONTROL, HeaderValue::from_static("private, no-store"));
    headers.insert(X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));

    Ok(response)
}

pub fn build_router(state: AppState) -> Router {
    let admin_rate_limiter = RateLimiter::new(2400, 60); // 2400 requests per 60 seconds
    let storefront_rate_limiter = RateLimiter::new(120, 60); // 120 requests per 60 seconds
    let auth_rate_limiter = RateLimiter::new(120, 60); // 120 requests per 60 seconds (strict auth routes apply 20/min)

    let protected = OpenApiRouter::new()
        .nest(
            "/admin",
            admin::routes()
                .layer(axum::middleware::from_fn(rate_limit_middleware))
                .layer(axum::extract::Extension(admin_rate_limiter)),
        )
        .nest(
            "/storefront",
            storefront::routes()
                .layer(axum::middleware::from_fn(rate_limit_middleware))
                .layer(axum::extract::Extension(storefront_rate_limiter)),
        )
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

    let mut router = router;
    if state.config.dev_mode {
        let spec = api.clone();
        router = router
            .route(
                "/api/openapi.json",
                axum::routing::get(move || {
                    let spec = spec.clone();
                    async move { Json(spec) }
                }),
            )
            .merge(Scalar::with_url("/api/docs", api));
    }

    let uploads_path = crate::media::get_uploads_root();
    let mut app = router.route(
        "/api/uploads/tickets/{filename}",
        axum::routing::get(download_ticket_attachment),
    );

    for public_dir in ["blog", "media", "avatars", "products"] {
        let dir_path = uploads_path.join(public_dir);
        app = app
            .nest_service(
                &format!("/uploads/{}", public_dir),
                tower_http::services::ServeDir::new(&dir_path),
            )
            .nest_service(
                &format!("/api/uploads/{}", public_dir),
                tower_http::services::ServeDir::new(&dir_path),
            );
    }

    app.with_state(state)
}
