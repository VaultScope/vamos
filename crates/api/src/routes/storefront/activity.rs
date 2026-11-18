use axum::extract::{Query, State};
use axum::routing::get;
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sqlx::FromRow;

use crate::auth::AuthCustomer;
use crate::error::ApiError;
use crate::state::AppState;

const MAX_PER_PAGE: i64 = 100;
const DEFAULT_PER_PAGE: i64 = 20;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/", get(list_mine))
}

#[derive(Deserialize, Default)]
pub struct ListActivityParams {
    pub search: Option<String>,
    pub category: Option<String>,
    pub action: Option<String>,
    pub page: Option<i64>,
    pub per_page: Option<i64>,
    pub limit: Option<i64>,
    pub paginate: Option<bool>,
}

#[derive(Serialize, FromRow, Clone)]
pub struct ActivityEntry {
    pub id: uuid::Uuid,
    pub category: String,
    pub action: String,
    pub target: String,
    pub detail: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

async fn list_mine(
    auth: AuthCustomer,
    State(state): State<AppState>,
    Query(params): Query<ListActivityParams>,
) -> Result<Json<Value>, ApiError> {
    let customer_id = auth.0.sub;
    let customer_id_str = customer_id.to_string();
    let email_pattern = format!("%{}%", auth.0.email);
    let id_pattern = format!("%{}%", customer_id_str);

    let mut entries: Vec<ActivityEntry> = sqlx::query_as(
        r#"SELECT id, category, action, target, detail, created_at FROM activity_log
           WHERE (actor_id = $1 AND actor_type = 'customer'::actor_type)
              OR target = $2
              OR target ILIKE $3
              OR detail ILIKE $3
              OR detail ILIKE $4
           ORDER BY created_at DESC LIMIT 1000"#,
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
                category: "account".to_string(),
                action: "Account Initialized".to_string(),
                target: "VaultScope Client Portal".to_string(),
                detail: format!("Account established for {}", auth.0.email),
                created_at,
            });
        }
    }

    let search_q = params
        .search
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_lowercase);
    let cat_filter = params
        .category
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty() && !s.eq_ignore_ascii_case("all"))
        .map(str::to_lowercase);
    let action_filter = params
        .action
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty() && !s.eq_ignore_ascii_case("all"))
        .map(str::to_lowercase);

    let filtered: Vec<ActivityEntry> = entries
        .into_iter()
        .filter(|e| {
            if let Some(ref cat) = cat_filter {
                if e.category.to_lowercase() != *cat {
                    return false;
                }
            }
            if let Some(ref act) = action_filter {
                if !e.action.to_lowercase().contains(act) {
                    return false;
                }
            }
            if let Some(ref q) = search_q {
                let matches = e.action.to_lowercase().contains(q)
                    || e.target.to_lowercase().contains(q)
                    || e.detail.to_lowercase().contains(q)
                    || e.category.to_lowercase().contains(q);
                if !matches {
                    return false;
                }
            }
            true
        })
        .collect();

    let is_paginated = params.paginate == Some(true) || params.page.is_some();
    let per_page = params
        .per_page
        .or(params.limit)
        .unwrap_or(if is_paginated {
            DEFAULT_PER_PAGE
        } else {
            MAX_PER_PAGE
        })
        .clamp(1, MAX_PER_PAGE);

    if is_paginated {
        let total = filtered.len() as i64;
        let total_pages = ((total + per_page - 1) / per_page).max(1);
        let page = params.page.unwrap_or(1).clamp(1, total_pages);
        let start = ((page - 1) * per_page) as usize;
        let end = (start + per_page as usize).min(filtered.len());
        let items = if start < filtered.len() {
            filtered[start..end].to_vec()
        } else {
            Vec::new()
        };

        Ok(Json(json!({
            "items": items,
            "total": total,
            "page": page,
            "per_page": per_page,
            "total_pages": total_pages,
            "max_per_page": MAX_PER_PAGE,
        })))
    } else {
        let limited: Vec<ActivityEntry> = filtered.into_iter().take(per_page as usize).collect();
        Ok(Json(json!(limited)))
    }
}
