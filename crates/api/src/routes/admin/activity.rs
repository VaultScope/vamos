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
    auth: AuthAdmin,
    State(state): State<AppState>,
    Query(params): Query<ListParams>,
) -> Result<Json<Vec<ActivityLog>>, ApiError> {
    if crate::middleware::rbac::check_permission(&auth, &state, "activity.view").await.is_err()
        && crate::middleware::rbac::check_permission(&auth, &state, "staff.view").await.is_err()
    {
        crate::require_permission!(auth, state, "settings.manage");
    }
    let limit = params.limit.unwrap_or(100).min(500);

    let logs = match (params.customer_id, params.category) {
        (Some(cid), Some(category)) => {
            let cid_str = cid.to_string();
            sqlx::query_as::<_, ActivityLog>(
                "SELECT * FROM activity_log WHERE (actor_id = $1 OR target = $2 OR detail ILIKE '%' || $2 || '%') AND category = $3 ORDER BY created_at DESC LIMIT $4",
            )
            .bind(cid)
            .bind(&cid_str)
            .bind(category)
            .bind(limit)
            .fetch_all(&state.db)
            .await?
        }
        (Some(cid), None) => {
            let cid_str = cid.to_string();
            sqlx::query_as::<_, ActivityLog>(
                "SELECT * FROM activity_log WHERE (actor_id = $1 OR target = $2 OR detail ILIKE '%' || $2 || '%') ORDER BY created_at DESC LIMIT $3",
            )
            .bind(cid)
            .bind(&cid_str)
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
