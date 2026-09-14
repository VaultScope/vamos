use axum::http::StatusCode;
use axum::response::IntoResponse;
use vaultscope_api::error::ApiError;

#[tokio::test]
async fn test_not_found_response() {
    let err = ApiError::NotFound("Widget not found".into());
    let response = err.into_response();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_bad_request_response() {
    let err = ApiError::BadRequest("Invalid input".into());
    let response = err.into_response();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn test_unauthorized_response() {
    let err = ApiError::Unauthorized;
    let response = err.into_response();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_forbidden_response() {
    let err = ApiError::Forbidden;
    let response = err.into_response();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn test_internal_error_response() {
    let err = ApiError::Internal("something broke".into());
    let response = err.into_response();
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
}

#[tokio::test]
async fn test_connector_error_response() {
    let err = ApiError::Connector(vaultscope_connectors::ConnectorError::AuthFailed);
    let response = err.into_response();
    assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
}

#[tokio::test]
async fn test_error_body_is_json() {
    let err = ApiError::NotFound("test item".into());
    let response = err.into_response();
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["error"], "test item");
}

#[tokio::test]
async fn test_unauthorized_body_message() {
    let err = ApiError::Unauthorized;
    let response = err.into_response();
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["error"], "unauthorized");
}

#[tokio::test]
async fn test_forbidden_body_message() {
    let err = ApiError::Forbidden;
    let response = err.into_response();
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["error"], "forbidden");
}
