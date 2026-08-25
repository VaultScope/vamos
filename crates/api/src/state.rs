use std::sync::Arc;

use sqlx::PgPool;

use crate::config::Config;

#[derive(Clone)]
pub struct AppState {
    pub db: PgPool,
    pub config: Arc<Config>,
    pub csrf_config: axum_csrf::CsrfConfig,
}

impl axum::extract::FromRef<AppState> for axum_csrf::CsrfConfig {
    fn from_ref(state: &AppState) -> Self {
        state.csrf_config.clone()
    }
}
