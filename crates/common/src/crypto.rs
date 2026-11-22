use aes_gcm::{
    aead::{Aead, KeyInit},
    Aes256Gcm, Key, Nonce,
};
use rand::Rng;
use thiserror::Error;

pub const ENCRYPTED_SECRET_PREFIX: &str = "enc:v1:";
const ALPHANUMERIC_BYTES: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";

#[derive(Debug, Error)]
pub enum CryptoError {
    #[error("encryption failed: {0}")]
    Encrypt(String),
    #[error("decryption failed: {0}")]
    Decrypt(String),
    #[error("invalid key length")]
    InvalidKey,
}

/// Generates a cryptographically secure random alphanumeric (`A-Za-z0-9`) string
/// of length `len` using `rand::rng()` with unbiased rejection sampling.
pub fn generate_random_secret(len: usize) -> String {
    if len == 0 {
        return String::new();
    }

    let mut out = String::with_capacity(len);
    let mut rng = rand::rng();
    let mut buf = [0u8; 64];
    // Largest multiple of 62 <= 256 is 248 (62 * 4), avoiding modulo bias.
    const UNBIASED_LIMIT: u8 = 248;

    while out.len() < len {
        rng.fill_bytes(&mut buf);
        for &b in &buf {
            if b < UNBIASED_LIMIT {
                let idx = (b % 62) as usize;
                out.push(ALPHANUMERIC_BYTES[idx] as char);
                if out.len() == len {
                    break;
                }
            }
        }
    }

    out
}

fn encode_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0x0f) as usize] as char);
    }
    out
}

fn decode_hex(s: &str) -> Result<Vec<u8>, CryptoError> {
    if !s.len().is_multiple_of(2) {
        return Err(CryptoError::Decrypt("invalid hex length".into()));
    }
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len() / 2);
    for chunk in bytes.chunks_exact(2) {
        let hi = hex_nibble(chunk[0])?;
        let lo = hex_nibble(chunk[1])?;
        out.push((hi << 4) | lo);
    }
    Ok(out)
}

fn hex_nibble(b: u8) -> Result<u8, CryptoError> {
    match b {
        b'0'..=b'9' => Ok(b - b'0'),
        b'a'..=b'f' => Ok(b - b'a' + 10),
        b'A'..=b'F' => Ok(b - b'A' + 10),
        _ => Err(CryptoError::Decrypt("invalid hex character".into())),
    }
}

/// Encrypts a UTF-8 secret string with AES-256-GCM and encodes it as `enc:v1:<hex>`.
pub fn encrypt_secret_string(key: &[u8; 32], plaintext: &str) -> Result<String, CryptoError> {
    let encrypted = encrypt(key, plaintext.as_bytes())?;
    Ok(format!("{ENCRYPTED_SECRET_PREFIX}{}", encode_hex(&encrypted)))
}

/// Decrypts an `enc:v1:<hex>` (or raw hex-encoded AES-256-GCM ciphertext) secret string.
pub fn decrypt_secret_string(key: &[u8; 32], stored: &str) -> Result<String, CryptoError> {
    let hex_ct = stored.strip_prefix(ENCRYPTED_SECRET_PREFIX).unwrap_or(stored);
    let bytes = decode_hex(hex_ct)?;
    let plaintext = decrypt(key, &bytes)?;
    String::from_utf8(plaintext)
        .map_err(|e| CryptoError::Decrypt(format!("invalid UTF-8 plaintext: {e}")))
}

pub fn encrypt(key: &[u8; 32], plaintext: &[u8]) -> Result<Vec<u8>, CryptoError> {
    let key_arr = Key::<Aes256Gcm>::try_from(key.as_slice()).expect("valid key length");
    let cipher = Aes256Gcm::new(&key_arr);

    let mut nonce_bytes = [0u8; 12];
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_random_secret_properties() {
        assert_eq!(generate_random_secret(0), "");

        let s1 = generate_random_secret(24);
        let s2 = generate_random_secret(24);
        assert_eq!(s1.len(), 24);
        assert_eq!(s2.len(), 24);
        assert!(s1.bytes().all(|b| b.is_ascii_alphanumeric()));
        assert!(s2.bytes().all(|b| b.is_ascii_alphanumeric()));
        assert_ne!(s1, s2);
    }

    #[test]
    fn test_encrypt_decrypt_secret_string_roundtrip() {
        let key = [7u8; 32];
        let secret = "S3cr3t!P@ssw0rd";
        let encoded = encrypt_secret_string(&key, secret).unwrap();
        assert!(encoded.starts_with(ENCRYPTED_SECRET_PREFIX));
        assert!(!encoded.contains(secret));

        let decoded = decrypt_secret_string(&key, &encoded).unwrap();
        assert_eq!(decoded, secret);

        // Wrong key and malformed inputs must fail closed
        assert!(decrypt_secret_string(&[8u8; 32], &encoded).is_err());
        assert!(decrypt_secret_string(&key, "not-valid-hex").is_err());
    }
}
