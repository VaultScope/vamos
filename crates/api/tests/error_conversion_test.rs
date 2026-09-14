use axum::http::StatusCode;
use axum::response::IntoResponse;
use vaultscope_api::error::ApiError;
use vaultscope_connectors::ConnectorError;

async fn body_string(err: ApiError) -> String {
    let response = err.into_response();
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    String::from_utf8(body.to_vec()).unwrap()
}

#[test]
fn test_connector_error_converts_to_api_error() {
    let ce = ConnectorError::Api("test".to_string());
    let ae: ApiError = ce.into();
    let resp = ae.into_response();
    assert_eq!(resp.status(), StatusCode::BAD_GATEWAY);
}

#[test]
fn test_connector_auth_failed_converts() {
    let ae: ApiError = ConnectorError::AuthFailed.into();
    let resp = ae.into_response();
    assert_eq!(resp.status(), StatusCode::BAD_GATEWAY);
}

#[test]
fn test_connector_rate_limited_converts() {
    let ae: ApiError = ConnectorError::RateLimited.into();
    let resp = ae.into_response();
    assert_eq!(resp.status(), StatusCode::BAD_GATEWAY);
}

#[test]
fn test_connector_not_found_converts() {
    let ae: ApiError = ConnectorError::NotFound("srv-1".to_string()).into();
    let resp = ae.into_response();
    assert_eq!(resp.status(), StatusCode::BAD_GATEWAY);
}

#[test]
fn test_connector_bad_config_converts() {
    let ae: ApiError = ConnectorError::BadConfig.into();
    let resp = ae.into_response();
    assert_eq!(resp.status(), StatusCode::BAD_GATEWAY);
}

#[tokio::test]
async fn test_connector_api_body_message() {
    let body = body_string(ApiError::Connector(ConnectorError::Api("provider timeout".into()))).await;
    assert!(body.contains("provider API error: provider timeout"), "body was: {}", body);
}

#[tokio::test]
async fn test_connector_auth_failed_body_message() {
    let body = body_string(ApiError::Connector(ConnectorError::AuthFailed)).await;
    assert!(body.contains("authentication failed"), "body was: {}", body);
}

#[tokio::test]
async fn test_connector_rate_limited_body_message() {
    let body = body_string(ApiError::Connector(ConnectorError::RateLimited)).await;
    assert!(body.contains("rate limited"), "body was: {}", body);
}

#[tokio::test]
async fn test_connector_not_found_body_message() {
    let body = body_string(ApiError::Connector(ConnectorError::NotFound("x".into()))).await;
    assert!(body.contains("resource not found: x"), "body was: {}", body);
}

#[tokio::test]
async fn test_connector_bad_config_body_message() {
    let body = body_string(ApiError::Connector(ConnectorError::BadConfig)).await;
    assert!(body.contains("configuration missing or invalid"), "body was: {}", body);
}
