use vaultscope_api::middleware::rate_limit::RateLimiter;

#[test]
fn test_very_large_max_requests() {
    let limiter = RateLimiter::new(u32::MAX - 1, 60);
    // Should allow at least a bunch of requests without blocking
    for _ in 0..1000 {
        assert!(limiter.check("10.0.0.1"));
    }
}

#[test]
fn test_cleanup_trigger_at_10001_entries() {
    // Window=0 means entries expire immediately, so cleanup should remove them all
    let limiter = RateLimiter::new(100, 0);
    // Fill up 10,001 distinct IPs — triggers the cleanup branch (cache.len() > 10_000)
    for i in 0..10_001u32 {
        let ip = format!("10.{}.{}.{}", (i >> 16) & 0xFF, (i >> 8) & 0xFF, i & 0xFF);
        limiter.check(&ip);
    }
    // After cleanup, a new check should still succeed
    assert!(limiter.check("192.168.0.1"));
}

#[test]
fn test_window_expiry_resets_counter() {
    // Window=0 means every check sees the window as expired, resetting the counter
    let limiter = RateLimiter::new(1, 0);
    // Even with max_requests=1, each check resets due to expired window
    for _ in 0..100 {
        assert!(limiter.check("10.0.0.1"));
    }
}

#[test]
fn test_max_requests_one_blocks_on_second() {
    // With a non-zero window, max_requests=1 should block the second call
    let limiter = RateLimiter::new(1, 3600);
    assert!(limiter.check("10.0.0.1"));
    assert!(!limiter.check("10.0.0.1"), "Second request should be blocked");
}
