use vaultscope_common::crypto::{decrypt, encrypt};

#[test]
fn test_encrypt_decrypt_12_byte_plaintext() {
    let key = [10u8; 32];
    let plaintext = b"exactly12byt";
    assert_eq!(plaintext.len(), 12);
    let encrypted = encrypt(&key, plaintext).unwrap();
    let decrypted = decrypt(&key, &encrypted).unwrap();
    assert_eq!(decrypted, plaintext);
}

#[test]
fn test_encrypt_decrypt_1_byte_plaintext() {
    let key = [11u8; 32];
    let plaintext = b"X";
    let encrypted = encrypt(&key, plaintext).unwrap();
    let decrypted = decrypt(&key, &encrypted).unwrap();
    assert_eq!(decrypted, plaintext);
}

#[test]
fn test_decrypt_12_bytes_nonce_only() {
    let key = [12u8; 32];
    // 12 bytes = nonce only, empty ciphertext (GCM needs at least the 16-byte auth tag)
    let data = vec![0u8; 12];
    let result = decrypt(&key, &data);
    assert!(result.is_err(), "12 bytes (nonce only) should fail GCM decryption");
}

#[test]
fn test_decrypt_11_bytes_fails() {
    let key = [13u8; 32];
    let data = vec![0u8; 11];
    let result = decrypt(&key, &data);
    assert!(result.is_err(), "Less than 12 bytes should fail");
}

#[test]
fn test_decrypt_0_bytes_fails() {
    let key = [14u8; 32];
    let result = decrypt(&key, &[]);
    assert!(result.is_err(), "Empty input should fail");
}
