use axum::extract::{Path, State};
use axum::Json;
use utoipa_axum::{router::OpenApiRouter, routes};
use uuid::Uuid;

use crate::auth::AuthAdmin;
use crate::error::ApiError;
use crate::openapi::ErrorResponse;
use crate::state::AppState;
use vaultscope_db::models::Job;

pub fn routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(list))
        .routes(routes!(retry_job))
        .routes(routes!(cancel_job))
}

#[utoipa::path(
    get,
    path = "/",
    tag = "Admin Jobs",
    security(("bearer_auth" = [])),
    responses((status = 200, description = "List background jobs", body = Vec<Job>))
)]
pub(crate) async fn list(auth: AuthAdmin, State(state): State<AppState>) -> Result<Json<Vec<Job>>, ApiError> {
    crate::require_permission!(auth, state, "jobs.view");
    let jobs = sqlx::query_as::<_, Job>("SELECT * FROM jobs ORDER BY created_at DESC LIMIT 500")
        .fetch_all(&state.db)
        .await?;

    Ok(Json(jobs))
}

#[utoipa::path(
    post,
    path = "/{id}/retry",
    tag = "Admin Jobs",
    security(("bearer_auth" = []), ("csrf_token" = [])),
    params(("id" = Uuid, Path, description = "Job ID")),
    responses(
        (status = 200, description = "Job requeued", body = Job),
        (status = 404, description = "Job not found", body = ErrorResponse)
    )
)]
pub(crate) async fn retry_job(
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
             AND status IN ('failed'::job_status, 'cancelled'::job_status)
           RETURNING *"#
    )
    .bind(id)
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::BadRequest("Only failed or cancelled jobs can be retried".into()))?;

    Ok(Json(job))
}

#[utoipa::path(
    post,
    path = "/{id}/cancel",
    tag = "Admin Jobs",
    security(("bearer_auth" = []), ("csrf_token" = [])),
    params(("id" = Uuid, Path, description = "Job ID")),
    responses(
        (status = 200, description = "Job cancelled", body = Job),
        (status = 404, description = "Job not found", body = ErrorResponse)
    )
)]
pub(crate) async fn cancel_job(
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
             AND status = 'queued'::job_status
           RETURNING *"#
    )
    .bind(id)
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::BadRequest("Only queued jobs can be cancelled".into()))?;

    Ok(Json(job))
}
