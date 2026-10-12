use axum::extract::{Path, State};
use axum::routing::{get, post};
use axum::{Json, Router};
use uuid::Uuid;

use crate::auth::AuthAdmin;
use crate::error::ApiError;
use crate::state::AppState;
use vaultscope_db::models::Job;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/", get(list))
        .route("/{id}/retry", post(retry_job))
        .route("/{id}/cancel", post(cancel_job))
}

async fn list(auth: AuthAdmin, State(state): State<AppState>) -> Result<Json<Vec<Job>>, ApiError> {
    crate::require_permission!(auth, state, "jobs.view");
    let jobs = sqlx::query_as::<_, Job>("SELECT * FROM jobs ORDER BY created_at DESC")
        .fetch_all(&state.db)
        .await?;

    Ok(Json(jobs))
}

async fn retry_job(
    auth: AuthAdmin,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<Job>, ApiError> {
    crate::require_permission!(auth, state, "jobs.manage");
    let job: Job = sqlx::query_as(
        r#"UPDATE jobs
           SET status = 'queued'::job_status,
               error = NULL,
               attempts = 0,
               scheduled_at = now(),
               started_at = NULL,
               completed_at = NULL
           WHERE id = $1
           RETURNING *"#
    )
    .bind(id)
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound("job not found".into()))?;

    Ok(Json(job))
}

async fn cancel_job(
    auth: AuthAdmin,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<Job>, ApiError> {
    crate::require_permission!(auth, state, "jobs.manage");
    let job: Job = sqlx::query_as(
        r#"UPDATE jobs
           SET status = 'cancelled'::job_status,
               completed_at = now()
           WHERE id = $1
           RETURNING *"#
    )
    .bind(id)
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound("job not found".into()))?;

    Ok(Json(job))
}
