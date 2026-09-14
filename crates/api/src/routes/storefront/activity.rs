use axum::extract::State;
use axum::routing::get;
use axum::{Json, Router};
use serde::Serialize;

use crate::auth::AuthCustomer;
use crate::error::ApiError;
use crate::state::AppState;

pub fn routes() -> Router<AppState> {
    Router::new().route("/", get(list_mine))
}

#[derive(Serialize)]
struct ActivityEntry {
    id: uuid::Uuid,
    action: String,
    target: String,
    detail: String,
    created_at: chrono::DateTime<chrono::Utc>,
}

async fn list_mine(
    auth: AuthCustomer,
    State(state): State<AppState>,
) -> Result<Json<Vec<ActivityEntry>>, ApiError> {
    let entries: Vec<ActivityEntry> = sqlx::query_as!(
        ActivityEntry,
        r#"SELECT id, action, target, detail, created_at as "created_at!" FROM activity_log
           WHERE actor_id = $1 AND actor_type = 'customer'
           ORDER BY created_at DESC LIMIT 100"#,
        auth.0.sub
    )
    .fetch_all(&state.db)
    .await?;

    Ok(Json(entries))
}
