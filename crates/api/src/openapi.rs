use serde::Serialize;
use utoipa::openapi::security::{ApiKey, ApiKeyValue, Http, HttpAuthScheme, SecurityScheme};
use utoipa::{Modify, OpenApi, ToSchema};

#[derive(OpenApi)]
#[openapi(
    info(
        title = "VaultScope API (VAMOS)",
        version = "0.1.0",
        description = "VaultScope backend REST API covering authentication, customer storefront portal, webhooks, and CAMOS administration."
    ),
    modifiers(&SecurityAddon)
)]
pub struct ApiDoc;

pub struct SecurityAddon;

impl Modify for SecurityAddon {
    fn modify(&self, openapi: &mut utoipa::openapi::OpenApi) {
        if let Some(components) = openapi.components.as_mut() {
            components.add_security_scheme(
                "bearer_auth",
                SecurityScheme::Http(
                    Http::builder()
                        .scheme(HttpAuthScheme::Bearer)
                        .bearer_format("JWT")
                        .build(),
                ),
            );
            components.add_security_scheme(
                "csrf_token",
                SecurityScheme::ApiKey(ApiKey::Header(ApiKeyValue::new("x-csrf-token"))),
            );
        }
    }
}

#[derive(Serialize, ToSchema)]
pub struct ErrorResponse {
    pub error: String,
}

#[derive(Serialize, ToSchema)]
pub struct OkResponse {
    pub ok: bool,
}

#[derive(Serialize, ToSchema)]
pub struct OkMessageResponse {
    pub ok: bool,
    pub message: String,
}

#[derive(Serialize, ToSchema)]
pub struct DeletedResponse {
    pub deleted: bool,
}

#[derive(Serialize, ToSchema)]
pub struct SuccessResponse {
    pub success: bool,
}

#[derive(Serialize, ToSchema)]
pub struct HealthResponse {
    pub status: String,
}

#[derive(Serialize, ToSchema)]
pub struct CsrfTokenResponse {
    pub token: String,
}

#[allow(dead_code)]
#[derive(ToSchema)]
pub struct BinaryFileUpload {
    #[schema(value_type = String, format = Binary)]
    pub file: Vec<u8>,
}
