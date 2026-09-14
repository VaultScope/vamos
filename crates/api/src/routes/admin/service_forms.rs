use axum::extract::{Path, State};
use axum::routing::get;
use axum::{Json, Router};
use serde::Deserialize;
use uuid::Uuid;

use crate::auth::AuthAdmin;
use crate::error::ApiError;
use crate::state::AppState;
use crate::validation::validate_required_string;
use vaultscope_db::models::ServiceForm;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/", get(list).post(create))
        .route("/{id}", get(get_one).put(update).delete(delete))
}

async fn list(
    auth: AuthAdmin,
    State(state): State<AppState>,
) -> Result<Json<Vec<ServiceForm>>, ApiError> {
    crate::require_permission!(auth, state, "forms.manage");
    let forms: Vec<ServiceForm> =
        sqlx::query_as("SELECT * FROM service_forms ORDER BY created_at DESC")
            .fetch_all(&state.db)
            .await?;
    Ok(Json(forms))
}

async fn get_one(
    auth: AuthAdmin,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<ServiceForm>, ApiError> {
    crate::require_permission!(auth, state, "forms.manage");
    let form: ServiceForm = sqlx::query_as("SELECT * FROM service_forms WHERE id = $1")
        .bind(id)
        .fetch_optional(&state.db)
        .await?
        .ok_or(ApiError::NotFound("service form not found".into()))?;
    Ok(Json(form))
}

#[derive(Deserialize)]
struct CreateServiceForm {
    name: String,
    description: String,
    fields: serde_json::Value,
}

async fn create(
    auth: AuthAdmin,
    State(state): State<AppState>,
    Json(payload): Json<CreateServiceForm>,
) -> Result<Json<ServiceForm>, ApiError> {
    crate::require_permission!(auth, state, "forms.manage");
    validate_required_string(&payload.name, "Name")?;

    let form: ServiceForm = sqlx::query_as(
        r#"INSERT INTO service_forms (name, description, fields)
           VALUES ($1, $2, $3)
           RETURNING *"#,
    )
    .bind(payload.name)
    .bind(payload.description)
    .bind(payload.fields)
    .fetch_one(&state.db)
    .await?;

    Ok(Json(form))
}

#[derive(Deserialize)]
struct UpdateServiceForm {
    name: Option<String>,
    description: Option<String>,
    fields: Option<serde_json::Value>,
}

async fn update(
    auth: AuthAdmin,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(payload): Json<UpdateServiceForm>,
) -> Result<Json<ServiceForm>, ApiError> {
    crate::require_permission!(auth, state, "forms.manage");
    let form: ServiceForm = sqlx::query_as(
        r#"UPDATE service_forms
           SET name = COALESCE($1, name),
               description = COALESCE($2, description),
               fields = COALESCE($3, fields),
               updated_at = now()
           WHERE id = $4
           RETURNING *"#,
    )
    .bind(payload.name)
    .bind(payload.description)
    .bind(payload.fields)
    .bind(id)
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound("service form not found".into()))?;

    Ok(Json(form))
}

async fn delete(
    auth: AuthAdmin,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<()>, ApiError> {
    crate::require_permission!(auth, state, "forms.manage");
    let result = sqlx::query("DELETE FROM service_forms WHERE id = $1")
        .bind(id)
        .execute(&state.db)
        .await?;

    if result.rows_affected() == 0 {
        return Err(ApiError::NotFound("service form not found".into()));
    }

    Ok(Json(()))
}
