use axum::Json;
use utoipa_axum::{router::OpenApiRouter, routes};

use crate::openapi::HealthResponse;
use crate::state::AppState;

pub fn routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new().routes(routes!(health_check))
}

#[utoipa::path(
    get,
    path = "/",
    tag = "Health",
    responses(
        (status = 200, description = "Service health status", body = HealthResponse)
    )
)]
pub(crate) async fn health_check() -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok".to_string(),
    })
}
