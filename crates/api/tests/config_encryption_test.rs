mod common;

use common::test_config;
use vaultscope_common::crypto::{decrypt, encrypt};

#[test]
fn test_empty_encryption_key_produces_all_zeros() {
    let mut config = test_config();
    config.encryption_key = String::new();
    let key = config.encryption_key_bytes();
    assert_eq!(key, [0u8; 32]);
}

#[test]
fn test_exactly_32_byte_key_no_padding() {
    let mut config = test_config();
    config.encryption_key = "abcdefghijklmnopqrstuvwxyz012345".to_string();
    assert_eq!(config.encryption_key.len(), 32);
    let key = config.encryption_key_bytes();
    assert_eq!(&key[..], config.encryption_key.as_bytes());
}

#[test]
fn test_derived_key_can_encrypt_decrypt() {
    let config = test_config();
    let key = config.encryption_key_bytes();
    let plaintext = b"sensitive customer data";
    let encrypted = encrypt(&key, plaintext).expect("encrypt should succeed");
    let decrypted = decrypt(&key, &encrypted).expect("decrypt should succeed");
    assert_eq!(decrypted, plaintext);
}

#[test]
fn test_unicode_key_uses_raw_bytes() {
    let mut config = test_config();
    // "ä" is 2 bytes in UTF-8, so "ä" repeated fills more bytes than chars
    config.encryption_key = "ääääääääääääääää".to_string(); // 16 chars = 32 bytes in UTF-8
    let key = config.encryption_key_bytes();
    assert_eq!(key.len(), 32);
    // Should be able to encrypt/decrypt with this key
    let encrypted = encrypt(&key, b"test").unwrap();
    let decrypted = decrypt(&key, &encrypted).unwrap();
    assert_eq!(decrypted, b"test");
}

#[test]
fn test_key_bytes_are_deterministic() {
    let config = test_config();
    let key1 = config.encryption_key_bytes();
    let key2 = config.encryption_key_bytes();
    assert_eq!(key1, key2);
}
