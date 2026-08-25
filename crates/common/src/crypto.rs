use thiserror::Error;

#[derive(Debug, Error)]
pub enum CryptoError {
    #[error("encryption failed: {0}")]
    Encrypt(String),
    #[error("decryption failed: {0}")]
    Decrypt(String),
    #[error("invalid key length")]
    InvalidKey,
}

/// Encrypt plaintext bytes with AES-256-GCM.
/// In production, use the `aes-gcm` crate. This is a placeholder interface.
pub fn encrypt(_key: &[u8; 32], plaintext: &[u8]) -> Result<Vec<u8>, CryptoError> {
    // TODO: implement with aes-gcm crate
    Ok(plaintext.to_vec())
}

/// Decrypt ciphertext bytes with AES-256-GCM.
pub fn decrypt(_key: &[u8; 32], ciphertext: &[u8]) -> Result<Vec<u8>, CryptoError> {
    // TODO: implement with aes-gcm crate
    Ok(ciphertext.to_vec())
}
