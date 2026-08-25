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
    pub cors_origins: Vec<String>,
    pub job_poll_interval_secs: u64,
}

impl Config {
    pub fn from_env() -> Self {
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
            jwt_secret: required("JWT_SECRET"),
            encryption_key: required("ENCRYPTION_KEY"),
            authentik_issuer: required("AUTHENTIK_ISSUER"),
            authentik_client_id_admin: required("AUTHENTIK_CLIENT_ID_ADMIN"),
            authentik_client_id_storefront: required("AUTHENTIK_CLIENT_ID_STOREFRONT"),
            authentik_client_secret_admin: required("AUTHENTIK_CLIENT_SECRET_ADMIN"),
            authentik_client_secret_storefront: required("AUTHENTIK_CLIENT_SECRET_STOREFRONT"),
            cors_origins: env::var("CORS_ORIGINS")
                .unwrap_or_else(|_| "http://localhost:5173,http://localhost:5174".to_string())
                .split(',')
                .map(|s| s.trim().to_string())
                .collect(),
            job_poll_interval_secs: env::var("JOB_POLL_INTERVAL_SECS")
                .unwrap_or_else(|_| "5".to_string())
                .parse()
                .expect("JOB_POLL_INTERVAL_SECS must be a number"),
        }
    }

    pub fn encryption_key_bytes(&self) -> [u8; 32] {
        let bytes = self.encryption_key.as_bytes();
        let mut key = [0u8; 32];
        let len = bytes.len().min(32);
        key[..len].copy_from_slice(&bytes[..len]);
        key
    }
}

fn required(name: &str) -> String {
    env::var(name).unwrap_or_else(|_| panic!("{} must be set", name))
}
