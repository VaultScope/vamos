use axum::{extract::Request, http::StatusCode, middleware::Next, response::Response};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

#[derive(Clone)]
pub struct RateLimiter {
    requests: Arc<Mutex<HashMap<String, (u32, Instant)>>>,
    max_requests: u32,
    window: Duration,
}

impl RateLimiter {
    pub fn new(max_requests: u32, window_secs: u64) -> Self {
        Self {
            requests: Arc::new(Mutex::new(HashMap::new())),
            max_requests,
            window: Duration::from_secs(window_secs),
        }
    }

    pub fn check(&self, ip: &str) -> bool {
        let mut cache = self.requests.lock().unwrap();
        let now = Instant::now();

        // Cleanup old entries occasionally (simple approach)
        if cache.len() > 10_000 {
            cache.retain(|_, &mut (_, start)| now.duration_since(start) <= self.window);
        }

        let (count, start) = cache.entry(ip.to_string()).or_insert((0, now));

        if now.duration_since(*start) > self.window {
            *start = now;
            *count = 1;
            true
        } else if *count < self.max_requests {
            *count += 1;
            true
        } else {
            false
        }
    }
}

pub async fn rate_limit_middleware(
    axum::extract::Extension(limiter): axum::extract::Extension<RateLimiter>,
    req: Request,
    next: Next,
) -> Response {
    let ip = req
        .headers()
        .get("cf-connecting-ip")
        .or_else(|| req.headers().get("x-forwarded-for"))
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.split(',').next())
        .map(|s| s.trim())
        .unwrap_or("unknown");

    if !limiter.check(ip) {
        return axum::response::IntoResponse::into_response((
            StatusCode::TOO_MANY_REQUESTS,
            axum::Json(serde_json::json!({
                "error": "Too many requests. Please wait a moment and try again."
            })),
        ));
    }

    next.run(req).await
}
