use vaultscope_api::middleware::rate_limit::RateLimiter;

#[test]
fn test_allows_requests_within_limit() {
    let limiter = RateLimiter::new(5, 60);
    for _ in 0..5 {
        assert!(limiter.check("192.168.1.1"));
    }
}

#[test]
fn test_blocks_after_limit_exceeded() {
    let limiter = RateLimiter::new(3, 60);
    assert!(limiter.check("10.0.0.1"));
    assert!(limiter.check("10.0.0.1"));
    assert!(limiter.check("10.0.0.1"));
    assert!(!limiter.check("10.0.0.1"));
    assert!(!limiter.check("10.0.0.1"));
}

#[test]
fn test_different_ips_have_separate_limits() {
    let limiter = RateLimiter::new(2, 60);
    assert!(limiter.check("1.1.1.1"));
    assert!(limiter.check("1.1.1.1"));
    assert!(!limiter.check("1.1.1.1"));

    assert!(limiter.check("2.2.2.2"));
    assert!(limiter.check("2.2.2.2"));
    assert!(!limiter.check("2.2.2.2"));
}

#[test]
fn test_single_request_limit() {
    let limiter = RateLimiter::new(1, 60);
    assert!(limiter.check("10.0.0.5"));
    assert!(!limiter.check("10.0.0.5"));
}

#[test]
fn test_zero_window_still_works() {
    let limiter = RateLimiter::new(100, 0);
    // With a 0-second window, every check resets (window has already expired)
    for _ in 0..200 {
        assert!(limiter.check("10.0.0.1"));
    }
}

#[test]
fn test_concurrent_access() {
    use std::sync::Arc;
    use std::thread;

    let limiter = Arc::new(RateLimiter::new(100, 60));
    let mut handles = vec![];

    for i in 0..10 {
        let limiter = Arc::clone(&limiter);
        let ip = format!("10.0.0.{}", i);
        handles.push(thread::spawn(move || {
            let mut allowed = 0;
            for _ in 0..150 {
                if limiter.check(&ip) {
                    allowed += 1;
                }
            }
            assert_eq!(allowed, 100, "IP {} should have exactly 100 allowed", ip);
        }));
    }

    for handle in handles {
        handle.join().unwrap();
    }
}
