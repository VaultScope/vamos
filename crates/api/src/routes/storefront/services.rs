use axum::extract::{Path, State};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::auth::AuthCustomer;
use crate::error::ApiError;
use crate::state::AppState;
use vaultscope_common::types::PowerAction;
use vaultscope_db::models::Service;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/", get(list_mine))
        .route("/{id}", get(get_one).delete(delete_service))
        .route("/{id}/start", post(power_start))
        .route("/{id}/stop", post(power_stop))
        .route("/{id}/restart", post(power_restart))
        .route("/{id}/reinstall", post(reinstall))
}

async fn list_mine(
    auth: AuthCustomer,
    State(state): State<AppState>,
) -> Result<Json<Vec<Service>>, ApiError> {
    let services: Vec<Service> =
        sqlx::query_as("SELECT * FROM services WHERE customer_id = $1 ORDER BY created_at DESC")
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
    let service: Service =
        sqlx::query_as("SELECT * FROM services WHERE id = $1 AND customer_id = $2")
            .bind(id)
            .bind(auth.0.sub)
            .fetch_optional(&state.db)
            .await?
            .ok_or(ApiError::NotFound("service not found".into()))?;
    Ok(Json(service))
}

async fn power_start(
    auth: AuthCustomer,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, ApiError> {
    power_action_helper(auth, state, id, PowerAction::Start).await
}

async fn power_stop(
    auth: AuthCustomer,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, ApiError> {
    power_action_helper(auth, state, id, PowerAction::Stop).await
}

async fn power_restart(
    auth: AuthCustomer,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, ApiError> {
    power_action_helper(auth, state, id, PowerAction::Restart).await
}

async fn power_action_helper(
    auth: AuthCustomer,
    state: AppState,
    service_id: Uuid,
    action: PowerAction,
) -> Result<Json<Value>, ApiError> {
    let service: Service =
        sqlx::query_as("SELECT * FROM services WHERE id = $1 AND customer_id = $2")
            .bind(service_id)
            .bind(auth.0.sub)
            .fetch_optional(&state.db)
            .await?
            .ok_or(ApiError::NotFound("service not found".into()))?;

    if service.provider_resource_id.is_empty() {
        return Err(ApiError::BadRequest("service not yet provisioned".into()));
    }

    let connector_id = service
        .connector_id
        .ok_or(ApiError::Internal("no connector".into()))?;

    let job_id = Uuid::now_v7();
    let payload = json!({
        "resource_id": service.provider_resource_id,
        "action": action
    });

    sqlx::query(
        "INSERT INTO jobs (id, task, target_api, connector_id, customer_id, service_id, request_payload) VALUES ($1, 'power_action', $2, $3, $4, $5, $6)"
    )
    .bind(job_id)
    .bind(service.config.get("provider").and_then(|v| v.as_str()).unwrap_or("hetzner_cloud"))
    .bind(connector_id)
    .bind(auth.0.sub)
    .bind(service_id)
    .bind(&payload)
    .execute(&state.db)
    .await?;

    Ok(Json(json!({ "job_id": job_id, "status": "queued" })))
}

#[derive(Deserialize)]
struct ReinstallPayload {
    os: String,
}

async fn reinstall(
    auth: AuthCustomer,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(payload): Json<ReinstallPayload>,
) -> Result<Json<Value>, ApiError> {
    let service: Service =
        sqlx::query_as("SELECT * FROM services WHERE id = $1 AND customer_id = $2")
            .bind(id)
            .bind(auth.0.sub)
            .fetch_optional(&state.db)
            .await?
            .ok_or(ApiError::NotFound("service not found".into()))?;

    if service.provider_resource_id.is_empty() {
        return Err(ApiError::BadRequest("service not yet provisioned".into()));
    }

    let connector_id = service
        .connector_id
        .ok_or(ApiError::Internal("no connector".into()))?;

    let job_id = Uuid::now_v7();
    let reinstall_payload = json!({
        "resource_id": service.provider_resource_id,
        "params": {
            "image": payload.os
        }
    });

    sqlx::query(
        "INSERT INTO jobs (id, task, target_api, connector_id, customer_id, service_id, request_payload) VALUES ($1, 'reinstall', $2, $3, $4, $5, $6)"
    )
    .bind(job_id)
    .bind(service.config.get("provider").and_then(|v| v.as_str()).unwrap_or("hetzner_cloud"))
    .bind(connector_id)
    .bind(auth.0.sub)
    .bind(id)
    .bind(&reinstall_payload)
    .execute(&state.db)
    .await?;

    Ok(Json(json!({ "job_id": job_id, "status": "queued" })))
}

async fn delete_service(
    auth: AuthCustomer,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, ApiError> {
    let service: Service =
        sqlx::query_as("SELECT * FROM services WHERE id = $1 AND customer_id = $2")
            .bind(id)
            .bind(auth.0.sub)
            .fetch_optional(&state.db)
            .await?
            .ok_or(ApiError::NotFound("service not found".into()))?;

    if service.status == vaultscope_db::models::ServiceStatus::Running {
        return Err(ApiError::BadRequest(
            "Cannot delete a running service. Please stop or suspend it first.".into(),
        ));
    }

    if !service.provider_resource_id.is_empty() {
        let connector_id = service
            .connector_id
            .ok_or(ApiError::Internal("no connector".into()))?;

        let job_id = Uuid::now_v7();
        let delete_payload = json!({
            "resource_id": service.provider_resource_id
        });

        sqlx::query(
            "INSERT INTO jobs (id, task, target_api, connector_id, customer_id, service_id, request_payload) VALUES ($1, 'delete', $2, $3, $4, $5, $6)"
        )
        .bind(job_id)
        .bind(service.config.get("provider").and_then(|v| v.as_str()).unwrap_or("hetzner_cloud"))
        .bind(connector_id)
        .bind(auth.0.sub)
        .bind(id)
        .bind(&delete_payload)
        .execute(&state.db)
        .await?;
    }

    sqlx::query("UPDATE services SET status = 'terminated' WHERE id = $1")
        .bind(id)
        .execute(&state.db)
        .await?;

    Ok(Json(json!({ "ok": true })))
}
