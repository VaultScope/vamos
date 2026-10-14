use std::path::PathBuf;
use tokio::fs;
use uuid::Uuid;

use crate::error::ApiError;

#[derive(serde::Serialize)]
pub struct ProcessedImage {
    pub id: Uuid,
    pub url: String,
    pub filename: String,
    pub content_type: String,
    pub size: usize,
}

pub fn get_uploads_root() -> PathBuf {
    let dir = std::env::var("UPLOADS_DIR").unwrap_or_else(|_| "uploads".to_string());
    PathBuf::from(dir)
}

/// Optimizes an uploaded image:
/// - Decodes PNG, JPEG, WebP
/// - Scales down if dimensions exceed `max_dimension` (preserving aspect ratio)
/// - Encodes to highly compressed WebP format for optimal bandwidth and storage efficiency
/// - Gracefully falls back to original bytes for SVG / GIF or unconvertible images
pub async fn save_optimized_image(
    raw_data: &[u8],
    subfolder: &str,
    max_dimension: u32,
) -> Result<ProcessedImage, ApiError> {
    if raw_data.is_empty() {
        return Err(ApiError::BadRequest("File is empty".into()));
    }

    let id = Uuid::now_v7();
    let root = get_uploads_root();
    let target_dir = root.join(subfolder);
    fs::create_dir_all(&target_dir)
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to create upload dir: {}", e)))?;

    // Try to decode with image crate
    let (final_bytes, ext, content_type) = match image::load_from_memory(raw_data) {
        Ok(dynamic_img) => {
            let processed_img = if dynamic_img.width() > max_dimension || dynamic_img.height() > max_dimension {
                dynamic_img.resize(max_dimension, max_dimension, image::imageops::FilterType::Lanczos3)
            } else {
                dynamic_img
            };

            let mut webp_buf = std::io::Cursor::new(Vec::new());
            match processed_img.write_to(&mut webp_buf, image::ImageFormat::WebP) {
                Ok(_) => {
                    let out = webp_buf.into_inner();
                    (out, "webp".to_string(), "image/webp".to_string())
                }
                Err(e) => {
                    tracing::warn!("WebP encoding failed ({}), saving original bytes", e);
                    (raw_data.to_vec(), "png".to_string(), "image/png".to_string())
                }
            }
        }
        Err(_) => {
            // If it's SVG or animated GIF that image crate doesn't decode, check magic bytes
            if raw_data.starts_with(b"GIF87a") || raw_data.starts_with(b"GIF89a") {
                (raw_data.to_vec(), "gif".to_string(), "image/gif".to_string())
            } else if raw_data.starts_with(b"<?xml") || raw_data.starts_with(b"<svg") {
                (raw_data.to_vec(), "svg".to_string(), "image/svg+xml".to_string())
            } else {
                return Err(ApiError::BadRequest("Invalid image format or corrupted file".into()));
            }
        }
    };

    let filename = format!("{}.{}", id, ext);
    let file_path = target_dir.join(&filename);
    fs::write(&file_path, &final_bytes)
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to write file: {}", e)))?;

    let url = format!("/uploads/{}/{}", subfolder, filename);
    let size = final_bytes.len();

    Ok(ProcessedImage {
        id,
        url,
        filename,
        content_type,
        size,
    })
}
