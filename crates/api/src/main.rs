mod auth;
mod config;
mod error;
mod middleware;
mod routes;
mod state;
mod validation;

use std::net::SocketAddr;
use std::sync::Arc;

use tokio::net::TcpListener;
use tokio::sync::watch;
use tower_http::trace::TraceLayer;
use tracing::info;
use tracing_subscriber::EnvFilter;
use uuid::Uuid;

use vaultscope_connectors::ProvisioningProvider;
use vaultscope_connectors::hetzner_cloud::HetznerCloud;
use vaultscope_jobs::runner::{ConnectorRegistry, JobRunner};

use config::Config;
use state::AppState;

struct StaticConnectorRegistry;

impl ConnectorRegistry for StaticConnectorRegistry {
    fn get(&self, _connector_id: &Uuid) -> Option<Arc<dyn ProvisioningProvider>> {
        Some(Arc::new(HetznerCloud::new("dummy".into())))
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

    let state = AppState {
        db: pool.clone(),
        config: Arc::new(config.clone()),
        csrf_config: axum_csrf::CsrfConfig::default(),
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
    
    let runner = JobRunner::new(
        pool.clone(),
        Arc::new(StaticConnectorRegistry),
        std::time::Duration::from_secs(5),
    );
    tokio::spawn(runner.run(shutdown_rx));

    axum::serve(listener, app).await.expect("server error");
    let _ = shutdown_tx.send(true);
}
