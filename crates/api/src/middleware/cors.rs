use axum::http::{header, HeaderName, HeaderValue, Method, Uri};
use tower_http::cors::CorsLayer;

pub fn layer(origins: &[String]) -> CorsLayer {
    let origins: Vec<HeaderValue> = origins
        .iter()
        .filter(|&o| {
            if o == "*" {
                tracing::warn!("Ignoring wildcard '*' CORS origin");
                false
            } else {
                true
            }
        })
        .filter_map(|o| {
            let valid_uri = o
                .parse::<Uri>()
                .ok()
                .is_some_and(|u| matches!(u.scheme_str(), Some("http" | "https")) && u.authority().is_some());
            if !valid_uri {
                tracing::warn!(origin = %o, "Ignoring malformed CORS origin URL");
                return None;
            }
            match o.parse::<HeaderValue>() {
                Ok(hv) => Some(hv),
                Err(e) => {
                    tracing::warn!(origin = %o, error = %e, "Ignoring invalid CORS origin header value");
                    None
                }
            }
        })
        .collect();

    CorsLayer::new()
        .allow_origin(origins)
        .allow_methods([
            Method::GET,
            Method::POST,
            Method::PUT,
            Method::DELETE,
            Method::PATCH,
        ])
        .allow_headers([
            header::AUTHORIZATION,
            header::CONTENT_TYPE,
            header::ACCEPT,
            HeaderName::from_static("x-csrf-token"),
        ])
        .allow_credentials(true)
}
