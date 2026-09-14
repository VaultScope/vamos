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
    auth: AuthAdmin,
    State(state): State<AppState>,
    Query(params): Query<ListParams>,
) -> Result<Json<Vec<CustomerWithStats>>, ApiError> {
    crate::require_permission!(auth, state, "customers.view");
    let limit = params.limit.unwrap_or(50).min(200);
    let offset = params.offset.unwrap_or(0);

    let customers: Vec<Customer> = match (&params.status, &params.search) {
        (Some(status), Some(search)) => {
            let pattern = format!("%{}%", search);
            sqlx::query_as(
                "SELECT * FROM customers WHERE status = $1::customer_status AND (name ILIKE $2 OR email ILIKE $2) ORDER BY created_at DESC LIMIT $3 OFFSET $4"
            )
            .bind(status)
            .bind(&pattern)
            .bind(limit)
            .bind(offset)
            .fetch_all(&state.db)
            .await?
        }
        (Some(status), None) => {
            sqlx::query_as(
                "SELECT * FROM customers WHERE status = $1::customer_status ORDER BY created_at DESC LIMIT $2 OFFSET $3"
            )
            .bind(status)
            .bind(limit)
            .bind(offset)
            .fetch_all(&state.db)
            .await?
        }
        (None, Some(search)) => {
            let pattern = format!("%{}%", search);
            sqlx::query_as(
                "SELECT * FROM customers WHERE name ILIKE $1 OR email ILIKE $1 ORDER BY created_at DESC LIMIT $2 OFFSET $3"
            )
            .bind(&pattern)
            .bind(limit)
            .bind(offset)
            .fetch_all(&state.db)
            .await?
        }
        (None, None) => {
            sqlx::query_as("SELECT * FROM customers ORDER BY created_at DESC LIMIT $1 OFFSET $2")
                .bind(limit)
                .bind(offset)
                .fetch_all(&state.db)
                .await?
        }
    };

    let mut with_stats = Vec::new();
    for c in customers {
        let count: (i64,) = sqlx::query_as("SELECT count(*) FROM services WHERE customer_id = $1 AND status IN ('running', 'pending')")
            .bind(c.id)
            .fetch_one(&state.db).await.unwrap_or((0,));
        with_stats.push(CustomerWithStats {
            customer: c,
            active_services: count.0,
        });
    }

    Ok(Json(with_stats))
}

async fn get_one(
    auth: AuthAdmin,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<Customer>, ApiError> {
    crate::require_permission!(auth, state, "customers.view");
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
    auth: AuthAdmin,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(payload): Json<UpdateCustomer>,
) -> Result<Json<Customer>, ApiError> {
    crate::require_permission!(auth, state, "customers.edit");
    let customer: Customer = sqlx::query_as(
        r#"UPDATE customers
           SET status = COALESCE($1::customer_status, status),
               notes = COALESCE($2, notes),
               updated_at = now()
           WHERE id = $3
           RETURNING *"#,
    )
    .bind(payload.status)
    .bind(payload.notes)
    .bind(id)
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound("customer not found".into()))?;

    Ok(Json(customer))
}

#[derive(serde::Serialize)]
pub struct CustomerWithStats {
    #[serde(flatten)]
    pub customer: Customer,
    pub active_services: i64,
}
