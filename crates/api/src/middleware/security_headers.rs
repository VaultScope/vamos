use axum::{
    extract::Request,
    http::header::{
        HeaderName, HeaderValue, CONTENT_SECURITY_POLICY, REFERRER_POLICY, X_CONTENT_TYPE_OPTIONS,
        X_FRAME_OPTIONS,
    },
    middleware::Next,
    response::Response,
};

static PERMISSIONS_POLICY: HeaderName = HeaderName::from_static("permissions-policy");
static X_XSS_PROTECTION: HeaderName = HeaderName::from_static("x-xss-protection");

pub async fn security_headers_middleware(req: Request, next: Next) -> Response {
    let path = req.uri().path().to_string();
    let mut res = next.run(req).await;
    let headers = res.headers_mut();

    headers.insert(X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
    headers.insert(X_FRAME_OPTIONS, HeaderValue::from_static("DENY"));
    headers.insert(
        REFERRER_POLICY,
        HeaderValue::from_static("strict-origin-when-cross-origin"),
    );
    headers.insert(
        PERMISSIONS_POLICY.clone(),
        HeaderValue::from_static("camera=(), microphone=(), geolocation=()"),
    );
    headers.insert(X_XSS_PROTECTION.clone(), HeaderValue::from_static("0"));

    let is_docs = path == "/api/docs" || path.starts_with("/api/docs/");
    let is_uploads = path == "/uploads"
        || path.starts_with("/uploads/")
        || path == "/api/uploads"
        || path.starts_with("/api/uploads/");

    if is_uploads {
        headers.insert(
            CONTENT_SECURITY_POLICY,
            HeaderValue::from_static("default-src 'none'; frame-ancestors 'none'; sandbox"),
        );
    } else if !is_docs {
        headers.insert(
            CONTENT_SECURITY_POLICY,
            HeaderValue::from_static("default-src 'none'; frame-ancestors 'none'"),
        );
    }

    res
}
