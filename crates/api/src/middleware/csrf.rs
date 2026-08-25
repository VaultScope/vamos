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
    
    if method == Method::POST || method == Method::PUT || method == Method::DELETE || method == Method::PATCH {
        let token = req
            .headers()
            .get("x-csrf-token")
            .and_then(|h| h.to_str().ok())
            .unwrap_or_default();
            
        if csrf.verify(token).is_err() {
            return Err(StatusCode::FORBIDDEN);
        }
    }
    
    Ok(next.run(req).await)
}
