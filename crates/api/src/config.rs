use std::env;

#[derive(Debug, Clone)]
pub struct Config {
    pub database_url: String,
    pub db_max_connections: u32,
    pub host: String,
    pub port: u16,
    pub jwt_secret: String,
    pub encryption_key: String,
    pub authentik_issuer: String,
    pub authentik_client_id_admin: String,
    pub authentik_client_id_storefront: String,
    pub authentik_client_secret_admin: String,
    pub authentik_client_secret_storefront: String,
    pub authentik_api_token: Option<String>,
    pub stripe_secret_key: Option<String>,
    pub cors_origins: Vec<String>,
    pub job_poll_interval_secs: u64,
    pub redis_url: String,
}

impl Config {
    pub fn from_env() -> Self {
        let encryption_key = required("ENCRYPTION_KEY");
        validate_encryption_key(&encryption_key);

        let jwt_secret = required("JWT_SECRET");
        validate_jwt_secret(&jwt_secret);

        Self {
            database_url: required("DATABASE_URL"),
            db_max_connections: env::var("DB_MAX_CONNECTIONS")
                .unwrap_or_else(|_| "10".to_string())
                .parse()
                .expect("DB_MAX_CONNECTIONS must be a number"),
            host: env::var("HOST").unwrap_or_else(|_| "0.0.0.0".to_string()),
            port: env::var("PORT")
                .unwrap_or_else(|_| "3000".to_string())
                .parse()
                .expect("PORT must be a number"),
            jwt_secret,
            encryption_key,
            authentik_issuer: required("AUTHENTIK_ISSUER"),
            authentik_client_id_admin: required("AUTHENTIK_CLIENT_ID_ADMIN"),
            authentik_client_id_storefront: required("AUTHENTIK_CLIENT_ID_STOREFRONT"),
            authentik_client_secret_admin: required("AUTHENTIK_CLIENT_SECRET_ADMIN"),
            authentik_client_secret_storefront: required("AUTHENTIK_CLIENT_SECRET_STOREFRONT"),
            authentik_api_token: env::var("AUTHENTIK_API_TOKEN").ok(),
            stripe_secret_key: env::var("STRIPE_SECRET_KEY").ok(),
            cors_origins: env::var("CORS_ORIGINS")
                .unwrap_or_else(|_| "http://localhost:5173,http://localhost:5174".to_string())
                .split(',')
                .map(|s| s.trim().to_string())
                .collect(),
            job_poll_interval_secs: env::var("JOB_POLL_INTERVAL_SECS")
                .unwrap_or_else(|_| "5".to_string())
                .parse()
                .expect("JOB_POLL_INTERVAL_SECS must be a number"),
            redis_url: env::var("REDIS_URL")
                .unwrap_or_else(|_| "redis://redis:6379".to_string()),
        }
    }

    pub fn encryption_key_bytes(&self) -> [u8; 32] {
        let bytes = self.encryption_key.as_bytes();
        let mut key = [0u8; 32];

        // Since we validated the key is at least 32 bytes, we can safely take first 32
        // If you want to support hex-encoded keys, decode them first
        let len = bytes.len().min(32);
        key[..len].copy_from_slice(&bytes[..len]);

        key
    }
}

fn required(name: &str) -> String {
    env::var(name).unwrap_or_else(|_| panic!("{} must be set", name))
}

/// SEC-002 FIX: Validate encryption key strength
fn validate_encryption_key(key: &str) {
    // Minimum length check: 32 bytes = 64 hex chars or 44 base64 chars
    if key.len() < 32 {
        panic!(
            "ENCRYPTION_KEY must be at least 32 characters long (got {} chars). \
             Generate a secure key with: openssl rand -hex 32",
            key.len()
        );
    }

    // Reject common weak development keys
    let weak_keys = [
        "dev", "test", "secret", "password", "changeme", "localhost",
        "development", "testing", "mysecret", "key", "encryption",
        "12345", "qwerty", "admin", "root", "demo",
    ];

    let key_lower = key.to_lowercase();
    for weak in &weak_keys {
        if key_lower.contains(weak) && key.len() < 64 {
            panic!(
                "ENCRYPTION_KEY appears to be a weak development key containing '{}'. \
                 Use a cryptographically secure random key. \
                 Generate one with: openssl rand -hex 32",
                weak
            );
        }
    }

    // Check if key has sufficient entropy (not all same character)
    let chars: Vec<char> = key.chars().collect();
    let unique_chars: std::collections::HashSet<_> = chars.iter().collect();
    if unique_chars.len() < 8 {
        panic!(
            "ENCRYPTION_KEY has insufficient entropy (only {} unique characters). \
             Use a cryptographically secure random key. \
             Generate one with: openssl rand -hex 32",
            unique_chars.len()
        );
    }

    #[cfg(not(debug_assertions))]
    {
        // In production/release builds, require even stronger keys
        if key.len() < 64 {
            panic!(
                "ENCRYPTION_KEY must be at least 64 characters in production (got {} chars). \
                 Generate a secure key with: openssl rand -hex 32",
                key.len()
            );
        }
    }
}

/// SEC-002 FIX: Validate JWT secret strength
fn validate_jwt_secret(secret: &str) {
    if secret.len() < 32 {
        panic!(
            "JWT_SECRET must be at least 32 characters long (got {} chars). \
             Generate a secure secret with: openssl rand -hex 32",
            secret.len()
        );
    }

    // Reject common weak secrets
    let weak_secrets = ["secret", "jwt", "token", "auth", "key"];
    let secret_lower = secret.to_lowercase();
    for weak in &weak_secrets {
        if secret_lower == *weak {
            panic!(
                "JWT_SECRET is too weak ('{}'). \
                 Use a cryptographically secure random secret. \
                 Generate one with: openssl rand -hex 32",
                weak
            );
        }
    }
}
