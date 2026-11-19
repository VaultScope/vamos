use std::path::PathBuf;
use tokio::fs;
use uuid::Uuid;

use crate::error::ApiError;

pub const MAX_IMAGE_WIDTH: u32 = 8192;
pub const MAX_IMAGE_HEIGHT: u32 = 8192;
pub const MAX_IMAGE_ALLOC_BYTES: u64 = 64 * 1024 * 1024;
pub const MAX_TICKET_ATTACHMENT_SIZE: usize = 10 * 1024 * 1024;
pub const MAX_TICKET_ATTACHMENTS: usize = 10;

#[derive(serde::Serialize, utoipa::ToSchema)]
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

fn reject_svg_xml_or_html(raw_data: &[u8]) -> Result<(), ApiError> {
    let trimmed = raw_data.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(raw_data);
    let prefix: String = trimmed
        .iter()
        .skip_while(|b| b.is_ascii_whitespace())
        .take(64)
        .map(|b| b.to_ascii_lowercase() as char)
        .collect();

    if prefix.starts_with("<?xml")
        || prefix.starts_with("<svg")
        || prefix.starts_with("<!doctype svg")
        || prefix.starts_with("<!doctype html")
        || prefix.starts_with("<html")
        || prefix.starts_with("<script")
        || prefix.starts_with("<!--")
    {
        return Err(ApiError::BadRequest(
            "SVG, XML, and HTML payloads are not allowed".into(),
        ));
    }

    Ok(())
}

/// Decodes and validates a PNG, JPEG, or WebP payload under strict dimension and
/// memory limits, resizes if needed, and re-encodes to WebP.
/// Rejects SVG/XML/HTML, unsupported formats (including un-re-encoded GIF),
/// and decompression bombs.
pub fn decode_and_reencode_webp(raw_data: &[u8], max_dimension: u32) -> Result<Vec<u8>, ApiError> {
    if raw_data.is_empty() {
        return Err(ApiError::BadRequest("File is empty".into()));
    }

    reject_svg_xml_or_html(raw_data)?;

    let mut reader = image::ImageReader::new(std::io::Cursor::new(raw_data))
        .with_guessed_format()
        .map_err(|_| ApiError::BadRequest("Failed to inspect image format".into()))?;

    match reader.format() {
        Some(image::ImageFormat::Png | image::ImageFormat::Jpeg | image::ImageFormat::WebP) => {}
        _ => {
            return Err(ApiError::BadRequest(
                "Unsupported image format. Only PNG, JPEG, and WebP are accepted.".into(),
            ));
        }
    }

    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_IMAGE_WIDTH);
    limits.max_image_height = Some(MAX_IMAGE_HEIGHT);
    limits.max_alloc = Some(MAX_IMAGE_ALLOC_BYTES);
    reader.limits(limits);

    let dynamic_img = reader.decode().map_err(|e| {
        tracing::warn!("Rejected image upload during decode: {}", e);
        ApiError::BadRequest(
            "Invalid, corrupted, or oversized image (max 8192x8192, 64MB decode memory)".into(),
        )
    })?;

    let processed_img = if max_dimension > 0
        && (dynamic_img.width() > max_dimension || dynamic_img.height() > max_dimension)
    {
        dynamic_img.resize(
            max_dimension,
            max_dimension,
            image::imageops::FilterType::Lanczos3,
        )
    } else {
        dynamic_img
    };

    let mut webp_buf = std::io::Cursor::new(Vec::new());
    processed_img
        .write_to(&mut webp_buf, image::ImageFormat::WebP)
        .map_err(|e| {
            tracing::error!("WebP encoding failed: {}", e);
            ApiError::Internal("Failed to re-encode uploaded image".into())
        })?;

    Ok(webp_buf.into_inner())
}

/// Optimizes an uploaded image:
/// - Rejects SVG/XML and non-PNG/JPEG/WebP payloads
/// - Enforces strict ImageReader decode dimension and allocation limits
/// - Scales down if dimensions exceed `max_dimension` (preserving aspect ratio)
/// - Always re-encodes to WebP (never writes raw un-re-encoded bytes)
pub async fn save_optimized_image(
    raw_data: &[u8],
    subfolder: &str,
    max_dimension: u32,
) -> Result<ProcessedImage, ApiError> {
    if subfolder.is_empty()
        || !subfolder
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
    {
        return Err(ApiError::BadRequest("Invalid upload subfolder".into()));
    }

    let owned = raw_data.to_vec();
    let final_bytes =
        tokio::task::spawn_blocking(move || decode_and_reencode_webp(&owned, max_dimension))
            .await
            .map_err(|e| ApiError::Internal(format!("Image processing task failed: {e}")))??;

    let id = Uuid::now_v7();
    let root = get_uploads_root();
    let target_dir = root.join(subfolder);
    fs::create_dir_all(&target_dir)
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to create upload dir: {}", e)))?;

    let ext = "webp";
    let content_type = "image/webp".to_string();
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

/// Validates and stores a ticket image attachment (PNG, JPEG, WebP only; no GIF/SVG).
pub async fn validate_and_save_ticket_attachment(
    raw_filename: &str,
    content_type: Option<&str>,
    data: &[u8],
) -> Result<ProcessedImage, ApiError> {
    let ext = raw_filename
        .rsplit('.')
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();

    if !["png", "jpg", "jpeg", "webp"].contains(&ext.as_str()) {
        return Err(ApiError::BadRequest(
            "Only image files (PNG, JPEG, WebP) are accepted.".into(),
        ));
    }

    if let Some(ct) = content_type {
        let base_ct = ct
            .split(';')
            .next()
            .unwrap_or("")
            .trim()
            .to_ascii_lowercase();
        if !["image/png", "image/jpeg", "image/jpg", "image/webp"].contains(&base_ct.as_str()) {
            return Err(ApiError::BadRequest(
                "Only image files (PNG, JPEG, WebP) are accepted.".into(),
            ));
        }
    }

    if data.is_empty() {
        return Err(ApiError::BadRequest("File is empty.".into()));
    }

    if data.len() > MAX_TICKET_ATTACHMENT_SIZE {
        return Err(ApiError::BadRequest(
            "Image file exceeds the 10MB size limit.".into(),
        ));
    }

    let is_valid_magic = match ext.as_str() {
        "png" => data.starts_with(&[0x89, 0x50, 0x4E, 0x47]),
        "jpg" | "jpeg" => data.starts_with(&[0xFF, 0xD8, 0xFF]),
        "webp" => data.starts_with(b"RIFF") && data.len() > 12 && &data[8..12] == b"WEBP",
        _ => false,
    };

    if !is_valid_magic {
        return Err(ApiError::BadRequest(
            "File content does not match a valid image format.".into(),
        ));
    }

    save_optimized_image(data, "tickets", 2048).await
}

/// Validates a ticket attachment URL, permitting only `/uploads/...`, `https://...`,
/// or `http://...` and explicitly rejecting scriptable/dangerous schemes.
pub fn validate_attachment_url(raw_url: &str) -> Result<String, ApiError> {
    if raw_url.chars().any(|c| c.is_control()) {
        return Err(ApiError::BadRequest(
            "Attachment URL contains invalid control characters".into(),
        ));
    }

    let url = raw_url.trim();
    if url.is_empty() || url.len() > 2048 {
        return Err(ApiError::BadRequest(
            "Attachment URL must be between 1 and 2048 characters".into(),
        ));
    }

    let lower = url.to_ascii_lowercase();
    for dangerous in [
        "javascript:",
        "data:",
        "vbscript:",
        "file:",
        "blob:",
        "about:",
    ] {
        if lower.starts_with(dangerous) {
            return Err(ApiError::BadRequest(
                "Disallowed URI scheme in attachment URL".into(),
            ));
        }
    }

    if url.starts_with("/uploads/") {
        if url.starts_with("//")
            || url.contains("..")
            || url.contains('\\')
            || url.len() <= "/uploads/".len()
        {
            return Err(ApiError::BadRequest(
                "Invalid local attachment upload path".into(),
            ));
        }
        return Ok(url.to_string());
    }

    if lower.starts_with("https://") || lower.starts_with("http://") {
        let parsed = reqwest::Url::parse(url).map_err(|_| {
            ApiError::BadRequest("Invalid HTTP(S) attachment URL".into())
        })?;
        if (parsed.scheme() != "https" && parsed.scheme() != "http")
            || parsed.host_str().is_none()
            || !parsed.username().is_empty()
            || parsed.password().is_some()
        {
            return Err(ApiError::BadRequest(
                "Invalid HTTP(S) attachment URL".into(),
            ));
        }
        return Ok(url.to_string());
    }

    Err(ApiError::BadRequest(
        "Attachment URL must start with https://, http://, or /uploads/".into(),
    ))
}

/// Validates and normalizes a ticket message `attachments` JSON array.
pub fn validate_ticket_attachments(
    attachments: Option<&serde_json::Value>,
) -> Result<serde_json::Value, ApiError> {
    let Some(val) = attachments else {
        return Ok(serde_json::json!([]));
    };

    let items = val
        .as_array()
        .ok_or_else(|| ApiError::BadRequest("Attachments must be a JSON array".into()))?;

    if items.len() > MAX_TICKET_ATTACHMENTS {
        return Err(ApiError::BadRequest(format!(
            "Maximum of {} attachments allowed per message",
            MAX_TICKET_ATTACHMENTS
        )));
    }

    let mut sanitized = Vec::with_capacity(items.len());
    for item in items {
        let mut obj = item
            .as_object()
            .ok_or_else(|| {
                ApiError::BadRequest("Each attachment must be a JSON object".into())
            })?
            .clone();

        let raw_url = obj
            .get("url")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                ApiError::BadRequest("Each attachment must include a string url".into())
            })?;
        let clean_url = validate_attachment_url(raw_url)?;
        obj.insert("url".to_string(), serde_json::Value::String(clean_url));

        if let Some(fname_val) = obj.get("filename") {
            let fname = fname_val.as_str().ok_or_else(|| {
                ApiError::BadRequest("Attachment filename must be a string".into())
            })?;
            if fname.len() > 255 || fname.chars().any(|c| c.is_control()) {
                return Err(ApiError::BadRequest(
                    "Invalid attachment filename".into(),
                ));
            }
        }

        if let Some(ct_val) = obj.get("content_type") {
            let ct = ct_val.as_str().ok_or_else(|| {
                ApiError::BadRequest("Attachment content_type must be a string".into())
            })?;
            let ct_low = ct.to_ascii_lowercase();
            if ct_low.contains("svg") || ct_low.contains("html") || ct_low.contains("script") {
                return Err(ApiError::BadRequest(
                    "Disallowed attachment content_type".into(),
                ));
            }
        }

        sanitized.push(serde_json::Value::Object(obj));
    }

    Ok(serde_json::Value::Array(sanitized))
}
