use axum::extract::{Query, State};
use axum::Json;
use serde::Deserialize;
use utoipa::IntoParams;
use utoipa_axum::{router::OpenApiRouter, routes};
use uuid::Uuid;

use crate::auth::AuthAdmin;
use crate::error::ApiError;
use crate::openapi::ErrorResponse;
use crate::state::AppState;
use vaultscope_db::models::ActivityLog;

pub fn routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new().routes(routes!(list))
}

#[derive(Deserialize, IntoParams)]
pub(crate) struct ListParams {
    category: Option<String>,
    customer_id: Option<Uuid>,
    limit: Option<i64>,
}

#[utoipa::path(
    get,
    path = "/",
    tag = "Admin Activity",
    security(("bearer_auth" = [])),
    params(ListParams),
    responses(
        (status = 200, description = "Audit activity logs", body = Vec<ActivityLog>),
        (status = 403, description = "Forbidden", body = ErrorResponse)
    )
)]
pub(crate) async fn list(
    auth: AuthAdmin,
    State(state): State<AppState>,
    Query(params): Query<ListParams>,
) -> Result<Json<Vec<ActivityLog>>, ApiError> {
    if crate::middleware::rbac::check_permission(&auth, &state, "activity.view").await.is_err()
        && crate::middleware::rbac::check_permission(&auth, &state, "staff.view").await.is_err()
    {
        crate::require_permission!(auth, state, "settings.manage");
    }
    let limit = params.limit.unwrap_or(100).clamp(1, 500);

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
