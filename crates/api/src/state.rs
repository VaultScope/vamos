use std::collections::HashMap;
use std::sync::Arc;

use sqlx::PgPool;
use tokio::sync::{broadcast, RwLock};
use uuid::Uuid;
use vaultscope_db::models::TicketMessage;

use crate::auth::Audience;
use crate::config::Config;

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct TicketEvent {
    pub ticket_id: Uuid,
    pub message: TicketMessage,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct SseTicketEntry {
    pub sub: Uuid,
    pub email: String,
    pub role: String,
    pub audience: Audience,
    pub auth_version: Option<i64>,
    pub expires_at: i64,
}

pub type SseTicketStore = Arc<RwLock<HashMap<String, SseTicketEntry>>>;

pub fn new_sse_tickets() -> SseTicketStore {
    Arc::new(RwLock::new(HashMap::new()))
}

#[derive(Clone)]
pub struct AppState {
    pub db: PgPool,
    pub config: Arc<Config>,
    pub csrf_config: axum_csrf::CsrfConfig,
    pub redis: Option<redis::aio::ConnectionManager>,
    pub ticket_bus: broadcast::Sender<TicketEvent>,
    pub sse_tickets: SseTicketStore,
}

impl axum::extract::FromRef<AppState> for axum_csrf::CsrfConfig {
    fn from_ref(state: &AppState) -> Self {
        state.csrf_config.clone()
    }
}
