use std::sync::Arc;

use sqlx::PgPool;
use tokio::sync::broadcast;
use uuid::Uuid;
use vaultscope_db::models::TicketMessage;

use crate::config::Config;

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct TicketEvent {
    pub ticket_id: Uuid,
    pub message: TicketMessage,
}

#[derive(Clone)]
pub struct AppState {
    pub db: PgPool,
    pub config: Arc<Config>,
    pub csrf_config: axum_csrf::CsrfConfig,
    pub redis: Option<redis::aio::ConnectionManager>,
    pub ticket_bus: broadcast::Sender<TicketEvent>,
}

impl axum::extract::FromRef<AppState> for axum_csrf::CsrfConfig {
    fn from_ref(state: &AppState) -> Self {
        state.csrf_config.clone()
    }
}
