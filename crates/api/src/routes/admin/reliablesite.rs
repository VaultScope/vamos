use axum::extract::{Path, Query, State};
use axum::Json;
use serde::Deserialize;
use utoipa::{IntoParams, ToSchema};
use utoipa_axum::{router::OpenApiRouter, routes};
use vaultscope_connectors::reliablesite::{PaymentMethodsResponse, ResellerPromoCodesResponse};
use vaultscope_db::models::ReliableSiteServer;

use crate::auth::AuthAdmin;
use crate::error::ApiError;
use crate::openapi::ErrorResponse;
use crate::reliablesite_catalog::{
    self, ReliableSiteConfigGroupView, SyncSummary,
};
use crate::state::AppState;

pub fn routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(list_servers))
        .routes(routes!(get_server_options))
        .routes(routes!(sync_inventory))
        .routes(routes!(get_payment_methods))
        .routes(routes!(get_promo_codes))
}

#[derive(Deserialize, IntoParams)]
pub(crate) struct ListServersQuery {
    data_center: Option<String>,
}

#[utoipa::path(
    get,
    path = "/servers",
    tag = "Admin ReliableSite",
    security(("bearer_auth" = [])),
    params(ListServersQuery),
    responses((status = 200, description = "List synced ReliableSite servers", body = Vec<ReliableSiteServer>))
)]
pub(crate) async fn list_servers(
    auth: AuthAdmin,
    State(state): State<AppState>,
    Query(query): Query<ListServersQuery>,
) -> Result<Json<Vec<ReliableSiteServer>>, ApiError> {
    crate::require_permission!(auth, state, "products.view");

    let servers: Vec<ReliableSiteServer> = if let Some(dc) = query.data_center.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        sqlx::query_as(
            "SELECT * FROM reliablesite_servers WHERE UPPER(data_center) = UPPER($1) ORDER BY product_id ASC",
        )
        .bind(dc)
        .fetch_all(&state.db)
        .await?
    } else {
        sqlx::query_as("SELECT * FROM reliablesite_servers ORDER BY data_center ASC, product_id ASC")
            .fetch_all(&state.db)
            .await?
    };

    Ok(Json(servers))
}

#[utoipa::path(
    get,
    path = "/servers/{product_id}/options",
    tag = "Admin ReliableSite",
    security(("bearer_auth" = [])),
    params(("product_id" = i32, Path, description = "ReliableSite Inventory Product_Id")),
    responses(
        (status = 200, description = "Configuration groups and addon choices for a ReliableSite server", body = Vec<ReliableSiteConfigGroupView>),
        (status = 404, description = "ReliableSite server not found", body = ErrorResponse)
    )
)]
pub(crate) async fn get_server_options(
    auth: AuthAdmin,
    State(state): State<AppState>,
    Path(product_id): Path<i32>,
) -> Result<Json<Vec<ReliableSiteConfigGroupView>>, ApiError> {
    crate::require_permission!(auth, state, "products.view");

    let exists: Option<i32> = sqlx::query_scalar(
        "SELECT product_id FROM reliablesite_servers WHERE product_id = $1",
    )
    .bind(product_id)
    .fetch_optional(&state.db)
    .await?;

    if exists.is_none() {
        return Err(ApiError::NotFound(format!(
            "ReliableSite server {} not found",
            product_id
        )));
    }

    let groups = reliablesite_catalog::load_server_groups_with_addons(&state.db, product_id).await?;
    Ok(Json(groups))
}

#[derive(Deserialize, Default, ToSchema)]
pub(crate) struct SyncInventoryPayload {
    data_center: Option<String>,
}

#[utoipa::path(
    post,
    path = "/sync",
    tag = "Admin ReliableSite",
    security(("bearer_auth" = []), ("csrf_token" = [])),
    request_body = Option<SyncInventoryPayload>,
    responses(
        (status = 200, description = "Synchronized ReliableSite Inventory SOAP catalog into database", body = SyncSummary),
        (status = 500, description = "Inventory SOAP sync failed", body = ErrorResponse)
    )
)]
pub(crate) async fn sync_inventory(
    auth: AuthAdmin,
    State(state): State<AppState>,
    payload: Option<Json<SyncInventoryPayload>>,
) -> Result<Json<SyncSummary>, ApiError> {
    crate::require_permission!(auth, state, "products.manage");

    let dc = payload
        .and_then(|p| p.0.data_center)
        .map(|s| s.trim().to_uppercase())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "NL".to_string());

    let rs = state.config.reliablesite_connector();
    let summary = reliablesite_catalog::sync_datacenter_to_db(&state.db, &rs, &dc).await?;
    Ok(Json(summary))
}

#[derive(Deserialize, IntoParams)]
pub(crate) struct PaymentMethodsQuery {
    product_id: i32,
}

#[utoipa::path(
    get,
    path = "/payment-methods",
    tag = "Admin ReliableSite",
    security(("bearer_auth" = [])),
    params(PaymentMethodsQuery),
    responses(
        (status = 200, description = "Available ReliableSite Order API payment methods", body = PaymentMethodsResponse),
        (status = 400, description = "API key not configured or upstream error", body = ErrorResponse)
    )
)]
pub(crate) async fn get_payment_methods(
    auth: AuthAdmin,
    State(state): State<AppState>,
    Query(query): Query<PaymentMethodsQuery>,
) -> Result<Json<PaymentMethodsResponse>, ApiError> {
    crate::require_permission!(auth, state, "products.view");

    let rs = state.config.reliablesite_connector();
    if rs.is_mock() {
        return Err(ApiError::BadRequest(
            "RELIABLESITE_API_KEY is not configured on the server".into(),
        ));
    }

    let methods = rs
        .order_api()
        .get_payment_methods(query.product_id)
        .await
        .map_err(|e| ApiError::Internal(format!("ReliableSite PaymentMethods failed: {e}")))?;
    Ok(Json(methods))
}

#[derive(Deserialize, IntoParams)]
pub(crate) struct PromoCodesQuery {
    configuration_id: Option<i32>,
}

#[utoipa::path(
    get,
    path = "/promo-codes",
    tag = "Admin ReliableSite",
    security(("bearer_auth" = [])),
    params(PromoCodesQuery),
    responses(
        (status = 200, description = "Active ReliableSite reseller promo codes", body = ResellerPromoCodesResponse),
        (status = 400, description = "API key not configured or upstream error", body = ErrorResponse)
    )
)]
pub(crate) async fn get_promo_codes(
    auth: AuthAdmin,
    State(state): State<AppState>,
    Query(query): Query<PromoCodesQuery>,
) -> Result<Json<ResellerPromoCodesResponse>, ApiError> {
    crate::require_permission!(auth, state, "products.view");

    let rs = state.config.reliablesite_connector();
    if rs.is_mock() {
        return Err(ApiError::BadRequest(
            "RELIABLESITE_API_KEY is not configured on the server".into(),
        ));
    }

    let promos = rs
        .order_api()
        .get_reseller_promo_codes(query.configuration_id)
        .await
        .map_err(|e| ApiError::Internal(format!("ReliableSite Reseller PromoCodes failed: {e}")))?;
    Ok(Json(promos))
}
