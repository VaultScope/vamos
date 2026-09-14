use aes_gcm::{
    aead::{Aead, KeyInit},
    Aes256Gcm, Key, Nonce,
};
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

pub fn encrypt(key: &[u8; 32], plaintext: &[u8]) -> Result<Vec<u8>, CryptoError> {
    let key_arr = Key::<Aes256Gcm>::try_from(key.as_slice()).expect("valid key length");
    let cipher = Aes256Gcm::new(&key_arr);

    let mut nonce_bytes = [0u8; 12];
    use rand::Rng;
    rand::rng().fill_bytes(&mut nonce_bytes);
    let nonce = Nonce::try_from(nonce_bytes.as_slice()).expect("valid nonce length");

    let ciphertext = cipher
        .encrypt(&nonce, plaintext)
        .map_err(|e| CryptoError::Encrypt(e.to_string()))?;

    let mut result = Vec::with_capacity(nonce.len() + ciphertext.len());
    result.extend_from_slice(nonce.as_slice());
    result.extend_from_slice(&ciphertext);
    Ok(result)
}

pub fn decrypt(key: &[u8; 32], ciphertext: &[u8]) -> Result<Vec<u8>, CryptoError> {
    if ciphertext.len() < 12 {
        return Err(CryptoError::Decrypt("ciphertext too short".into()));
    }
    let (nonce_bytes, encrypted) = ciphertext.split_at(12);
    let key_arr = Key::<Aes256Gcm>::try_from(key.as_slice()).expect("valid key length");
    let cipher = Aes256Gcm::new(&key_arr);
    let nonce = Nonce::try_from(nonce_bytes).expect("valid nonce length");

    cipher
        .decrypt(&nonce, encrypted)
        .map_err(|e| CryptoError::Decrypt(e.to_string()))
}
