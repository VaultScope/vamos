use axum::extract::State;
use axum::routing::get;
use axum::{Json, Router};
use serde::Serialize;
use sqlx::FromRow;

use crate::auth::AuthCustomer;
use crate::error::ApiError;
use crate::state::AppState;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/", get(list_mine))
}

#[derive(Serialize, FromRow, Clone)]
pub struct ActivityEntry {
    pub id: uuid::Uuid,
    pub action: String,
    pub target: String,
    pub detail: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

async fn list_mine(
    auth: AuthCustomer,
    State(state): State<AppState>,
) -> Result<Json<Vec<ActivityEntry>>, ApiError> {
    let customer_id = auth.0.sub;
    let customer_id_str = customer_id.to_string();
    let email_pattern = format!("%{}%", auth.0.email);
    let id_pattern = format!("%{}%", customer_id_str);

    let mut entries: Vec<ActivityEntry> = sqlx::query_as(
        r#"SELECT id, action, target, detail, created_at FROM activity_log
           WHERE (actor_id = $1 AND actor_type = 'customer'::actor_type)
              OR target = $2
              OR target ILIKE $3
              OR detail ILIKE $3
              OR detail ILIKE $4
           ORDER BY created_at DESC LIMIT 100"#,
    )
    .bind(customer_id)
    .bind(&customer_id_str)
    .bind(&email_pattern)
    .bind(&id_pattern)
    .fetch_all(&state.db)
    .await?;

    if entries.is_empty() {
        // Fallback: provide initial account creation activity if no events logged yet
        let customer_created_at: Option<chrono::DateTime<chrono::Utc>> = sqlx::query_scalar(
            "SELECT created_at FROM customers WHERE id = $1",
        )
        .bind(customer_id)
        .fetch_optional(&state.db)
        .await
        .ok()
        .flatten();

        if let Some(created_at) = customer_created_at {
            entries.push(ActivityEntry {
                id: customer_id,
                action: "Account Initialized".to_string(),
                target: "VaultScope Client Portal".to_string(),
                detail: format!("Account established for {}", auth.0.email),
                created_at,
            });
        }
    }

    Ok(Json(entries))
}
