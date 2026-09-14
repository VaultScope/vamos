use vaultscope_common::crypto::{decrypt, encrypt};

#[test]
fn test_encrypt_empty_plaintext() {
    let key = [1u8; 32];
    let encrypted = encrypt(&key, b"").expect("encrypt empty");
    let decrypted = decrypt(&key, &encrypted).expect("decrypt empty");
    assert!(decrypted.is_empty());
}

#[test]
fn test_encrypt_large_payload() {
    let key = [2u8; 32];
    let plaintext = vec![0xABu8; 1024 * 1024]; // 1 MB
    let encrypted = encrypt(&key, &plaintext).unwrap();
    let decrypted = decrypt(&key, &encrypted).unwrap();
    assert_eq!(decrypted, plaintext);
}

#[test]
fn test_ciphertext_includes_nonce_prefix() {
    let key = [3u8; 32];
    let encrypted = encrypt(&key, b"hello").unwrap();
    // Nonce is 12 bytes, so ciphertext must be at least 12 + len("hello") + GCM tag (16)
    assert!(encrypted.len() >= 12 + 5 + 16);
}

#[test]
fn test_different_encryptions_produce_different_ciphertext() {
    let key = [4u8; 32];
    let plaintext = b"same data";
    let enc1 = encrypt(&key, plaintext).unwrap();
    let enc2 = encrypt(&key, plaintext).unwrap();
    // Random nonce should make ciphertexts differ
    assert_ne!(enc1, enc2);
}

#[test]
fn test_decrypt_nonce_only_no_ciphertext() {
    let key = [5u8; 32];
    // Exactly 12 bytes = nonce only, no actual ciphertext
    let fake = vec![0u8; 12];
    let result = decrypt(&key, &fake);
    assert!(result.is_err());
}

#[test]
fn test_decrypt_tampered_ciphertext() {
    let key = [6u8; 32];
    let mut encrypted = encrypt(&key, b"important data").unwrap();
    // Flip a byte in the ciphertext portion (after the 12-byte nonce)
    if let Some(byte) = encrypted.get_mut(15) {
        *byte ^= 0xFF;
    }
    let result = decrypt(&key, &encrypted);
    assert!(result.is_err(), "Tampered ciphertext should fail GCM auth");
}

#[test]
fn test_decrypt_tampered_nonce() {
    let key = [7u8; 32];
    let mut encrypted = encrypt(&key, b"data").unwrap();
    encrypted[0] ^= 0xFF;
    let result = decrypt(&key, &encrypted);
    assert!(result.is_err(), "Tampered nonce should fail decryption");
}

#[test]
fn test_encrypt_binary_data() {
    let key = [8u8; 32];
    let plaintext: Vec<u8> = (0..=255).collect();
    let encrypted = encrypt(&key, &plaintext).unwrap();
    let decrypted = decrypt(&key, &encrypted).unwrap();
    assert_eq!(decrypted, plaintext);
}
