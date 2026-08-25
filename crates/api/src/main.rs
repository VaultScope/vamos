mod auth;
mod config;
mod error;
mod middleware;
mod routes;
mod state;

use std::net::SocketAddr;
use std::sync::Arc;

use tokio::net::TcpListener;
use tower_http::trace::TraceLayer;
use tracing::info;
use tracing_subscriber::EnvFilter;

use config::Config;
use state::AppState;

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
    };

    let app = routes::build_router(state)
        .layer(TraceLayer::new_for_http())
        .layer(middleware::cors::layer(&config.cors_origins));

    let addr: SocketAddr = format!("{}:{}", config.host, config.port)
        .parse()
        .expect("invalid address");

    let listener = TcpListener::bind(addr).await.expect("failed to bind");
    info!("listening on {}", addr);

    axum::serve(listener, app).await.expect("server error");
}
