use axum::routing::get;
use axum::{Json, Router};
use serde_json::{json, Value};

use crate::state::AppState;

pub fn routes() -> Router<AppState> {
    Router::new().route("/", get(health_check))
}

async fn health_check() -> Json<Value> {
    Json(json!({ "status": "ok" }))
}
