use axum::extract::State;
use axum::routing::get;
use axum::{Json, Router};

use crate::auth::AuthAdmin;
use crate::error::ApiError;
use crate::state::AppState;
use vaultscope_db::models::Job;

pub fn routes() -> Router<AppState> {
    Router::new().route("/", get(list))
}

async fn list(auth: AuthAdmin, State(state): State<AppState>) -> Result<Json<Vec<Job>>, ApiError> {
    crate::require_permission!(auth, state, "jobs.view");
    let jobs = sqlx::query_as::<_, Job>("SELECT * FROM jobs ORDER BY created_at DESC")
        .fetch_all(&state.db)
        .await?;

    Ok(Json(jobs))
}
