use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;

#[derive(Debug)]
pub enum ApiError {
    NotFound(String),
    BadRequest(String),
    Unauthorized,
    Forbidden,
    Internal(String),
    Database(sqlx::Error),
    Connector(vaultscope_connectors::ConnectorError),
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, message) = match &self {
            ApiError::NotFound(msg) => (StatusCode::NOT_FOUND, msg.clone()),
            ApiError::BadRequest(msg) => (StatusCode::BAD_REQUEST, msg.clone()),
            ApiError::Unauthorized => (StatusCode::UNAUTHORIZED, "unauthorized".to_string()),
            ApiError::Forbidden => (StatusCode::FORBIDDEN, "forbidden".to_string()),
            ApiError::Internal(msg) => (StatusCode::INTERNAL_SERVER_ERROR, msg.clone()),
            ApiError::Database(_) => (StatusCode::INTERNAL_SERVER_ERROR, "database error".to_string()),
            ApiError::Connector(e) => (StatusCode::BAD_GATEWAY, e.to_string()),
        };

        let body = json!({ "error": message });
        (status, Json(body)).into_response()
    }
}

impl From<sqlx::Error> for ApiError {
    fn from(e: sqlx::Error) -> Self {
        tracing::error!("database error: {}", e);
        ApiError::Database(e)
    }
}

impl From<vaultscope_connectors::ConnectorError> for ApiError {
    fn from(e: vaultscope_connectors::ConnectorError) -> Self {
        ApiError::Connector(e)
    }
}
