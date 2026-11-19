use axum::http::{Method, Request, StatusCode};
use axum::middleware::Next;
use axum::response::Response;
use axum_csrf::CsrfToken;

/// Validates that a Bearer token is a non-empty three-segment JWT-like string.
pub fn is_jwt_like(token: &str) -> bool {
    let mut parts = token.split('.');
    match (parts.next(), parts.next(), parts.next(), parts.next()) {
        (Some(header), Some(payload), Some(sig), None) => {
            !header.is_empty()
                && !payload.is_empty()
                && !sig.is_empty()
                && [header, payload, sig].iter().all(|seg| {
                    seg.bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_' || b == b'=')
                })
        }
        _ => false,
    }
}

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
            .and_then(|v| v.strip_prefix("Bearer "))
            .map(str::trim)
            .map(is_jwt_like)
            .unwrap_or(false);

        if !has_bearer {
            let token = req
                .headers()
                .get("x-csrf-token")
                .and_then(|h| h.to_str().ok())
                .map(str::trim)
                .unwrap_or_default();

            if token.is_empty() || csrf.verify(token).is_err() {
                return Err(StatusCode::FORBIDDEN);
            }
        }
    }

    Ok(next.run(req).await)
}
