use sqlx::PgPool;
use std::sync::Arc;
use vaultscope_api::config::Config;
use vaultscope_api::state::AppState;

pub async fn setup_test_db() -> PgPool {
    let database_url = std::env::var("TEST_DATABASE_URL")
        .unwrap_or_else(|_| "postgres://postgres:postgres@localhost/vaultscope_test".to_string());

    let pool = sqlx::PgPool::connect(&database_url)
        .await
        .expect("Failed to connect to test database");

    // Skip dropping schema to preserve dev database
    // sqlx::query("DROP SCHEMA public CASCADE; CREATE SCHEMA public;")
    //     .execute(&pool)
    //     .await
    //     .expect("Failed to reset test database");

    vaultscope_db::run_migrations(&pool)
        .await
        .expect("Failed to run migrations");

    pool
}

pub fn test_config() -> Config {
    Config {
        database_url: std::env::var("TEST_DATABASE_URL").unwrap_or_else(|_| {
            "postgres://postgres:postgres@localhost/vaultscope_test".to_string()
        }),
        db_max_connections: 5,
        host: "127.0.0.1".to_string(),
        port: 3001,
        jwt_secret: "test_secret_key_for_testing_only".to_string(),
        encryption_key: "test_encryption_key_32_bytes_!!".to_string(),
        authentik_enabled: true,
        authentik_issuer: "https://auth.test.local".to_string(),
        authentik_client_id_admin: "test_admin".to_string(),
        authentik_client_id_storefront: "test_storefront".to_string(),
        authentik_client_secret_admin: "test_secret".to_string(),
        authentik_client_secret_storefront: "test_secret".to_string(),
        authentik_api_token: None,
        stripe_secret_key: None,
        cors_origins: vec!["http://localhost:5173".to_string()],
        job_poll_interval_secs: 5,
        redis_url: "redis://127.0.0.1:6379".to_string(),
        hetzner_api_token: None,
        reliablesite_api_key: None,
        reliablesite_api_url: None,
        sftpgo_url: None,
        sftpgo_user: None,
        sftpgo_pass: None,
        stripe_webhook_secret: None,
        smtp_host: None,
        smtp_username: None,
        smtp_password: None,
        smtp_from: None,
        dev_mode: true,
        storefront_url: "http://localhost:5173".to_string(),
    }
}

pub async fn test_app_state() -> AppState {
    let pool = setup_test_db().await;
    let config = test_config();
    let (ticket_bus, _) = tokio::sync::broadcast::channel(1024);

    AppState {
        db: pool,
        config: Arc::new(config),
        csrf_config: axum_csrf::CsrfConfig::default(),
        redis: None,
        ticket_bus,
    }
}
