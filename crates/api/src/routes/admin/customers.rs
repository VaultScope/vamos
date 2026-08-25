use axum::extract::{Path, Query, State};
use axum::routing::get;
use axum::{Json, Router};
use serde::Deserialize;
use uuid::Uuid;

use crate::auth::AuthAdmin;
use crate::error::ApiError;
use crate::state::AppState;
use vaultscope_db::models::{Customer, CustomerStatus};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/", get(list))
        .route("/{id}", get(get_one).put(update))
}

#[derive(Deserialize)]
struct ListParams {
    status: Option<String>,
    search: Option<String>,
    limit: Option<i64>,
    offset: Option<i64>,
}

async fn list(
    _auth: AuthAdmin,
    State(state): State<AppState>,
    Query(params): Query<ListParams>,
) -> Result<Json<Vec<Customer>>, ApiError> {
    let limit = params.limit.unwrap_or(50).min(200);
    let offset = params.offset.unwrap_or(0);

    let customers: Vec<Customer> = sqlx::query_as(
        "SELECT * FROM customers ORDER BY created_at DESC LIMIT $1 OFFSET $2"
    )
    .bind(limit)
    .bind(offset)
    .fetch_all(&state.db)
    .await?;

    Ok(Json(customers))
}

async fn get_one(
    _auth: AuthAdmin,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<Customer>, ApiError> {
    let customer: Customer = sqlx::query_as("SELECT * FROM customers WHERE id = $1")
        .bind(id)
        .fetch_optional(&state.db)
        .await?
        .ok_or(ApiError::NotFound("customer not found".into()))?;
    Ok(Json(customer))
}

#[derive(Deserialize)]
struct UpdateCustomer {
    status: Option<CustomerStatus>,
    notes: Option<String>,
}

async fn update(
    _auth: AuthAdmin,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(payload): Json<UpdateCustomer>,
) -> Result<Json<Customer>, ApiError> {
    let customer: Customer = sqlx::query_as(
        r#"UPDATE customers
           SET status = COALESCE($1::customer_status, status),
               notes = COALESCE($2, notes),
               updated_at = now()
           WHERE id = $3
           RETURNING *"#
    )
    .bind(payload.status)
    .bind(payload.notes)
    .bind(id)
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound("customer not found".into()))?;

    Ok(Json(customer))
}
