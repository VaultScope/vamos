use axum::{body::Body, http::Request};
use axum::http::StatusCode;
use tower::ServiceExt;

mod common;

#[tokio::test]
async fn test_products_list_requires_auth() {
    let app_state = common::test_app_state().await;
    let app = vaultscope_api::routes::build_router(app_state);

    let request = Request::builder()
        .uri("/api/admin/products")
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}
