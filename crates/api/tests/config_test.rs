mod common;

use common::test_config;

#[test]
fn test_config_defaults() {
    let config = test_config();
    assert_eq!(config.host, "127.0.0.1");
    assert_eq!(config.port, 3001);
    assert_eq!(config.db_max_connections, 5);
    assert_eq!(config.job_poll_interval_secs, 5);
    assert!(config.stripe_secret_key.is_none());
    assert_eq!(config.cors_origins, vec!["http://localhost:5173".to_string()]);
}

#[test]
fn test_encryption_key_bytes_exact_32() {
    let config = test_config();
    let key = config.encryption_key_bytes();
    assert_eq!(key.len(), 32);
    assert_eq!(&key[..31], b"test_encryption_key_32_bytes_!!");
}

#[test]
fn test_encryption_key_bytes_short_key_padded() {
    let mut config = test_config();
    config.encryption_key = "short".to_string();
    let key = config.encryption_key_bytes();
    assert_eq!(key.len(), 32);
    assert_eq!(&key[..5], b"short");
    assert_eq!(&key[5..], &[0u8; 27]);
}

#[test]
fn test_encryption_key_bytes_long_key_truncated() {
    let mut config = test_config();
    config.encryption_key = "a".repeat(64);
    let key = config.encryption_key_bytes();
    assert_eq!(key.len(), 32);
    assert!(key.iter().all(|&b| b == b'a'));
}
