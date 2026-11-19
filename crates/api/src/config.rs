use std::env;
use std::fmt;

#[derive(Clone)]
pub struct Config {
    pub database_url: String,
    pub db_max_connections: u32,
    pub host: String,
    pub port: u16,
    pub jwt_secret: String,
    pub encryption_key: String,
    pub authentik_enabled: bool,
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
    pub hetzner_api_token: Option<String>,
    pub reliablesite_api_key: Option<String>,
    pub reliablesite_api_url: Option<String>,
    pub reliablesite_order_api_url: Option<String>,
    pub reliablesite_inventory_url: Option<String>,
    pub reliablesite_payment_method: Option<String>,
    pub sftpgo_url: Option<String>,
    pub sftpgo_user: Option<String>,
    pub sftpgo_pass: Option<String>,
    pub stripe_webhook_secret: Option<String>,
    pub smtp_host: Option<String>,
    pub smtp_username: Option<String>,
    pub smtp_password: Option<String>,
    pub smtp_from: Option<String>,
    pub dev_mode: bool,
    pub storefront_url: String,
}

fn redact_opt(v: &Option<String>) -> Option<&'static str> {
    v.as_ref().map(|_| "[REDACTED]")
}

impl fmt::Debug for Config {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Config")
            .field("database_url", &"[REDACTED]")
            .field("db_max_connections", &self.db_max_connections)
            .field("host", &self.host)
            .field("port", &self.port)
            .field("jwt_secret", &"[REDACTED]")
            .field("encryption_key", &"[REDACTED]")
            .field("authentik_enabled", &self.authentik_enabled)
            .field("authentik_issuer", &self.authentik_issuer)
            .field("authentik_client_id_admin", &self.authentik_client_id_admin)
            .field(
                "authentik_client_id_storefront",
                &self.authentik_client_id_storefront,
            )
            .field("authentik_client_secret_admin", &"[REDACTED]")
            .field("authentik_client_secret_storefront", &"[REDACTED]")
            .field("authentik_api_token", &redact_opt(&self.authentik_api_token))
            .field("stripe_secret_key", &redact_opt(&self.stripe_secret_key))
            .field("cors_origins", &self.cors_origins)
            .field("job_poll_interval_secs", &self.job_poll_interval_secs)
            .field("redis_url", &"[REDACTED]")
            .field("hetzner_api_token", &redact_opt(&self.hetzner_api_token))
            .field("reliablesite_api_key", &redact_opt(&self.reliablesite_api_key))
            .field("reliablesite_api_url", &self.reliablesite_api_url)
            .field("reliablesite_order_api_url", &self.reliablesite_order_api_url)
            .field("reliablesite_inventory_url", &self.reliablesite_inventory_url)
            .field("reliablesite_payment_method", &self.reliablesite_payment_method)
            .field("sftpgo_url", &self.sftpgo_url)
            .field("sftpgo_user", &self.sftpgo_user)
            .field("sftpgo_pass", &redact_opt(&self.sftpgo_pass))
            .field("stripe_webhook_secret", &redact_opt(&self.stripe_webhook_secret))
            .field("smtp_host", &self.smtp_host)
            .field("smtp_username", &self.smtp_username)
            .field("smtp_password", &redact_opt(&self.smtp_password))
            .field("smtp_from", &self.smtp_from)
            .field("dev_mode", &self.dev_mode)
            .field("storefront_url", &self.storefront_url)
            .finish()
    }
}

impl Config {
    pub fn from_env() -> Self {
        let dev_mode = env::var("DEV_MODE")
            .map(|v| v == "true" || v == "1")
            .unwrap_or(false);

        let encryption_key = required("ENCRYPTION_KEY");
        validate_encryption_key(&encryption_key);

        let jwt_secret = required("JWT_SECRET");
        validate_jwt_secret(&jwt_secret);

        let auth_mode = env::var("AUTH_MODE").unwrap_or_else(|_| "local".to_string());
        let authentik_enabled = env::var("AUTHENTIK_ENABLED")
            .map(|v| v == "true" || v == "1")
            .unwrap_or_else(|_| env::var("AUTHENTIK_ISSUER").is_ok() && auth_mode != "local");

        let authentik_client_secret_admin = required_oidc_secret(
            "AUTHENTIK_CLIENT_SECRET_ADMIN",
            authentik_enabled,
            dev_mode,
        );
        let authentik_client_secret_storefront = required_oidc_secret(
            "AUTHENTIK_CLIENT_SECRET_STOREFRONT",
            authentik_enabled,
            dev_mode,
        );

        let storefront_url = env::var("STOREFRONT_URL").unwrap_or_else(|_| {
            if dev_mode {
                "http://localhost:5173".to_string()
            } else {
                "https://vaultscope.de".to_string()
            }
        });

        let cors_origins = match env::var("CORS_ORIGINS") {
            Ok(raw) => raw
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect(),
            Err(_) if dev_mode => vec![
                "http://localhost:5173".to_string(),
                "http://localhost:5174".to_string(),
            ],
            Err(_) => {
                tracing::warn!(
                    "CORS_ORIGINS is not set in non-dev mode; defaulting strictly to STOREFRONT_URL ({})",
                    storefront_url
                );
                vec![storefront_url.clone()]
            }
        };

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
            authentik_enabled,
            authentik_issuer: env::var("AUTHENTIK_ISSUER").unwrap_or_else(|_| "http://localhost:9000".to_string()),
            authentik_client_id_admin: env::var("AUTHENTIK_CLIENT_ID_ADMIN").unwrap_or_else(|_| "vaultscope-admin".to_string()),
            authentik_client_id_storefront: env::var("AUTHENTIK_CLIENT_ID_STOREFRONT").unwrap_or_else(|_| "vaultscope-storefront".to_string()),
            authentik_client_secret_admin,
            authentik_client_secret_storefront,
            authentik_api_token: env::var("AUTHENTIK_API_TOKEN").ok(),
            stripe_secret_key: env::var("STRIPE_SECRET_KEY").ok(),
            cors_origins,
            job_poll_interval_secs: env::var("JOB_POLL_INTERVAL_SECS")
                .unwrap_or_else(|_| "5".to_string())
                .parse()
                .expect("JOB_POLL_INTERVAL_SECS must be a number"),
            redis_url: env::var("REDIS_URL")
                .unwrap_or_else(|_| "redis://redis:6379".to_string()),
            hetzner_api_token: env::var("HETZNER_API_TOKEN").ok(),
            reliablesite_api_key: env::var("RELIABLESITE_API_KEY").ok(),
            reliablesite_api_url: env::var("RELIABLESITE_API_URL").ok(),
            reliablesite_order_api_url: env::var("RELIABLESITE_ORDER_API_URL").ok(),
            reliablesite_inventory_url: env::var("RELIABLESITE_INVENTORY_URL").ok(),
            reliablesite_payment_method: env::var("RELIABLESITE_PAYMENT_METHOD").ok(),
            sftpgo_url: env::var("SFTPGO_URL").ok(),
            sftpgo_user: env::var("SFTPGO_USER").ok(),
            sftpgo_pass: env::var("SFTPGO_PASS").ok(),
            stripe_webhook_secret: env::var("STRIPE_WEBHOOK_SECRET").ok(),
            smtp_host: env::var("SMTP_HOST").ok(),
            smtp_username: env::var("SMTP_USERNAME").ok(),
            smtp_password: env::var("SMTP_PASSWORD").ok(),
            smtp_from: env::var("SMTP_FROM").ok(),
            dev_mode,
            storefront_url,
        }
    }

    pub fn encryption_key_bytes(&self) -> [u8; 32] {
        let trimmed = self.encryption_key.trim();
        if trimmed.len() == 64 && trimmed.bytes().all(|b| b.is_ascii_hexdigit()) {
            if let Ok(decoded) = hex::decode(trimmed) {
                if decoded.len() == 32 {
                    let mut key = [0u8; 32];
                    key.copy_from_slice(&decoded);
                    return key;
                }
            }
        }

        let bytes = self.encryption_key.as_bytes();
        let mut key = [0u8; 32];

        // Since we validated the key is at least 32 bytes, we can safely take first 32
        let len = bytes.len().min(32);
        key[..len].copy_from_slice(&bytes[..len]);

        key
    }

    pub fn reliablesite_connector(&self) -> vaultscope_connectors::reliablesite::ReliableSite {
        use vaultscope_connectors::reliablesite::{
            inventory_soap::DEFAULT_INVENTORY_URL, order_api::DEFAULT_ORDER_API_URL, ReliableSite,
        };

        let api_key = self
            .reliablesite_api_key
            .as_deref()
            .unwrap_or("")
            .trim()
            .to_string();

        let order_url = self
            .reliablesite_order_api_url
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .or_else(|| {
                self.reliablesite_api_url
                    .as_deref()
                    .map(str::trim)
                    .filter(|s| !s.is_empty() && !s.contains("api.reliablesite.net"))
            })
            .unwrap_or(DEFAULT_ORDER_API_URL)
            .to_string();

        let inventory_url = self
            .reliablesite_inventory_url
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or(DEFAULT_INVENTORY_URL)
            .to_string();

        ReliableSite::with_endpoints(
            api_key,
            order_url,
            inventory_url,
            self.reliablesite_payment_method.clone(),
        )
    }
}

fn required(name: &str) -> String {
    env::var(name).unwrap_or_else(|_| panic!("{} must be set", name))
}

fn required_oidc_secret(name: &str, authentik_enabled: bool, dev_mode: bool) -> String {
    let val = env::var(name).unwrap_or_default();
    if authentik_enabled && !dev_mode && val.trim().is_empty() {
        panic!("{} must be set when AUTHENTIK_ENABLED is true in non-dev mode", name);
    }
    val
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

    let is_prod = env::var("ENVIRONMENT").unwrap_or_default() == "production";
    let key_lower = key.to_lowercase();
    if is_prod {
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
pub fn validate_jwt_secret(secret: &str) {
    validate_jwt_secret_with_mode(secret, cfg!(not(debug_assertions)));
}

pub fn validate_jwt_secret_with_mode(secret: &str, release_mode: bool) {
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

    let unique_chars: std::collections::HashSet<char> = secret.chars().collect();
    if unique_chars.len() < 12 {
        panic!(
            "JWT_SECRET has insufficient entropy (only {} unique characters, minimum 12). \
             Use a cryptographically secure random secret. \
             Generate one with: openssl rand -hex 32",
            unique_chars.len()
        );
    }

    if release_mode {
        if secret.len() < 64 {
            panic!(
                "JWT_SECRET must be at least 64 characters in release mode (got {} chars). \
                 Generate a secure secret with: openssl rand -hex 32",
                secret.len()
            );
        }
        for placeholder in ["change-me", "changeme", "dev-secret", "replace-me"] {
            if secret_lower.contains(placeholder) {
                panic!(
                    "JWT_SECRET contains placeholder substring '{}'. \
                     Use a cryptographically secure random secret.",
                    placeholder
                );
            }
        }
    }
}
