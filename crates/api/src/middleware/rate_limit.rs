use axum::{
    extract::{ConnectInfo, Request},
    http::{HeaderMap, StatusCode},
    middleware::Next,
    response::Response,
};
use std::{
    collections::{HashMap, VecDeque},
    net::{IpAddr, SocketAddr},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

pub const MAX_ENTRIES: usize = 50_000;

struct RateLimiterState {
    entries: HashMap<String, (u32, Instant)>,
    order: VecDeque<String>,
}

impl RateLimiterState {
    fn prune_expired(&mut self, now: Instant, window: Duration) {
        self.entries
            .retain(|_, &mut (_, start)| !window.is_zero() && now.duration_since(start) <= window);
        if self.entries.len() < self.order.len() {
            let entries = &self.entries;
            self.order.retain(|k| entries.contains_key(k));
        }
    }
}

#[derive(Clone)]
pub struct RateLimiter {
    state: Arc<Mutex<RateLimiterState>>,
    max_requests: u32,
    window: Duration,
}

impl RateLimiter {
    pub fn new(max_requests: u32, window_secs: u64) -> Self {
        Self {
            state: Arc::new(Mutex::new(RateLimiterState {
                entries: HashMap::new(),
                order: VecDeque::new(),
            })),
            max_requests,
            window: Duration::from_secs(window_secs),
        }
    }

    #[allow(dead_code)]
    pub fn entry_count(&self) -> usize {
        let cache = self.state.lock().unwrap_or_else(|e| e.into_inner());
        cache.entries.len()
    }

    pub fn check(&self, ip: &str) -> bool {
        let mut cache = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let now = Instant::now();

        if self.window.is_zero() {
            // 0-second windows always expire immediately.
            cache.entries.clear();
            cache.order.clear();
            return self.max_requests > 0;
        }

        // Cleanup old expired entries when cache grows large.
        if cache.entries.len() > 10_000 {
            cache.prune_expired(now, self.window);
        }

        if let Some((count, start)) = cache.entries.get_mut(ip) {
            if now.duration_since(*start) > self.window {
                *start = now;
                *count = 1;
                return self.max_requests > 0;
            } else if *count < self.max_requests {
                *count += 1;
                return true;
            } else {
                return false;
            }
        }

        if self.max_requests == 0 {
            return false;
        }

        // Enforce hard entry cap to prevent unbounded memory growth under spoofed/distributed floods.
        if cache.entries.len() >= MAX_ENTRIES {
            cache.prune_expired(now, self.window);
            while cache.entries.len() >= MAX_ENTRIES {
                if let Some(oldest_key) = cache.order.pop_front() {
                    cache.entries.remove(&oldest_key);
                } else {
                    break;
                }
            }
        }

        let key = ip.to_string();
        cache.entries.insert(key.clone(), (1, now));
        cache.order.push_back(key);
        true
    }
}

#[derive(Clone, Copy, Debug)]
pub enum IpCidr {
    V4 { net: u32, mask: u32 },
    V6 { net: u128, mask: u128 },
}

impl IpCidr {
    pub fn parse(raw: &str) -> Option<Self> {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return None;
        }
        if let Some((ip_part, prefix_part)) = trimmed.split_once('/') {
            let ip: IpAddr = ip_part.trim().parse().ok()?;
            let prefix: u32 = prefix_part.trim().parse().ok()?;
            match normalize_ip(ip) {
                IpAddr::V4(v4) if prefix <= 32 => {
                    let mask = if prefix == 0 { 0 } else { !0u32 << (32 - prefix) };
                    Some(Self::V4 {
                        net: u32::from(v4) & mask,
                        mask,
                    })
                }
                IpAddr::V6(v6) if prefix <= 128 => {
                    let mask = if prefix == 0 {
                        0
                    } else {
                        !0u128 << (128 - prefix)
                    };
                    Some(Self::V6 {
                        net: u128::from(v6) & mask,
                        mask,
                    })
                }
                _ => None,
            }
        } else {
            let ip: IpAddr = trimmed.parse().ok()?;
            match normalize_ip(ip) {
                IpAddr::V4(v4) => Some(Self::V4 {
                    net: u32::from(v4),
                    mask: !0u32,
                }),
                IpAddr::V6(v6) => Some(Self::V6 {
                    net: u128::from(v6),
                    mask: !0u128,
                }),
            }
        }
    }

    pub fn contains(&self, ip: IpAddr) -> bool {
        match (self, normalize_ip(ip)) {
            (Self::V4 { net, mask }, IpAddr::V4(v4)) => (u32::from(v4) & *mask) == *net,
            (Self::V6 { net, mask }, IpAddr::V6(v6)) => (u128::from(v6) & *mask) == *net,
            _ => false,
        }
    }
}

pub fn normalize_ip(ip: IpAddr) -> IpAddr {
    match ip {
        IpAddr::V6(v6) => match v6.to_ipv4_mapped() {
            Some(v4) => IpAddr::V4(v4),
            None => IpAddr::V6(v6),
        },
        other => other,
    }
}

pub fn parse_trusted_proxy_cidrs(raw: &str) -> Vec<IpCidr> {
    raw.split(',')
        .filter_map(|part| {
            let trimmed = part.trim();
            if trimmed.is_empty() {
                return None;
            }
            let parsed = IpCidr::parse(trimmed);
            if parsed.is_none() {
                tracing::warn!(cidr = %trimmed, "Ignoring invalid entry in TRUSTED_PROXY_CIDRS");
            }
            parsed
        })
        .collect()
}

pub fn is_trusted_proxy(ip: IpAddr, extra_cidrs: &[IpCidr]) -> bool {
    let norm = normalize_ip(ip);
    if norm.is_loopback() {
        return true;
    }
    match norm {
        IpAddr::V4(v4) if v4.is_private() => return true,
        IpAddr::V6(v6) if (v6.segments()[0] & 0xfe00) == 0xfc00 => return true,
        _ => {}
    }
    extra_cidrs.iter().any(|cidr| cidr.contains(norm))
}

/// Resolves the client IP address from the peer TCP address and proxy headers.
/// Forwarded headers (`CF-Connecting-IP`, `X-Forwarded-For`) are trusted ONLY
/// when the direct TCP peer is loopback, a private proxy hop, or within `extra_cidrs`.
pub fn resolve_client_ip_with_cidrs(
    headers: &HeaderMap,
    peer_ip: Option<IpAddr>,
    extra_cidrs: &[IpCidr],
) -> String {
    let Some(peer) = peer_ip.map(normalize_ip) else {
        // Without a verified TCP peer (e.g. oneshot unit test without ConnectInfo),
        // never trust spoofable forwarding headers.
        return "unknown".to_string();
    };

    if !is_trusted_proxy(peer, extra_cidrs) {
        return peer.to_string();
    }

    let trust_cf = std::env::var("TRUST_CLOUDFLARE_HEADERS")
        .map(|v| !matches!(v.trim().to_ascii_lowercase().as_str(), "false" | "0" | "no"))
        .unwrap_or(true);

    if trust_cf {
        if let Some(cf_ip) = headers
            .get("cf-connecting-ip")
            .and_then(|v| v.to_str().ok())
            .map(str::trim)
            .and_then(|s| s.parse::<IpAddr>().ok())
        {
            return normalize_ip(cf_ip).to_string();
        }
    }

    let hops: Vec<IpAddr> = headers
        .get_all("x-forwarded-for")
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|xff| xff.split(','))
        .filter_map(|s| s.trim().parse::<IpAddr>().ok())
        .map(normalize_ip)
        .collect();

    if !hops.is_empty() {
        if let Some(client) = hops.iter().rev().find(|ip| !is_trusted_proxy(**ip, extra_cidrs)) {
            return client.to_string();
        }
        if let Some(first) = hops.first() {
            return first.to_string();
        }
    }

    peer.to_string()
}

pub fn extract_trusted_client_ip(headers: &HeaderMap, peer_ip: Option<IpAddr>) -> Option<String> {
    peer_ip?;
    let cidrs = std::env::var("TRUSTED_PROXY_CIDRS")
        .map(|raw| parse_trusted_proxy_cidrs(&raw))
        .unwrap_or_default();
    let ip = resolve_client_ip_with_cidrs(headers, peer_ip, &cidrs);
    if ip == "unknown" {
        None
    } else {
        Some(ip)
    }
}

pub async fn rate_limit_middleware(
    axum::extract::Extension(limiter): axum::extract::Extension<RateLimiter>,
    req: Request,
    next: Next,
) -> Response {
    let peer_ip = req
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|ci| ci.0.ip());

    let extra_cidrs = std::env::var("TRUSTED_PROXY_CIDRS")
        .map(|raw| parse_trusted_proxy_cidrs(&raw))
        .unwrap_or_default();

    let ip = resolve_client_ip_with_cidrs(req.headers(), peer_ip, &extra_cidrs);

    if !limiter.check(&ip) {
        return axum::response::IntoResponse::into_response((
            StatusCode::TOO_MANY_REQUESTS,
            axum::Json(serde_json::json!({
                "error": "Too many requests. Please wait a moment and try again."
            })),
        ));
    }

    next.run(req).await
}
