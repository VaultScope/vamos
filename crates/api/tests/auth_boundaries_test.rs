use axum::{body::Body, http::Request};
use axum::http::StatusCode;
use tower::ServiceExt;
use vaultscope_api::auth::{jwt, Audience, Claims};

mod common;

#[tokio::test]
async fn test_customer_cannot_access_admin_routes() {
    let app_state = common::test_app_state().await;
    
    // Generate customer token
    let claims = Claims {
        sub: uuid::Uuid::now_v7(),
        email: "test@customer.com".to_string(),
        role: "customer".to_string(),
        audience: Audience::Storefront,
        iat: chrono::Utc::now().timestamp(),
        exp: (chrono::Utc::now() + chrono::Duration::hours(1)).timestamp(),
    };
    let token = jwt::sign(&claims, &app_state.config.jwt_secret).unwrap();
    
    let app = vaultscope_api::routes::build_router(app_state);

    // Attempt to access admin route
    let request = Request::builder()
        .uri("/api/admin/products")
        .header("Authorization", format!("Bearer {}", token))
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(request).await.unwrap();
    
    // Should be Forbidden because the token is valid but audience is Storefront
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}
