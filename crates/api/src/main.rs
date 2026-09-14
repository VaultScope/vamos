mod auth;
mod config;
mod error;
mod middleware;
mod routes;
mod state;
mod validation;

use std::net::SocketAddr;
use std::sync::Arc;

use sqlx::PgPool;
use tokio::net::TcpListener;
use tokio::sync::watch;
use tower_http::trace::TraceLayer;
use tracing::info;
use tracing_subscriber::EnvFilter;
use uuid::Uuid;

use vaultscope_connectors::hetzner_cloud::HetznerCloud;
use vaultscope_connectors::ProvisioningProvider;
use vaultscope_jobs::runner::{ConnectorRegistry, JobRunner};

use config::Config;
use state::AppState;

struct DbConnectorRegistry {
    pool: PgPool,
    encryption_key: [u8; 32],
}

#[async_trait::async_trait]
impl ConnectorRegistry for DbConnectorRegistry {
    async fn get(&self, connector_id: &Uuid) -> Option<Arc<dyn ProvisioningProvider>> {
        let pool = self.pool.clone();
        let id = *connector_id;
        let key = self.encryption_key;

        let row: Option<(String, Option<Vec<u8>>)> =
            sqlx::query_as("SELECT provider, config_encrypted FROM connectors WHERE id = $1")
                .bind(id)
                .fetch_optional(&pool)
                .await
                .ok()
                .flatten();

        let (provider, config_encrypted) = row?;
        let config_bytes = config_encrypted?;
        let decrypted = vaultscope_common::crypto::decrypt(&key, &config_bytes).ok()?;
        let config: serde_json::Value = serde_json::from_slice(&decrypted).ok()?;

        match provider.as_str() {
            "hetzner_cloud" => {
                let token = config.get("token")?.as_str()?.trim().to_string();
                Some(Arc::new(HetznerCloud::new(token)))
            }
            _ => None,
        }
    }
}

#[tokio::main]
async fn main() {
    dotenvy::dotenv().ok();

    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();

    let config = Config::from_env();
    info!("starting VaultScope API on {}:{}", config.host, config.port);

    let pool = vaultscope_db::create_pool(&config.database_url, config.db_max_connections)
        .await
        .expect("failed to connect to database");

    vaultscope_db::run_migrations(&pool)
        .await
        .expect("failed to run migrations");

    info!("database connected and migrations applied");

    // SEC-001: Connect to Redis for OIDC state storage
    let redis_client = redis::Client::open(config.redis_url.as_str())
        .expect("failed to connect to redis");
    let redis = redis::aio::ConnectionManager::new(redis_client)
        .await
        .expect("failed to create redis connection manager");

    info!("redis connected");

    let state = AppState {
        db: pool.clone(),
        config: Arc::new(config.clone()),
        csrf_config: axum_csrf::CsrfConfig::default(),
        redis,
    };

    let app = routes::build_router(state)
        .layer(TraceLayer::new_for_http())
        .layer(middleware::cors::layer(&config.cors_origins));

    let addr: SocketAddr = format!("{}:{}", config.host, config.port)
        .parse()
        .expect("invalid address");

    let listener = TcpListener::bind(addr).await.expect("failed to bind");
    info!("listening on {}", addr);

    let (shutdown_tx, shutdown_rx) = watch::channel(false);

    let registry = DbConnectorRegistry {
        pool: pool.clone(),
        encryption_key: config.encryption_key_bytes(),
    };
    let runner = JobRunner::new(
        pool.clone(),
        Arc::new(registry),
        std::time::Duration::from_secs(config.job_poll_interval_secs),
    );
    tokio::spawn(runner.run(shutdown_rx));

    axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            tokio::signal::ctrl_c()
                .await
                .expect("failed to install CTRL+C signal handler");
            info!("shutdown signal received, stopping server");
            let _ = shutdown_tx.send(true);
        })
        .await
        .expect("server error");
}
