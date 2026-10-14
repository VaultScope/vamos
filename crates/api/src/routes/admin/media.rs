use axum::extract::{DefaultBodyLimit, Multipart, State};
use axum::routing::post;
use axum::{Json, Router};

use crate::auth::AuthAdmin;
use crate::error::ApiError;
use crate::media::{save_optimized_image, ProcessedImage};
use crate::state::AppState;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/upload", post(upload_media).layer(DefaultBodyLimit::max(20 * 1024 * 1024)))
}

async fn upload_media(
    _auth: AuthAdmin,
    State(_state): State<AppState>,
    mut multipart: Multipart,
) -> Result<Json<ProcessedImage>, ApiError> {
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
