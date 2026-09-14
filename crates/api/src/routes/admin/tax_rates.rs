use axum::extract::{Path, State};
use axum::routing::get;
use axum::{Json, Router};
use serde::Deserialize;
use uuid::Uuid;

use crate::auth::AuthAdmin;
use crate::error::ApiError;
use crate::state::AppState;
use crate::validation::{
    validate_percentage, validate_required_string, validate_string_length, MAX_SHORT_STRING_LENGTH,
};
use vaultscope_db::models::TaxRate;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/", get(list).post(create))
        .route("/{id}", get(get_one).put(update).delete(delete))
}

async fn list(
    auth: AuthAdmin,
    State(state): State<AppState>,
) -> Result<Json<Vec<TaxRate>>, ApiError> {
    crate::require_permission!(auth, state, "tax.manage");
    let rates: Vec<TaxRate> = sqlx::query_as("SELECT * FROM tax_rates ORDER BY name")
        .fetch_all(&state.db)
        .await?;
    Ok(Json(rates))
}

async fn get_one(
    auth: AuthAdmin,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<TaxRate>, ApiError> {
    crate::require_permission!(auth, state, "tax.manage");
    let rate: TaxRate = sqlx::query_as("SELECT * FROM tax_rates WHERE id = $1")
        .bind(id)
        .fetch_optional(&state.db)
        .await?
        .ok_or(ApiError::NotFound("tax rate not found".into()))?;
    Ok(Json(rate))
}

#[derive(Deserialize)]
struct CreateTaxRate {
    name: String,
    country: String,
    rate: f64,
}

async fn create(
    auth: AuthAdmin,
    State(state): State<AppState>,
    Json(payload): Json<CreateTaxRate>,
) -> Result<Json<TaxRate>, ApiError> {
    crate::require_permission!(auth, state, "tax.manage");
    // Validate inputs
    validate_required_string(&payload.name, "Name")?;
    validate_string_length(&payload.name, "Name", MAX_SHORT_STRING_LENGTH)?;

    validate_required_string(&payload.country, "Country")?;
    validate_string_length(&payload.country, "Country", MAX_SHORT_STRING_LENGTH)?;

    validate_percentage(payload.rate, "Tax rate")?;

    let tax_rate: TaxRate = sqlx::query_as(
        r#"INSERT INTO tax_rates (name, country, rate)
           VALUES ($1, $2, $3)
           RETURNING *"#,
    )
    .bind(&payload.name)
    .bind(&payload.country)
    .bind(rust_decimal::Decimal::from_f64_retain(payload.rate).unwrap_or_default())
    .fetch_one(&state.db)
    .await?;

    Ok(Json(tax_rate))
}

#[derive(Deserialize)]
struct UpdateTaxRate {
    name: Option<String>,
    country: Option<String>,
    rate: Option<f64>,
}

async fn update(
    auth: AuthAdmin,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(payload): Json<UpdateTaxRate>,
) -> Result<Json<TaxRate>, ApiError> {
    crate::require_permission!(auth, state, "tax.manage");
    let tax_rate: TaxRate = sqlx::query_as(
        r#"UPDATE tax_rates
           SET name = COALESCE($1, name),
               country = COALESCE($2, country),
               rate = COALESCE($3, rate)
           WHERE id = $4
           RETURNING *"#,
    )
    .bind(payload.name)
    .bind(payload.country)
    .bind(
        payload
            .rate
            .map(|v| rust_decimal::Decimal::from_f64_retain(v).unwrap_or_default()),
    )
    .bind(id)
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound("tax rate not found".into()))?;

    Ok(Json(tax_rate))
}

async fn delete(
    auth: AuthAdmin,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<()>, ApiError> {
    crate::require_permission!(auth, state, "tax.manage");
    let result = sqlx::query("DELETE FROM tax_rates WHERE id = $1")
        .bind(id)
        .execute(&state.db)
        .await?;

    if result.rows_affected() == 0 {
        return Err(ApiError::NotFound("tax rate not found".into()));
    }

    Ok(Json(()))
}
