use axum::{
    body::Body,
    extract::FromRef,
    http::{header, Method, Request, StatusCode},
    routing::{get, post},
    Json, Router,
};
use axum_csrf::{CsrfConfig, CsrfToken};
use image::{DynamicImage, ImageFormat};
use serde_json::{json, Value};
use std::io::Cursor;
use tower::ServiceExt;

use vaultscope_api::{
    media::{
        decode_and_reencode_webp, validate_and_save_ticket_attachment, validate_attachment_url,
        validate_ticket_attachments,
    },
    middleware::{
        csrf::{csrf_middleware, is_jwt_like},
        security_headers::security_headers_middleware,
    },
    routes::{
        admin::{services::validate_optional_client_ip, staff::ensure_role_assignable},
        storefront::profile::{generate_recovery_codes, parse_and_validate_vat_id},
    },
};

#[derive(Clone)]
struct CsrfOnlyState {
    csrf_config: CsrfConfig,
}

impl FromRef<CsrfOnlyState> for CsrfConfig {
    fn from_ref(state: &CsrfOnlyState) -> Self {
        state.csrf_config.clone()
    }
}

fn build_csrf_test_app() -> Router {
    let state = CsrfOnlyState {
        csrf_config: CsrfConfig::default(),
    };

    let protected = Router::new()
        .route("/mutate", post(|| async { StatusCode::OK }).get(|| async { StatusCode::OK }))
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            csrf_middleware,
        ));

    Router::new()
        .route(
            "/csrf",
            get(|token: CsrfToken| async move {
                let raw = token.authenticity_token().unwrap_or_default();
                (token, Json(json!({ "token": raw })))
            }),
        )
        .merge(protected)
        .with_state(state)
}

fn make_png(width: u32, height: u32) -> Vec<u8> {
    let img = DynamicImage::new_rgba8(width, height);
    let mut buf = Cursor::new(Vec::new());
    img.write_to(&mut buf, ImageFormat::Png).expect("encode png");
    buf.into_inner()
}

fn make_jpeg(width: u32, height: u32) -> Vec<u8> {
    let img = DynamicImage::new_rgb8(width, height);
    let mut buf = Cursor::new(Vec::new());
    img.write_to(&mut buf, ImageFormat::Jpeg)
        .expect("encode jpeg");
    buf.into_inner()
}

#[test]
fn test_media_rejects_svg_xml_and_html_payloads() {
    let payloads: Vec<&[u8]> = vec![
        br#"<svg xmlns="http://www.w3.org/2000/svg"><script>alert(1)</script></svg>"#,
        br#"<?xml version="1.0" encoding="UTF-8"?><svg onload="alert(1)"/>"#,
        b"\xEF\xBB\xBF   \n\t<SVG onload=alert(1)>",
        br#"<!-- lead comment --><svg><script>alert(1)</script></svg>"#,
        br#"<!DOCTYPE html><html><script>alert(1)</script></html>"#,
        br#"<script>alert(1)</script>"#,
    ];

    for payload in payloads {
        let res = decode_and_reencode_webp(payload, 2048);
        assert!(res.is_err(), "Expected rejection for payload {:?}", String::from_utf8_lossy(payload));
    }
}

#[test]
fn test_media_rejects_raw_gif_and_non_image_payloads() {
    let gif_1x1: &[u8] = b"GIF89a\x01\x00\x01\x00\x80\x00\x00\x00\x00\x00\xff\xff\xff!\xf9\x04\x01\x00\x00\x00\x00,\x00\x00\x00\x00\x01\x00\x01\x00\x00\x02\x02D\x01\x00;";
    assert!(decode_and_reencode_webp(gif_1x1, 2048).is_err());
    assert!(decode_and_reencode_webp(b"", 2048).is_err());
    assert!(decode_and_reencode_webp(b"MZ\x90\x00not_an_image", 2048).is_err());
}

#[test]
fn test_media_enforces_image_dimension_limits_and_saves_valid_webp() {
    // 8193px width exceeds max_image_width(8192)
    let too_wide_png = make_png(8193, 1);
    assert!(
        decode_and_reencode_webp(&too_wide_png, 2048).is_err(),
        "Width > 8192 must be rejected by ImageReader limits"
    );

    // 8193px height exceeds max_image_height(8192)
    let too_tall_png = make_png(1, 8193);
    assert!(
        decode_and_reencode_webp(&too_tall_png, 2048).is_err(),
        "Height > 8192 must be rejected by ImageReader limits"
    );

    // Valid PNG -> resized & re-encoded as WebP
    let valid_png = make_png(64, 32);
    let webp_from_png = decode_and_reencode_webp(&valid_png, 32).expect("valid PNG should encode to WebP");
    assert!(webp_from_png.starts_with(b"RIFF"));
    assert_eq!(&webp_from_png[8..12], b"WEBP");
    let decoded = image::load_from_memory(&webp_from_png).expect("output must be valid WebP");
    assert_eq!(decoded.width(), 32);
    assert_eq!(decoded.height(), 16);

    // Valid JPEG -> re-encoded as WebP
    let valid_jpeg = make_jpeg(40, 20);
    let webp_from_jpeg = decode_and_reencode_webp(&valid_jpeg, 2048).expect("valid JPEG should encode to WebP");
    assert!(webp_from_jpeg.starts_with(b"RIFF"));
    assert_eq!(&webp_from_jpeg[8..12], b"WEBP");
}

#[tokio::test]
async fn test_ticket_upload_validation_rejects_gif_and_svg() {
    let gif_1x1: &[u8] = b"GIF89a\x01\x00\x01\x00\x80\x00\x00\x00\x00\x00\xff\xff\xff!\xf9\x04\x01\x00\x00\x00\x00,\x00\x00\x00\x00\x01\x00\x01\x00\x00\x02\x02D\x01\x00;";
    assert!(
        validate_and_save_ticket_attachment("anim.gif", Some("image/gif"), gif_1x1)
            .await
            .is_err()
    );
    assert!(
        validate_and_save_ticket_attachment("xss.svg", Some("image/svg+xml"), b"<svg/>")
            .await
            .is_err()
    );
    assert!(
        validate_and_save_ticket_attachment("spoof.png", Some("image/svg+xml"), &make_png(4, 4))
            .await
            .is_err()
    );
    assert!(
        validate_and_save_ticket_attachment("spoof.png", Some("image/png"), b"<svg/>")
            .await
            .is_err()
    );
}

#[tokio::test]
async fn test_csrf_rejects_forged_origin_and_dummy_header() {
    let app = build_csrf_test_app();

    // Substring bypass attempt with evil-vaultscope.de + dummy header must be rejected
    let res_evil_origin = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/mutate")
                .header(header::ORIGIN, "https://evil-vaultscope.de")
                .header("x-csrf-token", "dummy-token")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res_evil_origin.status(), StatusCode::FORBIDDEN);

    // Even exact vaultscope.de origin with invalid/missing cookie token must be rejected
    let res_exact_origin_bad_token = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/mutate")
                .header(header::ORIGIN, "https://vaultscope.de")
                .header(header::REFERER, "https://localhost:5173/admin")
                .header("x-csrf-token", "dummy-token")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res_exact_origin_bad_token.status(), StatusCode::FORBIDDEN);

    // Malformed / non-JWT Bearer header must not bypass CSRF
    let res_fake_bearer = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/mutate")
                .header(header::AUTHORIZATION, "Bearer dummy_not_a_jwt")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res_fake_bearer.status(), StatusCode::FORBIDDEN);

    assert!(!is_jwt_like(""));
    assert!(!is_jwt_like("a.b"));
    assert!(!is_jwt_like("a..c"));
    assert!(!is_jwt_like("a.b.c.d"));
    assert!(is_jwt_like("eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjMifQ.signature_part-1"));

    // Valid JWT-like Bearer header passes CSRF layer (auth extractor validates signature)
    let res_jwt_bearer = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/mutate")
                .header(
                    header::AUTHORIZATION,
                    "Bearer eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxIn0.c2ln",
                )
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res_jwt_bearer.status(), StatusCode::OK);

    // Genuine /csrf cookie + x-csrf-token pair succeeds
    let csrf_resp = app
        .clone()
        .oneshot(Request::builder().uri("/csrf").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(csrf_resp.status(), StatusCode::OK);
    let cookie = csrf_resp
        .headers()
        .get_all(header::SET_COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .filter_map(|s| s.split(';').next())
        .collect::<Vec<_>>()
        .join("; ");
    let body_bytes = axum::body::to_bytes(csrf_resp.into_body(), 1024 * 64)
        .await
        .unwrap();
    let json_val: Value = serde_json::from_slice(&body_bytes).unwrap();
    let token = json_val["token"].as_str().unwrap();

    let res_valid_csrf = app
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/mutate")
                .header(header::COOKIE, cookie)
                .header("x-csrf-token", token)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res_valid_csrf.status(), StatusCode::OK);
}

#[tokio::test]
async fn test_security_headers_middleware_applied() {
    let app = Router::new()
        .route("/api/storefront/profile", get(|| async { StatusCode::OK }))
        .route("/uploads/tickets/pic.webp", get(|| async { StatusCode::OK }))
        .route("/api/docs", get(|| async { StatusCode::OK }))
        .layer(axum::middleware::from_fn(security_headers_middleware));

    let api_resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/storefront/profile")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let h = api_resp.headers();
    assert_eq!(h.get("x-content-type-options").unwrap(), "nosniff");
    assert_eq!(h.get("x-frame-options").unwrap(), "DENY");
    assert_eq!(
        h.get("referrer-policy").unwrap(),
        "strict-origin-when-cross-origin"
    );
    assert_eq!(
        h.get("permissions-policy").unwrap(),
        "camera=(), microphone=(), geolocation=()"
    );
    assert_eq!(h.get("x-xss-protection").unwrap(), "0");
    assert_eq!(
        h.get("content-security-policy").unwrap(),
        "default-src 'none'; frame-ancestors 'none'"
    );

    let upload_resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/uploads/tickets/pic.webp")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        upload_resp.headers().get("content-security-policy").unwrap(),
        "default-src 'none'; frame-ancestors 'none'; sandbox"
    );

    let docs_resp = app
        .oneshot(
            Request::builder()
                .uri("/api/docs")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(
        docs_resp.headers().get("content-security-policy").is_none(),
        "Restrictive default-src 'none' must be skipped on /api/docs HTML"
    );
    assert_eq!(
        docs_resp.headers().get("permissions-policy").unwrap(),
        "camera=(), microphone=(), geolocation=()"
    );
}

#[test]
fn test_verify_vat_rejects_utf8_multibyte_and_path_traversal() {
    // Multi-byte UTF-8 inputs must fail cleanly without slicing panic
    for bad in ["äö", "äöü", "DE1234ä", "ß123456", "🔥123456", "ÉÈ123456"] {
        assert!(
            parse_and_validate_vat_id(bad).is_err(),
            "UTF-8 VAT ID '{bad}' must be rejected"
        );
    }

    // Path traversal / query / fragment injection attempts
    for bad in [
        "DE../../admin",
        "../../etc/passwd",
        "DE123/456",
        "DE123?foo=bar",
        "DE123#frag",
        "DE123\\456",
        "121234567",
        "DE1",
        "DE1234567890123456",
    ] {
        assert!(
            parse_and_validate_vat_id(bad).is_err(),
            "Malformed/traversal VAT ID '{bad}' must be rejected"
        );
    }

    let (full, cc, num) =
        parse_and_validate_vat_id(" de 123.456-789 ").expect("valid German VAT ID");
    assert_eq!(full, "DE123456789");
    assert_eq!(cc, "DE");
    assert_eq!(num, "123456789");

    let (full_fr, cc_fr, num_fr) =
        parse_and_validate_vat_id("frXX123456789").expect("valid French alphanumeric VAT ID");
    assert_eq!(full_fr, "FRXX123456789");
    assert_eq!(cc_fr, "FR");
    assert_eq!(num_fr, "XX123456789");
}

#[test]
fn test_recovery_code_entropy_at_least_80_bits() {
    let codes = generate_recovery_codes(8);
    assert_eq!(codes.len(), 8);
    let mut seen = std::collections::HashSet::new();
    for (formatted, hash) in codes {
        let normalized = formatted.replace('-', "");
        assert_eq!(normalized.len(), 20, "10 CSPRNG bytes = 20 hex chars = 80 bits");
        assert!(normalized.bytes().all(|b| b.is_ascii_hexdigit()));
        assert_eq!(hash.len(), 64);
        assert!(seen.insert(formatted));
    }
}

#[test]
fn test_attachment_url_and_array_sanitizer_rejects_xss_schemes() {
    for bad in [
        "javascript:alert(1)",
        "  JaVaScRiPt:alert(document.cookie)",
        "data:text/html,<script>alert(1)</script>",
        "vbscript:msgbox(1)",
        "file:///etc/passwd",
        "//evil.com/payload.png",
        "/uploads/../../etc/passwd",
        "/uploads/tickets\\evil.png",
        "/uploads/",
        "https://user:pass@evil.com/a.png",
        "ftp://example.com/a.png",
        "",
    ] {
        assert!(
            validate_attachment_url(bad).is_err(),
            "URL '{bad}' must be rejected"
        );
    }

    assert_eq!(
        validate_attachment_url(" /uploads/tickets/abc.webp ").unwrap(),
        "/uploads/tickets/abc.webp"
    );
    assert_eq!(
        validate_attachment_url("https://cdn.vaultscope.de/img/a.png").unwrap(),
        "https://cdn.vaultscope.de/img/a.png"
    );
    assert_eq!(
        validate_attachment_url("http://localhost:5173/img.png").unwrap(),
        "http://localhost:5173/img.png"
    );

    // Reject javascript: inside ticket attachment JSON array
    let bad_arr = json!([{ "url": "javascript:alert(1)", "filename": "x.png" }]);
    assert!(validate_ticket_attachments(Some(&bad_arr)).is_err());

    // Reject SVG content_type
    let svg_ct = json!([{ "url": "/uploads/tickets/x.webp", "content_type": "image/svg+xml" }]);
    assert!(validate_ticket_attachments(Some(&svg_ct)).is_err());

    // Reject > 10 attachments
    let too_many: Vec<Value> = (0..11)
        .map(|i| json!({ "url": format!("/uploads/tickets/{i}.webp") }))
        .collect();
    assert!(validate_ticket_attachments(Some(&Value::Array(too_many))).is_err());

    // Accept valid array up to 10 items
    let ok_arr = json!([
        {
            "id": "att-1",
            "filename": "screenshot.png",
            "url": "/uploads/tickets/0193.webp",
            "size": 1234,
            "content_type": "image/webp"
        }
    ]);
    let validated = validate_ticket_attachments(Some(&ok_arr)).expect("valid attachments");
    assert_eq!(validated.as_array().unwrap().len(), 1);
}

#[test]
fn test_staff_privilege_escalation_logic() {
    let wildcard = vec!["*".to_string()];
    let staff_mgr = vec!["staff.manage".to_string(), "tickets.view".to_string()];
    let staff_and_rbac = vec!["staff.manage".to_string(), "rbac.manage".to_string()];

    // Wildcard owner can assign anything
    assert!(ensure_role_assignable(&wildcard, &wildcard).is_ok());
    assert!(ensure_role_assignable(&wildcard, &["rbac.manage".into()]).is_ok());

    // Non-wildcard staff manager cannot grant "*", "rbac.manage", or unheld permissions
    assert!(ensure_role_assignable(&staff_mgr, &wildcard).is_err());
    assert!(ensure_role_assignable(&staff_mgr, &["rbac.manage".into()]).is_err());
    assert!(ensure_role_assignable(&staff_mgr, &["invoices.delete".into()]).is_err());
    assert!(ensure_role_assignable(
        &staff_mgr,
        &["tickets.view".into(), "invoices.view".into()]
    )
    .is_err());

    // Subset of caller's permissions is permitted
    assert!(ensure_role_assignable(&staff_mgr, &["tickets.view".into()]).is_ok());
    assert!(ensure_role_assignable(&staff_mgr, &staff_mgr).is_ok());
    assert!(ensure_role_assignable(&staff_mgr, &[]).is_ok());

    // Caller who already holds rbac.manage may assign rbac.manage, but still not "*"
    assert!(ensure_role_assignable(&staff_and_rbac, &["rbac.manage".into()]).is_ok());
    assert!(ensure_role_assignable(&staff_and_rbac, &wildcard).is_err());
}

#[test]
fn test_admin_ipmi_client_ip_validation() {
    assert_eq!(validate_optional_client_ip(None).unwrap(), None);
    assert_eq!(validate_optional_client_ip(Some("   ")).unwrap(), None);
    assert_eq!(
        validate_optional_client_ip(Some(" 198.51.100.42 ")).unwrap(),
        Some("198.51.100.42".to_string())
    );
    assert_eq!(
        validate_optional_client_ip(Some("2001:db8::1")).unwrap(),
        Some("2001:db8::1".to_string())
    );
    assert!(validate_optional_client_ip(Some("not-an-ip")).is_err());
    assert!(validate_optional_client_ip(Some("1.2.3.4; whoami")).is_err());
}
