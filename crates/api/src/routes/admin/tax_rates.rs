use axum::extract::{Path, State};
use axum::Json;
use serde::Deserialize;
use utoipa::ToSchema;
use utoipa_axum::{router::OpenApiRouter, routes};
use uuid::Uuid;

use crate::auth::AuthAdmin;
use crate::error::ApiError;
use crate::openapi::ErrorResponse;
use crate::state::AppState;
use crate::validation::{
    validate_percentage, validate_required_string, validate_string_length, MAX_SHORT_STRING_LENGTH,
};
use vaultscope_db::models::TaxRate;

pub fn routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(list, create))
        .routes(routes!(get_one, update, delete))
}

#[utoipa::path(
    get,
    path = "/",
    tag = "Admin Tax Rates",
    security(("bearer_auth" = [])),
    responses((status = 200, description = "List tax rates", body = Vec<TaxRate>))
)]
pub(crate) async fn list(
    auth: AuthAdmin,
    State(state): State<AppState>,
) -> Result<Json<Vec<TaxRate>>, ApiError> {
    crate::require_permission!(auth, state, "tax.manage");
    let rates: Vec<TaxRate> = sqlx::query_as("SELECT * FROM tax_rates ORDER BY name")
        .fetch_all(&state.db)
        .await?;
    Ok(Json(rates))
}

#[utoipa::path(
    get,
    path = "/{id}",
    tag = "Admin Tax Rates",
    security(("bearer_auth" = [])),
    params(("id" = Uuid, Path, description = "Tax rate ID")),
    responses(
        (status = 200, description = "Tax rate details", body = TaxRate),
        (status = 404, description = "Tax rate not found", body = ErrorResponse)
    )
)]
pub(crate) async fn get_one(
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

#[derive(Deserialize, ToSchema)]
pub(crate) struct CreateTaxRate {
    name: String,
    country: String,
    rate: f64,
}

#[utoipa::path(
    post,
    path = "/",
    tag = "Admin Tax Rates",
    security(("bearer_auth" = []), ("csrf_token" = [])),
    request_body = CreateTaxRate,
    responses(
        (status = 200, description = "Tax rate created", body = TaxRate),
        (status = 400, description = "Invalid input", body = ErrorResponse)
    )
)]
pub(crate) async fn create(
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

#[derive(Deserialize, ToSchema)]
pub(crate) struct UpdateTaxRate {
    name: Option<String>,
    country: Option<String>,
    rate: Option<f64>,
}

#[utoipa::path(
    put,
    path = "/{id}",
    tag = "Admin Tax Rates",
    security(("bearer_auth" = []), ("csrf_token" = [])),
    params(("id" = Uuid, Path, description = "Tax rate ID")),
    request_body = UpdateTaxRate,
    responses(
        (status = 200, description = "Tax rate updated", body = TaxRate),
        (status = 404, description = "Tax rate not found", body = ErrorResponse)
    )
)]
pub(crate) async fn update(
    auth: AuthAdmin,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(payload): Json<UpdateTaxRate>,
) -> Result<Json<TaxRate>, ApiError> {
    crate::require_permission!(auth, state, "tax.manage");
    if let Some(ref name) = payload.name {
        validate_required_string(name, "Name")?;
        validate_string_length(name, "Name", MAX_SHORT_STRING_LENGTH)?;
    }
    if let Some(ref country) = payload.country {
        validate_required_string(country, "Country")?;
        validate_string_length(country, "Country", MAX_SHORT_STRING_LENGTH)?;
    }
    if let Some(rate) = payload.rate {
        validate_percentage(rate, "Tax rate")?;
    }
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

#[utoipa::path(
    delete,
    path = "/{id}",
    tag = "Admin Tax Rates",
    security(("bearer_auth" = []), ("csrf_token" = [])),
    params(("id" = Uuid, Path, description = "Tax rate ID")),
    responses(
        (status = 200, description = "Tax rate deleted"),
        (status = 404, description = "Tax rate not found", body = ErrorResponse)
    )
)]
pub(crate) async fn delete(
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
