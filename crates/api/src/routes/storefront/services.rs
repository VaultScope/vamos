use axum::extract::{Path, State};
use axum::routing::get;
use axum::{Json, Router};
use uuid::Uuid;

use crate::auth::AuthCustomer;
use crate::error::ApiError;
use crate::state::AppState;
use vaultscope_db::models::Service;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/", get(list_mine))
        .route("/{id}", get(get_one))
}

async fn list_mine(
    auth: AuthCustomer,
    State(state): State<AppState>,
) -> Result<Json<Vec<Service>>, ApiError> {
    let services: Vec<Service> = sqlx::query_as(
        "SELECT * FROM services WHERE customer_id = $1 ORDER BY created_at DESC"
    )
    .bind(auth.0.sub)
    .fetch_all(&state.db)
    .await?;
    Ok(Json(services))
}

async fn get_one(
    auth: AuthCustomer,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<Service>, ApiError> {
    let service: Service = sqlx::query_as(
        "SELECT * FROM services WHERE id = $1 AND customer_id = $2"
    )
    .bind(id)
    .bind(auth.0.sub)
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound("service not found".into()))?;
    Ok(Json(service))
}
