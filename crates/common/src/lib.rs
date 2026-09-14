pub mod crypto;
pub mod types;

#[cfg(test)]
mod tests {
    use super::crypto::*;

    #[test]
    fn test_encrypt_decrypt() {
        let key = [42u8; 32];
        let plaintext = b"Hello, World!";

        let encrypted = encrypt(&key, plaintext).expect("Encryption failed");
        assert_ne!(
            &encrypted[12..],
            plaintext,
            "Ciphertext should differ from plaintext"
        );

        let decrypted = decrypt(&key, &encrypted).expect("Decryption failed");
        assert_eq!(decrypted, plaintext, "Decrypted text should match original");
    }

    #[test]
    fn test_decrypt_wrong_key() {
        let key1 = [42u8; 32];
        let key2 = [43u8; 32];
        let plaintext = b"Secret message";

        let encrypted = encrypt(&key1, plaintext).expect("Encryption failed");
        let result = decrypt(&key2, &encrypted);

        assert!(result.is_err(), "Decryption should fail with wrong key");
    }

    #[test]
    fn test_decrypt_truncated() {
        let key = [42u8; 32];
        let plaintext = b"Test";

        let encrypted = encrypt(&key, plaintext).expect("Encryption failed");
        let truncated = &encrypted[..5];

        let result = decrypt(&key, truncated);
        assert!(result.is_err(), "Should fail with truncated ciphertext");
    }
}
