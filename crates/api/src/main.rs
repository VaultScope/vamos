mod activity;
mod auth;
mod config;
mod error;
mod mailer;
mod media;
mod middleware;
mod openapi;
mod pdf;
mod reliablesite_catalog;
mod routes;
mod state;
mod validation;

use std::net::SocketAddr;
use std::sync::Arc;

use tokio::net::TcpListener;
use tokio::sync::watch;
use tower_http::trace::TraceLayer;
use tracing::{info, warn};
use tracing_subscriber::EnvFilter;

use vaultscope_connectors::hetzner_cloud::HetznerCloud;
use vaultscope_connectors::sftpgo::SftpGo;
use vaultscope_connectors::ProvisioningProvider;
use vaultscope_jobs::billing::BillingNotifier;
use vaultscope_jobs::runner::{ConnectorRegistry, JobNotifier, JobRunner};

use config::Config;
use state::AppState;

struct EnvConnectorRegistry {
    config: Arc<Config>,
}

#[async_trait::async_trait]
impl ConnectorRegistry for EnvConnectorRegistry {
    async fn get(&self, provider: &str) -> Option<Arc<dyn ProvisioningProvider>> {
        match provider {
            "hetzner_cloud" => {
                let token = self.config.hetzner_api_token.as_ref()?.trim().to_string();
                Some(Arc::new(HetznerCloud::new(token)))
            }
            "sftpgo" => {
                let base_url = self.config.sftpgo_url.as_ref()?.trim().to_string();
                let admin_user = self.config.sftpgo_user.as_ref()?.trim().to_string();
                let admin_pass = self.config.sftpgo_pass.as_ref()?.trim().to_string();
                Some(Arc::new(SftpGo::new(base_url, admin_user, admin_pass)))
            }
            "reliablesite" => Some(Arc::new(self.config.reliablesite_connector())),
            _ => None,
        }
    }
}

struct ApiJobNotifier {
    state: AppState,
}

#[async_trait::async_trait]
impl JobNotifier for ApiJobNotifier {
    async fn on_service_provisioned(&self, service_id: uuid::Uuid, response: &serde_json::Value) {
        mailer::spawn_service_welcome_notification(self.state.clone(), service_id, response.clone());
    }
}

#[async_trait::async_trait]
impl BillingNotifier for ApiJobNotifier {
    async fn on_upcoming_renewal_7d(
        &self,
        service_id: uuid::Uuid,
        customer_id: uuid::Uuid,
        service_name: &str,
        price: rust_decimal::Decimal,
        due_date: &str,
    ) {
        mailer::spawn_upcoming_payment_7d_notification(
            self.state.clone(),
            service_id,
            customer_id,
            service_name.to_string(),
            price,
            due_date.to_string(),
        );
    }

    async fn on_invoice_created(&self, invoice_id: uuid::Uuid) {
        mailer::spawn_new_invoice_notification(self.state.clone(), invoice_id);
    }

    async fn on_invoice_payment_warning(&self, invoice_id: Uuid, is_overdue: bool) {
        mailer::spawn_payment_warning_notification(self.state.clone(), invoice_id, is_overdue);
    }
}

use uuid::Uuid;

#[tokio::main]
async fn main() {
    dotenvy::dotenv().ok();

    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();

    let config = Config::from_env();
    info!("starting VaultScope API on {}:{}", config.host, config.port);

    let pool = {
        let mut attempts = 0u32;
        loop {
            attempts += 1;
            match vaultscope_db::create_pool(&config.database_url, config.db_max_connections).await {
                Ok(p) => break p,
                Err(e) if attempts < 10 => {
                    warn!("database connection attempt {}/10 failed ({}), retrying in 2s...", attempts, e);
                    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                }
                Err(e) => panic!("failed to connect to database after {} attempts: {}", attempts, e),
            }
        }
    };

    vaultscope_db::run_migrations(&pool)
        .await
        .expect("failed to run migrations");

    info!("database connected and migrations applied");

    // SEC-001: Connect to Redis for OIDC state storage (non-blocking fallback with fast 500ms timeout)
    let redis = match redis::Client::open(config.redis_url.as_str()) {
        Ok(client) => {
            match tokio::time::timeout(
                std::time::Duration::from_millis(500),
                redis::aio::ConnectionManager::new(client),
            )
            .await
            {
                Ok(Ok(mgr)) => {
                    info!("redis connected");
                    Some(mgr)
                }
                Ok(Err(e)) => {
                    warn!("redis connection manager failed ({}), continuing without redis", e);
                    None
                }
                Err(_) => {
                    warn!("redis connection timed out after 500ms, continuing without redis");
                    None
                }
            }
        }
        Err(e) => {
            warn!("redis client open failed ({}), continuing without redis", e);
            None
        }
    };

    let (ticket_bus, _) = tokio::sync::broadcast::channel(1024);

    let state = AppState {
        db: pool.clone(),
        config: Arc::new(config.clone()),
        csrf_config: axum_csrf::CsrfConfig::default(),
        redis,
        ticket_bus,
    };

    let app = routes::build_router(state.clone())
        .layer(TraceLayer::new_for_http())
        .layer(axum::middleware::from_fn(middleware::security_headers::security_headers_middleware))
        .layer(middleware::cors::layer(&config.cors_origins));

    let addr: SocketAddr = format!("{}:{}", config.host, config.port)
        .parse()
        .expect("invalid address");

    let listener = TcpListener::bind(addr).await.expect("failed to bind");
    info!("listening on {}", addr);

    let (shutdown_tx, shutdown_rx) = watch::channel(false);

    let registry = EnvConnectorRegistry {
        config: Arc::new(config.clone()),
    };
    let runner = JobRunner::new(
        pool.clone(),
        Arc::new(registry),
        std::time::Duration::from_secs(config.job_poll_interval_secs),
    ).with_notifier(Arc::new(ApiJobNotifier {
        state: state.clone(),
    }));
    tokio::spawn(runner.run(shutdown_rx.clone()));

    let billing_runner = vaultscope_jobs::BillingRunner::new(
        pool.clone(),
        std::time::Duration::from_secs(3600), // Check every hour
    )
    .with_notifier(Arc::new(ApiJobNotifier {
        state: state.clone(),
    }));
    tokio::spawn(billing_runner.run(shutdown_rx.clone()));

    axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            let ctrl_c = async {
                tokio::signal::ctrl_c()
                    .await
                    .expect("failed to install CTRL+C signal handler");
            };
            #[cfg(unix)]
            let terminate = async {
                tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
                    .expect("failed to install SIGTERM signal handler")
                    .recv()
                    .await;
            };
            #[cfg(not(unix))]
            let terminate = std::future::pending::<()>();

            tokio::select! {
                _ = ctrl_c => {},
                _ = terminate => {},
            }
            info!("shutdown signal received, stopping server");
            let _ = shutdown_tx.send(true);
        })
        .await
        .expect("server error");
}
