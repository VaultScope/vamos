use axum::http::{Method, Request, StatusCode};
use axum::middleware::Next;
use axum::response::Response;
use axum_csrf::CsrfToken;

pub async fn csrf_middleware(
    csrf: CsrfToken,
    req: Request<axum::body::Body>,
    next: Next,
) -> Result<Response, StatusCode> {
    let method = req.method().clone();

    if method == Method::POST
        || method == Method::PUT
        || method == Method::DELETE
        || method == Method::PATCH
    {
        let has_bearer = req
            .headers()
            .get(axum::http::header::AUTHORIZATION)
            .and_then(|h| h.to_str().ok())
            .map(|v| v.starts_with("Bearer ") && v.len() > 7)
            .unwrap_or(false);

        if !has_bearer {
            let token = req
                .headers()
                .get("x-csrf-token")
                .and_then(|h| h.to_str().ok())
                .unwrap_or_default();

            let trusted_origin = req
                .headers()
                .get(axum::http::header::ORIGIN)
                .or_else(|| req.headers().get(axum::http::header::REFERER))
                .and_then(|h| h.to_str().ok())
                .map(|o| {
                    o.contains("vaultscope.de")
                        || o.contains("localhost")
                        || o.contains("127.0.0.1")
                })
                .unwrap_or(false);

            if csrf.verify(token).is_err() && !(trusted_origin && !token.is_empty()) {
                return Err(StatusCode::FORBIDDEN);
            }
        }
    }

    Ok(next.run(req).await)
}
