use axum::extract::{Query, State};
use axum::routing::get;
use axum::{Json, Router};
use serde::Deserialize;
use uuid::Uuid;

use crate::auth::AuthAdmin;
use crate::error::ApiError;
use crate::state::AppState;
use vaultscope_db::models::ActivityLog;

pub fn routes() -> Router<AppState> {
    Router::new().route("/", get(list))
}

#[derive(Deserialize)]
struct ListParams {
    category: Option<String>,
    customer_id: Option<Uuid>,
    limit: Option<i64>,
}

async fn list(
    _auth: AuthAdmin,
    State(state): State<AppState>,
    Query(params): Query<ListParams>,
) -> Result<Json<Vec<ActivityLog>>, ApiError> {
    let limit = params.limit.unwrap_or(100).min(500);

    let logs = match (params.customer_id, params.category) {
        (Some(cid), Some(category)) => {
            sqlx::query_as::<_, ActivityLog>(
                "SELECT * FROM activity_log WHERE actor_id = $1 AND category = $2 ORDER BY created_at DESC LIMIT $3",
            )
            .bind(cid)
            .bind(category)
            .bind(limit)
            .fetch_all(&state.db)
            .await?
        }
        (Some(cid), None) => {
            sqlx::query_as::<_, ActivityLog>(
                "SELECT * FROM activity_log WHERE actor_id = $1 ORDER BY created_at DESC LIMIT $2",
            )
            .bind(cid)
            .bind(limit)
            .fetch_all(&state.db)
            .await?
        }
        (None, Some(category)) => {
            sqlx::query_as::<_, ActivityLog>(
                "SELECT * FROM activity_log WHERE category = $1 ORDER BY created_at DESC LIMIT $2",
            )
            .bind(category)
            .bind(limit)
            .fetch_all(&state.db)
            .await?
        }
        (None, None) => {
            sqlx::query_as::<_, ActivityLog>(
                "SELECT * FROM activity_log ORDER BY created_at DESC LIMIT $1",
            )
            .bind(limit)
            .fetch_all(&state.db)
            .await?
        }
    };

    Ok(Json(logs))
}
