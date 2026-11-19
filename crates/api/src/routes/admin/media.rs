use axum::extract::{DefaultBodyLimit, Multipart, State};
use axum::Json;
use utoipa_axum::{router::OpenApiRouter, routes};

use crate::auth::AuthAdmin;
use crate::error::ApiError;
use crate::media::{save_optimized_image, ProcessedImage};
use crate::openapi::{BinaryFileUpload, ErrorResponse};
use crate::state::AppState;

pub fn routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(upload_media))
        .layer(DefaultBodyLimit::max(20 * 1024 * 1024))
}

#[utoipa::path(
    post,
    path = "/upload",
    tag = "Admin Media",
    security(("bearer_auth" = []), ("csrf_token" = [])),
    request_body(content = BinaryFileUpload, content_type = "multipart/form-data"),
    responses(
        (status = 200, description = "Optimized uploaded media metadata", body = ProcessedImage),
        (status = 400, description = "Missing or invalid image file", body = ErrorResponse),
        (status = 403, description = "Insufficient permissions", body = ErrorResponse)
    )
)]
pub(crate) async fn upload_media(
    auth: AuthAdmin,
    State(state): State<AppState>,
    mut multipart: Multipart,
) -> Result<Json<ProcessedImage>, ApiError> {
    let mut authorized = false;
    for perm in [
        "media.manage",
        "blog.write",
        "blog.edit",
        "products.create",
        "products.edit",
    ] {
        match crate::middleware::rbac::check_permission(&auth, &state, perm).await {
            Ok(()) => {
                authorized = true;
                break;
            }
            Err(ApiError::Forbidden) => continue,
            Err(e) => return Err(e),
        }
    }
    if !authorized {
        return Err(ApiError::Forbidden);
    }

    let mut file_data: Option<Vec<u8>> = None;
    let mut subfolder = "media".to_string();

    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| ApiError::BadRequest(e.to_string()))?
    {
        let name = field.name().unwrap_or("").to_string();
        if name == "subfolder" {
            if let Ok(text) = field.text().await {
                let trimmed = text.trim();
                if ["blog", "media", "avatars", "products"].contains(&trimmed) {
                    subfolder = trimmed.to_string();
                }
            }
        } else if name == "file" || name == "image" || name == "upload" {
            let bytes = field
                .bytes()
                .await
                .map_err(|e| ApiError::BadRequest(e.to_string()))?;
            file_data = Some(bytes.to_vec());
        }
    }

    let raw_bytes = file_data.ok_or_else(|| ApiError::BadRequest("No image file provided in multipart payload".into()))?;

    // Max 2048px for general media/blog images
    let processed = save_optimized_image(&raw_bytes, &subfolder, 2048).await?;

    Ok(Json(processed))
}
