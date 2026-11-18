mod common;

use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use serde_json::Value;
use tower::ServiceExt;

#[tokio::test]
async fn test_openapi_spec_and_scalar_docs_exposed() {
    let state = common::test_app_state().await;
    let app = vaultscope_api::routes::build_router(state);

    // 1. Fetch /api/openapi.json
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/openapi.json")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("Failed to execute /api/openapi.json request");

    assert_eq!(response.status(), StatusCode::OK);

    let body_bytes = to_bytes(response.into_body(), 10 * 1024 * 1024)
        .await
        .expect("Failed to read /api/openapi.json body");
    let spec: Value = serde_json::from_slice(&body_bytes).expect("Invalid OpenAPI JSON");

    assert!(
        spec["openapi"]
            .as_str()
            .unwrap_or_default()
            .starts_with("3.1"),
        "Expected OpenAPI 3.1.x, got {:?}",
        spec["openapi"]
    );
    assert_eq!(spec["info"]["title"], "VaultScope API (VAMOS)");

    // Security schemes
    assert!(spec["components"]["securitySchemes"]["bearer_auth"].is_object());
    assert!(spec["components"]["securitySchemes"]["csrf_token"].is_object());

    let paths = spec["paths"]
        .as_object()
        .expect("OpenAPI spec should have a paths object");

    let has_path_or_prefix = |prefix: &str| {
        paths
            .keys()
            .any(|k| k == prefix || k.starts_with(&format!("{prefix}/")))
    };

    // Core, Health, CSRF, Auth, Webhooks
    for expected in [
        "/api/csrf",
        "/api/health",
        "/api/auth/login",
        "/api/auth/register",
        "/api/auth/verify-email",
        "/api/auth/resend-verification",
        "/api/webhooks/stripe",
    ] {
        assert!(
            has_path_or_prefix(expected),
            "Missing core path {expected} in OpenAPI paths: {:?}",
            paths.keys().collect::<Vec<_>>()
        );
    }

    // Storefront groups
    for expected in [
        "/api/storefront/config",
        "/api/storefront/catalog",
        "/api/storefront/options",
        "/api/storefront/partners",
        "/api/storefront/ps",
        "/api/storefront/meta/ps",
        "/api/storefront/meta/blog",
        "/api/storefront/blog",
        "/api/storefront/billing",
        "/api/storefront/activity",
        "/api/storefront/invoices",
        "/api/storefront/orders",
        "/api/storefront/profile",
        "/api/storefront/dedicated/products",
        "/api/storefront/services",
        "/api/storefront/tickets",
    ] {
        assert!(
            has_path_or_prefix(expected),
            "Missing storefront path group {expected} in OpenAPI spec"
        );
    }

    assert!(
        !paths
            .keys()
            .any(|k| k.to_ascii_lowercase().contains("reliablesite") && k.starts_with("/api/storefront")),
        "Storefront OpenAPI paths must stay white-labeled"
    );

    // Admin groups
    for expected in [
        "/api/admin/activity",
        "/api/admin/blog",
        "/api/admin/coupons",
        "/api/admin/customers",
        "/api/admin/dashboard/stats",
        "/api/admin/departments",
        "/api/admin/email-logs",
        "/api/admin/email-templates",
        "/api/admin/internal-tools",
        "/api/admin/invoices",
        "/api/admin/jobs",
        "/api/admin/mailboxes",
        "/api/admin/media/upload",
        "/api/admin/notifications",
        "/api/admin/partners",
        "/api/admin/products",
        "/api/admin/reliablesite/servers",
        "/api/admin/reliablesite/sync",
        "/api/admin/services",
        "/api/admin/service-forms",
        "/api/admin/settings",
        "/api/admin/staff",
        "/api/admin/tax-rates",
        "/api/admin/tickets",
    ] {
        assert!(
            has_path_or_prefix(expected),
            "Missing admin path group {expected} in OpenAPI spec"
        );
    }

    let op_count: usize = paths
        .values()
        .filter_map(|v| v.as_object())
        .map(|obj| {
            ["get", "post", "put", "patch", "delete"]
                .iter()
                .filter(|m| obj.contains_key(**m))
                .count()
        })
        .sum();
    assert!(
        paths.len() >= 80 && op_count >= 120,
        "Unexpectedly low OpenAPI coverage: {} paths, {} operations",
        paths.len(),
        op_count
    );
    println!("OpenAPI spec verified: {} paths, {} operations", paths.len(), op_count);

    // 2. Fetch /api/docs (Scalar interactive documentation)
    let docs_response = app
        .oneshot(
            Request::builder()
                .uri("/api/docs")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("Failed to execute /api/docs request");

    assert_eq!(docs_response.status(), StatusCode::OK);
    let docs_bytes = to_bytes(docs_response.into_body(), 5 * 1024 * 1024)
        .await
        .expect("Failed to read /api/docs body");
    let docs_html = String::from_utf8_lossy(&docs_bytes);
    assert!(
        docs_html.to_lowercase().contains("scalar") || docs_html.contains("VaultScope API"),
        "Expected Scalar HTML page at /api/docs"
    );
}
