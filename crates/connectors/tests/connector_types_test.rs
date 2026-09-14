use vaultscope_connectors::ConnectorError;

#[test]
fn test_connector_error_display_api() {
    let err = ConnectorError::Api("rate limit exceeded".to_string());
    assert_eq!(err.to_string(), "provider API error: rate limit exceeded");
}

#[test]
fn test_connector_error_display_auth() {
    let err = ConnectorError::AuthFailed;
    assert_eq!(err.to_string(), "authentication failed");
}

#[test]
fn test_connector_error_display_not_found() {
    let err = ConnectorError::NotFound("srv-999".to_string());
    assert_eq!(err.to_string(), "resource not found: srv-999");
}

#[test]
fn test_connector_error_display_rate_limited() {
    let err = ConnectorError::RateLimited;
    assert_eq!(err.to_string(), "rate limited");
}

#[test]
fn test_connector_error_display_bad_config() {
    let err = ConnectorError::BadConfig;
    assert_eq!(err.to_string(), "configuration missing or invalid");
}

#[test]
fn test_connector_error_is_debug() {
    let err = ConnectorError::Api("test".to_string());
    let debug = format!("{:?}", err);
    assert!(debug.contains("Api"));
}
