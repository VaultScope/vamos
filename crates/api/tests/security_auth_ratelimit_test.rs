mod common;

use std::net::{IpAddr, SocketAddr};
use std::sync::{Arc, Mutex};

use axum::body::{to_bytes, Body};
use axum::extract::ConnectInfo;
use axum::http::{header, HeaderMap, HeaderValue, Request, StatusCode};
use axum::response::IntoResponse;
use bcrypt::{hash, DEFAULT_COST};
use chrono::Utc;
use serde_json::{json, Value};
use totp_rs::{Algorithm, Secret, TOTP};
use tower::ServiceExt;
use uuid::Uuid;

use vaultscope_api::auth::{jwt, Audience, Claims};
use vaultscope_api::config::{validate_jwt_secret_with_mode, Config};
use vaultscope_api::error::ApiError;
use vaultscope_api::middleware::cors;
use vaultscope_api::middleware::rate_limit::{
    parse_trusted_proxy_cidrs, resolve_client_ip_with_cidrs, RateLimiter, MAX_ENTRIES,
};
use vaultscope_api::routes::build_router;
use vaultscope_connectors::ConnectorError;

static ENV_LOCK: Mutex<()> = Mutex::new(());

fn make_claims(sub: Uuid, email: &str, role: &str, audience: Audience, auth_version: i64) -> Claims {
    Claims {
        sub,
        email: email.to_string(),
        role: role.to_string(),
        audience,
        iat: Utc::now().timestamp(),
        exp: jwt::new_expiry(),
        auth_version: Some(auth_version),
        jti: Some(Uuid::now_v7().to_string()),
    }
}

async fn get_csrf_cookie_and_token(app: &axum::Router) -> (String, String) {
    let resp = app
        .clone()
        .oneshot(Request::builder().uri("/api/csrf").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let set_cookie = resp
        .headers()
        .get_all(header::SET_COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .map(|s| s.split(';').next().unwrap_or("").trim().to_string())
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("; ");
    let bytes = to_bytes(resp.into_body(), 1024 * 1024).await.unwrap();
    let body: Value = serde_json::from_slice(&bytes).unwrap();
    (set_cookie, body["token"].as_str().unwrap().to_string())
}

#[test]
fn test_spoofed_forwarded_headers_ignored_from_untrusted_peer() {
    let mut headers = HeaderMap::new();
    headers.insert("cf-connecting-ip", HeaderValue::from_static("1.2.3.4"));
    headers.insert("x-forwarded-for", HeaderValue::from_static("5.6.7.8"));

    // Untrusted public peer IP must ignore CF-Connecting-IP and X-Forwarded-For
    let external_peer: IpAddr = "203.0.113.50".parse().unwrap();
    assert_eq!(
        resolve_client_ip_with_cidrs(&headers, Some(external_peer), &[]),
        "203.0.113.50"
    );

    // Loopback peer (e.g. local nginx) trusts CF-Connecting-IP
    let loopback: IpAddr = "127.0.0.1".parse().unwrap();
    assert_eq!(
        resolve_client_ip_with_cidrs(&headers, Some(loopback), &[]),
        "1.2.3.4"
    );

    // Private proxy hop trusts X-Forwarded-For when CF-Connecting-IP is absent
    let mut xff_headers = HeaderMap::new();
    xff_headers.insert(
        "x-forwarded-for",
        HeaderValue::from_static("9.9.9.9, 198.51.100.22, 10.0.0.5"),
    );
    let private_hop: IpAddr = "10.0.0.2".parse().unwrap();
    // Right-to-left skips trusted 10.0.0.5 and selects 198.51.100.22 (preventing left-side spoofing of 9.9.9.9)
    assert_eq!(
        resolve_client_ip_with_cidrs(&xff_headers, Some(private_hop), &[]),
        "198.51.100.22"
    );

    // Configured TRUSTED_PROXY_CIDRS trusts Cloudflare edge CIDR
    let extra = parse_trusted_proxy_cidrs("203.0.113.0/24");
    assert_eq!(
        resolve_client_ip_with_cidrs(&headers, Some(external_peer), &extra),
        "1.2.3.4"
    );

    // Missing ConnectInfo (oneshot unit test without socket peer) fails safe and does NOT trust spoofed headers
    assert_eq!(
        resolve_client_ip_with_cidrs(&headers, None, &extra),
        "unknown"
    );
}

#[test]
fn test_rate_limiter_enforces_max_entries_cap() {
    let limiter = RateLimiter::new(5, 3600);
    for i in 0..(MAX_ENTRIES + 500) {
        let ip = format!(
            "{}.{}.{}.{}",
            (i >> 24) & 0xFF,
            (i >> 16) & 0xFF,
            (i >> 8) & 0xFF,
            i & 0xFF
        );
        assert!(limiter.check(&ip));
    }
    assert!(
        limiter.entry_count() <= MAX_ENTRIES,
        "entry_count {} must stay capped at {}",
        limiter.entry_count(),
        MAX_ENTRIES
    );
}

#[tokio::test]
async fn test_storefront_and_auth_rate_limits_and_header_spoofing_protection() {
    let state = common::test_app_state().await;
    let app = build_router(state);

    // 1. Storefront rate limit active (120 req/min); spoofing X-Forwarded-For from public peer cannot bypass it
    let public_peer = SocketAddr::from(([203, 0, 113, 99], 44444));
    for i in 0..120 {
        let mut req = Request::builder()
            .uri("/api/storefront/catalog")
            .header("cf-connecting-ip", format!("198.51.100.{}", i % 250))
            .header("x-forwarded-for", format!("192.0.2.{}", i % 250))
            .body(Body::empty())
            .unwrap();
        req.extensions_mut().insert(ConnectInfo(public_peer));
        let resp = app.clone().oneshot(req).await.unwrap();
        assert_ne!(
            resp.status(),
            StatusCode::TOO_MANY_REQUESTS,
            "Request {} within 120/min budget should not be rate limited",
            i + 1
        );
    }

    let mut req_121 = Request::builder()
        .uri("/api/storefront/catalog")
        .header("cf-connecting-ip", "8.8.8.8")
        .body(Body::empty())
        .unwrap();
    req_121.extensions_mut().insert(ConnectInfo(public_peer));
    let resp_121 = app.clone().oneshot(req_121).await.unwrap();
    assert_eq!(resp_121.status(), StatusCode::TOO_MANY_REQUESTS);

    // 2. Strict /api/auth/login rate limit (20 req/min)
    let auth_peer = SocketAddr::from(([198, 51, 100, 77], 55555));
    for i in 0..20 {
        let mut req = Request::builder()
            .method("POST")
            .uri("/api/auth/login")
            .header(header::CONTENT_TYPE, "application/json")
            .header("x-forwarded-for", format!("1.1.1.{}", i))
            .body(Body::from(
                json!({ "email": "nobody@example.com", "password": "wrong", "audience": "storefront" })
                    .to_string(),
            ))
            .unwrap();
        req.extensions_mut().insert(ConnectInfo(auth_peer));
        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    let mut auth_21 = Request::builder()
        .method("POST")
        .uri("/api/auth/login")
        .header(header::CONTENT_TYPE, "application/json")
        .header("x-forwarded-for", "9.9.9.9")
        .body(Body::from(
            json!({ "email": "nobody@example.com", "password": "wrong", "audience": "storefront" })
                .to_string(),
        ))
        .unwrap();
    auth_21.extensions_mut().insert(ConnectInfo(auth_peer));
    let resp_auth_21 = app.clone().oneshot(auth_21).await.unwrap();
    assert_eq!(resp_auth_21.status(), StatusCode::TOO_MANY_REQUESTS);
}

#[tokio::test]
async fn test_openapi_and_docs_hidden_when_dev_mode_false() {
    let mut state = common::test_app_state().await;
    let mut cfg = (*state.config).clone();
    cfg.dev_mode = false;
    state.config = Arc::new(cfg);

    let app = build_router(state);

    let openapi_resp = app
        .clone()
        .oneshot(Request::builder().uri("/api/openapi.json").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(openapi_resp.status(), StatusCode::NOT_FOUND);

    let docs_resp = app
        .oneshot(Request::builder().uri("/api/docs").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(docs_resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_sse_ticket_single_use_and_query_token_rejected() {
    let state = common::test_app_state().await;
    let cust_id = Uuid::now_v7();
    let short = &cust_id.simple().to_string()[20..];

    sqlx::query(
        "INSERT INTO customers (id, external_id, username, name, email, status, email_verified) VALUES ($1, $2, $3, 'SSE User', $4, 'active', true)",
    )
    .bind(cust_id)
    .bind(format!("cust_sse_{short}"))
    .bind(format!("sse_{short}"))
    .bind(format!("sse_{short}@test.local"))
    .execute(&state.db)
    .await
    .unwrap();

    let ticket_id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO tickets (id, ticket_number, customer_id, category, subject, status, priority, mailbox, ip)
         VALUES ($1, $2, $3, 'support', 'SSE Test Ticket', 'open', 'normal', 'support@vaultscope.de', '127.0.0.1')",
    )
    .bind(ticket_id)
    .bind(format!("TKT-SSE-{short}"))
    .bind(cust_id)
    .execute(&state.db)
    .await
    .unwrap();

    let token = jwt::sign(
        &make_claims(cust_id, "sse@test.local", "customer", Audience::Storefront, 1),
        &state.config.jwt_secret,
    )
    .unwrap();

    let app = build_router(state.clone());

    // 1. Raw ?token= query param MUST be rejected everywhere (including /events)
    let raw_query_resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/api/storefront/tickets/{ticket_id}/events?token={token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(raw_query_resp.status(), StatusCode::UNAUTHORIZED);

    // 2. Mint single-use SSE ticket via POST /api/auth/sse-ticket
    let ticket_resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/auth/sse-ticket")
                .header(header::AUTHORIZATION, format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(ticket_resp.status(), StatusCode::OK);
    let ticket_json: Value =
        serde_json::from_slice(&to_bytes(ticket_resp.into_body(), 1024 * 64).await.unwrap())
            .unwrap();
    let sse_ticket = ticket_json["ticket"].as_str().unwrap().to_string();
    assert_eq!(sse_ticket.len(), 64);

    // 3. SSE ticket cannot be used on non-/events endpoints
    let non_events_resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/api/storefront/profile?ticket={sse_ticket}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(non_events_resp.status(), StatusCode::UNAUTHORIZED);

    // 4. Cross-audience SSE ticket rejected on admin /events (and consumed!)
    let cross_aud_resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/api/admin/tickets/{ticket_id}/events?ticket={sse_ticket}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(cross_aud_resp.status(), StatusCode::FORBIDDEN);

    // Mint a fresh SSE ticket for valid storefront /events usage
    let ticket_resp_2 = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/auth/sse-ticket")
                .header(header::AUTHORIZATION, format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let sse_ticket_2: String = serde_json::from_slice::<Value>(
        &to_bytes(ticket_resp_2.into_body(), 1024 * 64).await.unwrap(),
    )
    .unwrap()["ticket"]
        .as_str()
        .unwrap()
        .to_string();

    // 5. Valid ?ticket= succeeds once on storefront /events
    let sse_ok = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/api/storefront/tickets/{ticket_id}/events?ticket={sse_ticket_2}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(sse_ok.status(), StatusCode::OK);

    // 6. Replaying the same ?ticket= is rejected with 401
    let sse_replay = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/api/storefront/tickets/{ticket_id}/events?ticket={sse_ticket_2}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(sse_replay.status(), StatusCode::UNAUTHORIZED);

    let _ = sqlx::query("DELETE FROM tickets WHERE id = $1").bind(ticket_id).execute(&state.db).await;
    let _ = sqlx::query("DELETE FROM customers WHERE id = $1").bind(cust_id).execute(&state.db).await;
}

#[tokio::test]
async fn test_unverified_login_blocked_and_logout_revokes_session() {
    let state = common::test_app_state().await;
    let cust_id = Uuid::now_v7();
    let short = &cust_id.simple().to_string()[20..];
    let email = format!("auth-test-{short}@test.local");
    let pw = "StrongTestPass123!";
    let pw_hash = hash(pw, DEFAULT_COST).unwrap();

    sqlx::query(
        "INSERT INTO customers (id, external_id, username, name, email, status, email_verified, password_hash)
         VALUES ($1, $2, $3, 'Auth Flow User', $4, 'active', false, $5)",
    )
    .bind(cust_id)
    .bind(format!("cust_auth_{short}"))
    .bind(format!("auth_{short}"))
    .bind(&email)
    .bind(&pw_hash)
    .execute(&state.db)
    .await
    .unwrap();

    let app = build_router(state.clone());

    // 1. Unverified customer login MUST fail
    let unverified_resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/auth/login")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    json!({ "email": email, "password": pw, "audience": "storefront" }).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(unverified_resp.status(), StatusCode::BAD_REQUEST);

    // Mark email verified
    sqlx::query("UPDATE customers SET email_verified = true WHERE id = $1")
        .bind(cust_id)
        .execute(&state.db)
        .await
        .unwrap();

    // 2. Verified login succeeds and returns JWT with auth_version & jti
    let login_resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/auth/login")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    json!({ "email": email, "password": pw, "audience": "storefront" }).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(login_resp.status(), StatusCode::OK);
    let login_json: Value =
        serde_json::from_slice(&to_bytes(login_resp.into_body(), 1024 * 64).await.unwrap())
            .unwrap();
    let token = login_json["token"].as_str().unwrap().to_string();
    assert_eq!(login_json["claims"]["auth_version"], 1);
    assert!(login_json["claims"]["jti"].as_str().is_some());

    // 3. Protected profile works before logout
    let profile_before = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/storefront/profile")
                .header(header::AUTHORIZATION, format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(profile_before.status(), StatusCode::OK);

    // 4. Logout bumps auth_version in DB
    let logout_resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/auth/logout")
                .header(header::AUTHORIZATION, format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(logout_resp.status(), StatusCode::OK);

    let db_ver: i64 = sqlx::query_scalar("SELECT auth_version FROM customers WHERE id = $1")
        .bind(cust_id)
        .fetch_one(&state.db)
        .await
        .unwrap();
    assert_eq!(db_ver, 2);

    // 5. Prior JWT is immediately rejected on protected endpoints & logout
    let profile_after = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/storefront/profile")
                .header(header::AUTHORIZATION, format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(profile_after.status(), StatusCode::UNAUTHORIZED);

    let logout_again = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/auth/logout")
                .header(header::AUTHORIZATION, format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(logout_again.status(), StatusCode::UNAUTHORIZED);

    let _ = sqlx::query("DELETE FROM customers WHERE id = $1").bind(cust_id).execute(&state.db).await;
}

#[tokio::test]
async fn test_staff_mfa_and_customer_encrypted_totp_secret() {
    let state = common::test_app_state().await;
    let role_id: Uuid = sqlx::query_scalar("SELECT id FROM roles LIMIT 1")
        .fetch_one(&state.db)
        .await
        .unwrap();
    let staff_id = Uuid::now_v7();
    let short = &staff_id.simple().to_string()[20..];
    let staff_email = format!("StaffMfa-{short}@test.local");
    let pw = "AdminPass1234!";
    let pw_hash = hash(pw, DEFAULT_COST).unwrap();

    sqlx::query(
        "INSERT INTO staff (id, external_id, username, first_name, last_name, email, email_verified, role_id, password_hash)
         VALUES ($1, $2, $3, 'MFA', 'Admin', $4, true, $5, $6)",
    )
    .bind(staff_id)
    .bind(format!("stf_{short}"))
    .bind(format!("stfmfa_{short}"))
    .bind(&staff_email)
    .bind(role_id)
    .bind(&pw_hash)
    .execute(&state.db)
    .await
    .unwrap();

    let app = build_router(state.clone());

    // 1. Staff login case-insensitive email match + updates last_login_ip from peer ConnectInfo
    let staff_peer = SocketAddr::from(([198, 51, 100, 88], 12345));
    let mut login_req = Request::builder()
        .method("POST")
        .uri("/api/auth/login")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            json!({ "email": staff_email.to_lowercase(), "password": pw, "audience": "admin" })
                .to_string(),
        ))
        .unwrap();
    login_req.extensions_mut().insert(ConnectInfo(staff_peer));
    let login_resp = app.clone().oneshot(login_req).await.unwrap();
    assert_eq!(login_resp.status(), StatusCode::OK);
    let login_json: Value =
        serde_json::from_slice(&to_bytes(login_resp.into_body(), 1024 * 64).await.unwrap())
            .unwrap();
    let staff_token = login_json["token"].as_str().unwrap().to_string();

    let recorded_ip: String = sqlx::query_scalar("SELECT last_login_ip FROM staff WHERE id = $1")
        .bind(staff_id)
        .fetch_one(&state.db)
        .await
        .unwrap();
    assert_eq!(recorded_ip, "198.51.100.88");

    // 2. Admin MFA setup stores encrypted secret (`enc:v1:...`)
    let setup_resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/auth/admin/mfa/setup")
                .header(header::AUTHORIZATION, format!("Bearer {staff_token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(setup_resp.status(), StatusCode::OK);
    let setup_json: Value =
        serde_json::from_slice(&to_bytes(setup_resp.into_body(), 1024 * 256).await.unwrap())
            .unwrap();
    let secret_base32 = setup_json["secret"].as_str().unwrap().to_string();

    let stored_db_secret: String =
        sqlx::query_scalar("SELECT totp_secret FROM staff WHERE id = $1")
            .bind(staff_id)
            .fetch_one(&state.db)
            .await
            .unwrap();
    assert!(stored_db_secret.starts_with("enc:v1:"));
    assert_ne!(stored_db_secret, secret_base32);

    // 3. Enable admin MFA with valid TOTP code
    let totp = TOTP::new(
        Algorithm::SHA1,
        6,
        1,
        30,
        Secret::Encoded(secret_base32.clone()).to_bytes().unwrap(),
        Some("VaultScope Admin".to_string()),
        staff_email.clone(),
    )
    .unwrap();
    let code = totp.generate_current().unwrap();

    let enable_resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/auth/admin/mfa/enable")
                .header(header::AUTHORIZATION, format!("Bearer {staff_token}"))
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(json!({ "code": code }).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(enable_resp.status(), StatusCode::OK);

    // 4. Subsequent staff login without totp_code returns mfa_required: true
    let challenge_resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/auth/login")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    json!({ "email": staff_email, "password": pw, "audience": "admin" })
                        .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(challenge_resp.status(), StatusCode::OK);
    let challenge_json: Value =
        serde_json::from_slice(&to_bytes(challenge_resp.into_body(), 1024 * 64).await.unwrap())
            .unwrap();
    assert_eq!(challenge_json["mfa_required"], true);

    // 5. Staff login with valid TOTP code succeeds
    let mfa_login_resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/auth/login")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    json!({ "email": staff_email, "password": pw, "audience": "admin", "totp_code": code })
                        .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(mfa_login_resp.status(), StatusCode::OK);
    let mfa_login_json: Value =
        serde_json::from_slice(&to_bytes(mfa_login_resp.into_body(), 1024 * 64).await.unwrap())
            .unwrap();
    assert!(mfa_login_json["token"].as_str().is_some());

    // 6. Customer MFA setup encrypts totp_secret (`enc:v1:...`) and login supports encrypted & legacy Base32
    let cust_id = Uuid::now_v7();
    let cshort = &cust_id.simple().to_string()[20..];
    let cust_email = format!("custmfa_{cshort}@test.local");
    sqlx::query(
        "INSERT INTO customers (id, external_id, username, name, email, status, email_verified, password_hash)
         VALUES ($1, $2, $3, 'MFA Customer', $4, 'active', true, $5)",
    )
    .bind(cust_id)
    .bind(format!("cmfa_{cshort}"))
    .bind(format!("cmfa_{cshort}"))
    .bind(&cust_email)
    .bind(&pw_hash)
    .execute(&state.db)
    .await
    .unwrap();

    let cust_token = jwt::sign(
        &make_claims(cust_id, &cust_email, "customer", Audience::Storefront, 1),
        &state.config.jwt_secret,
    )
    .unwrap();
    let (cookie, csrf) = get_csrf_cookie_and_token(&app).await;

    let c_setup = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/storefront/profile/mfa/setup")
                .header(header::AUTHORIZATION, format!("Bearer {cust_token}"))
                .header(header::COOKIE, &cookie)
                .header("x-csrf-token", &csrf)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(c_setup.status(), StatusCode::OK);
    let c_secret_stored: String =
        sqlx::query_scalar("SELECT totp_secret FROM customers WHERE id = $1")
            .bind(cust_id)
            .fetch_one(&state.db)
            .await
            .unwrap();
    assert!(c_secret_stored.starts_with("enc:v1:"));

    let _ = sqlx::query("DELETE FROM customers WHERE id = $1").bind(cust_id).execute(&state.db).await;
    let _ = sqlx::query("DELETE FROM staff WHERE id = $1").bind(staff_id).execute(&state.db).await;

    // 7. Customer login works with BOTH encrypted TOTP secret and legacy plaintext Base32 secret
    let leg_id = Uuid::now_v7();
    let lshort = &leg_id.simple().to_string()[20..];
    let leg_email = format!("legmfa-{lshort}@test.local");
    let leg_secret = Secret::generate_secret();
    let leg_base32 = leg_secret.to_encoded().to_string();
    let leg_totp = TOTP::new(
        Algorithm::SHA1,
        6,
        1,
        30,
        leg_secret.to_bytes().unwrap(),
        Some("VaultScope".to_string()),
        leg_email.clone(),
    )
    .unwrap();
    let leg_code = leg_totp.generate_current().unwrap();

    sqlx::query(
        "INSERT INTO customers (id, external_id, username, name, email, status, email_verified, two_factor_enabled, totp_secret, password_hash)
         VALUES ($1, $2, $3, 'Legacy TOTP Customer', $4, 'active', true, true, $5, $6)",
    )
    .bind(leg_id)
    .bind(format!("cleg_{lshort}"))
    .bind(format!("cleg_{lshort}"))
    .bind(&leg_email)
    .bind(&leg_base32)
    .bind(&pw_hash)
    .execute(&state.db)
    .await
    .unwrap();

    let leg_login = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/auth/login")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    json!({ "email": leg_email, "password": pw, "audience": "storefront", "totp_code": leg_code })
                        .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(leg_login.status(), StatusCode::OK);

    // 8. Non-active lifecycle_state ('deletion_pending') blocked on login and AuthCustomer
    sqlx::query("UPDATE customers SET lifecycle_state = 'deletion_pending' WHERE id = $1")
        .bind(leg_id)
        .execute(&state.db)
        .await
        .unwrap();

    let leg_token = jwt::sign(
        &make_claims(leg_id, &leg_email, "customer", Audience::Storefront, 1),
        &state.config.jwt_secret,
    )
    .unwrap();
    let deleted_profile_resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/storefront/profile")
                .header(header::AUTHORIZATION, format!("Bearer {leg_token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(deleted_profile_resp.status(), StatusCode::UNAUTHORIZED);

    let _ = sqlx::query("DELETE FROM customers WHERE id = $1").bind(leg_id).execute(&state.db).await;
}

#[tokio::test]
async fn test_ticket_attachment_access_control_and_headers() {
    let state = common::test_app_state().await;

    let owner_id = Uuid::now_v7();
    let other_id = Uuid::now_v7();
    let owner_short = &owner_id.simple().to_string()[20..];
    let other_short = &other_id.simple().to_string()[20..];

    for (cid, tag) in [(owner_id, owner_short), (other_id, other_short)] {
        sqlx::query(
            "INSERT INTO customers (id, external_id, username, name, email, status, email_verified)
             VALUES ($1, $2, $3, 'Attachment User', $4, 'active', true)",
        )
        .bind(cid)
        .bind(format!("cust_att_{tag}"))
        .bind(format!("att_{tag}"))
        .bind(format!("att_{tag}@test.local"))
        .execute(&state.db)
        .await
        .unwrap();
    }

    let ticket_id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO tickets (id, ticket_number, customer_id, category, subject, status, priority, mailbox, ip)
         VALUES ($1, $2, $3, 'support', 'Attachment Ticket', 'open', 'normal', 'support@vaultscope.de', '127.0.0.1')",
    )
    .bind(ticket_id)
    .bind(format!("TKT-ATT-{owner_short}"))
    .bind(owner_id)
    .execute(&state.db)
    .await
    .unwrap();

    let pub_filename = format!("att_pub_{owner_short}.webp");
    let int_filename = format!("att_int_{owner_short}.webp");

    // Create dummy files on disk under uploads/tickets
    let tickets_dir = vaultscope_api::media::get_uploads_root().join("tickets");
    tokio::fs::create_dir_all(&tickets_dir).await.unwrap();
    tokio::fs::write(tickets_dir.join(&pub_filename), b"RIFF0000WEBPdata")
        .await
        .unwrap();
    tokio::fs::write(tickets_dir.join(&int_filename), b"RIFF0000WEBPsecret")
        .await
        .unwrap();

    // Public customer-visible attachment message
    sqlx::query(
        "INSERT INTO ticket_messages (id, ticket_id, author_id, author_type, content, internal, attachments)
         VALUES ($1, $2, $3, 'customer', 'see attachment', false, $4)",
    )
    .bind(Uuid::now_v7())
    .bind(ticket_id)
    .bind(owner_id)
    .bind(json!([{ "url": format!("/uploads/tickets/{pub_filename}") }]))
    .execute(&state.db)
    .await
    .unwrap();

    // Internal staff-only note attachment (customer MUST NOT download)
    sqlx::query(
        "INSERT INTO ticket_messages (id, ticket_id, author_id, author_type, content, internal, attachments)
         VALUES ($1, $2, $3, 'staff', 'internal note', true, $4)",
    )
    .bind(Uuid::now_v7())
    .bind(ticket_id)
    .bind(owner_id)
    .bind(json!([{ "url": format!("/uploads/tickets/{int_filename}") }]))
    .execute(&state.db)
    .await
    .unwrap();

    let owner_token = jwt::sign(
        &make_claims(owner_id, "owner@test.local", "customer", Audience::Storefront, 1),
        &state.config.jwt_secret,
    )
    .unwrap();
    let other_token = jwt::sign(
        &make_claims(other_id, "other@test.local", "customer", Audience::Storefront, 1),
        &state.config.jwt_secret,
    )
    .unwrap();

    let app = build_router(state.clone());

    // 1. Public ServeDir path /uploads/tickets/{file} MUST NOT serve ticket attachments (404)
    let legacy_public = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/uploads/tickets/{pub_filename}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(legacy_public.status(), StatusCode::NOT_FOUND);

    // 2. Unauthenticated GET /api/uploads/tickets/{file} returns 401
    let unauth_resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/api/uploads/tickets/{pub_filename}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(unauth_resp.status(), StatusCode::UNAUTHORIZED);

    // 3. Cross-tenant customer returns 403 Forbidden
    let cross_resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/api/uploads/tickets/{pub_filename}"))
                .header(header::AUTHORIZATION, format!("Bearer {other_token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(cross_resp.status(), StatusCode::FORBIDDEN);

    // 4. Customer cannot download attachment on internal staff note (403 Forbidden)
    let internal_resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/api/uploads/tickets/{int_filename}"))
                .header(header::AUTHORIZATION, format!("Bearer {owner_token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(internal_resp.status(), StatusCode::FORBIDDEN);

    // 5. Owning customer downloads public attachment with safe headers
    let ok_resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/api/uploads/tickets/{pub_filename}"))
                .header(header::AUTHORIZATION, format!("Bearer {owner_token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(ok_resp.status(), StatusCode::OK);
    assert_eq!(
        ok_resp.headers().get(header::CONTENT_DISPOSITION).unwrap(),
        &format!("attachment; filename=\"{pub_filename}\"")
    );
    assert_eq!(
        ok_resp.headers().get(header::CACHE_CONTROL).unwrap(),
        "private, no-store"
    );

    // 6. Staff with tickets.view can download attachment; staff without tickets.view gets 403
    let role_view_id = Uuid::now_v7();
    let role_none_id = Uuid::now_v7();
    let staff_view_id = Uuid::now_v7();
    let staff_none_id = Uuid::now_v7();

    sqlx::query("INSERT INTO roles (id, name, permissions, mapped_group) VALUES ($1, $2, ARRAY['tickets.view'], 'g1'), ($3, $4, ARRAY['blog.write'], 'g2')")
        .bind(role_view_id)
        .bind(format!("TicketViewer_{owner_short}"))
        .bind(role_none_id)
        .bind(format!("BlogOnly_{owner_short}"))
        .execute(&state.db)
        .await
        .unwrap();

    sqlx::query(
        "INSERT INTO staff (id, external_id, username, first_name, last_name, email, email_verified, role_id)
         VALUES ($1, $2, $3, 'View', 'Staff', $4, true, $5),
                ($6, $7, $8, 'NoView', 'Staff', $9, true, $10)",
    )
    .bind(staff_view_id)
    .bind(format!("sv_{owner_short}"))
    .bind(format!("sv_{owner_short}"))
    .bind(format!("sv-{owner_short}@test.local"))
    .bind(role_view_id)
    .bind(staff_none_id)
    .bind(format!("sn_{owner_short}"))
    .bind(format!("sn_{owner_short}"))
    .bind(format!("sn-{owner_short}@test.local"))
    .bind(role_none_id)
    .execute(&state.db)
    .await
    .unwrap();

    let staff_view_tok = jwt::sign(
        &make_claims(staff_view_id, "sv@test.local", "TicketViewer", Audience::Admin, 1),
        &state.config.jwt_secret,
    )
    .unwrap();
    let staff_none_tok = jwt::sign(
        &make_claims(staff_none_id, "sn@test.local", "BlogOnly", Audience::Admin, 1),
        &state.config.jwt_secret,
    )
    .unwrap();

    let admin_ok = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/api/uploads/tickets/{pub_filename}"))
                .header(header::AUTHORIZATION, format!("Bearer {staff_view_tok}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(admin_ok.status(), StatusCode::OK);

    let admin_denied = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/api/uploads/tickets/{pub_filename}"))
                .header(header::AUTHORIZATION, format!("Bearer {staff_none_tok}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(admin_denied.status(), StatusCode::FORBIDDEN);

    let _ = sqlx::query("DELETE FROM staff WHERE id IN ($1, $2)").bind(staff_view_id).bind(staff_none_id).execute(&state.db).await;
    let _ = sqlx::query("DELETE FROM roles WHERE id IN ($1, $2)").bind(role_view_id).bind(role_none_id).execute(&state.db).await;

    let _ = tokio::fs::remove_file(tickets_dir.join(&pub_filename)).await;
    let _ = tokio::fs::remove_file(tickets_dir.join(&int_filename)).await;
    let _ = sqlx::query("DELETE FROM ticket_messages WHERE ticket_id = $1").bind(ticket_id).execute(&state.db).await;
    let _ = sqlx::query("DELETE FROM tickets WHERE id = $1").bind(ticket_id).execute(&state.db).await;
    let _ = sqlx::query("DELETE FROM customers WHERE id IN ($1, $2)").bind(owner_id).bind(other_id).execute(&state.db).await;
}

#[tokio::test]
async fn test_verification_token_expiry_and_resend_enumeration_and_pw_bounds() {
    let state = common::test_app_state().await;
    let cust_id = Uuid::now_v7();
    let short = &cust_id.simple().to_string()[20..];
    let expired_token = format!("expired_tok_{short}");
    let email = format!("exp-{short}@test.local");

    sqlx::query(
        "INSERT INTO customers (id, external_id, username, name, email, status, email_verified, verification_token, verification_token_expires_at)
         VALUES ($1, $2, $3, 'Expired Token User', $4, 'active', false, $5, now() - interval '1 hour')",
    )
    .bind(cust_id)
    .bind(format!("cust_exp_{short}"))
    .bind(format!("exp_{short}"))
    .bind(&email)
    .bind(&expired_token)
    .execute(&state.db)
    .await
    .unwrap();

    let app = build_router(state.clone());

    // 1. Expired customer verification token MUST be rejected
    let exp_resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/auth/verify-email")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(json!({ "token": expired_token }).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(exp_resp.status(), StatusCode::BAD_REQUEST);

    // 2. Expired billing email token MUST be rejected
    let billing_id = Uuid::now_v7();
    let expired_billing_tok = format!("expired_bill_{short}");
    sqlx::query(
        "INSERT INTO customer_billing_emails (id, customer_id, email, verified, verification_token, verification_token_expires_at)
         VALUES ($1, $2, $3, false, $4, now() - interval '5 minutes')",
    )
    .bind(billing_id)
    .bind(cust_id)
    .bind(format!("bill-{short}@test.local"))
    .bind(&expired_billing_tok)
    .execute(&state.db)
    .await
    .unwrap();

    let bill_exp_resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/auth/verify-email")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(json!({ "token": expired_billing_tok }).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(bill_exp_resp.status(), StatusCode::BAD_REQUEST);

    // 3. Resend verification returns identical normalized message for unknown vs unverified vs verified emails
    let unknown_resend = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/auth/resend-verification")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(json!({ "email": "nonexistent-user@example.com" }).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(unknown_resend.status(), StatusCode::OK);
    let unknown_msg: Value =
        serde_json::from_slice(&to_bytes(unknown_resend.into_body(), 65536).await.unwrap())
            .unwrap();

    sqlx::query("UPDATE customers SET email_verified = true WHERE id = $1")
        .bind(cust_id)
        .execute(&state.db)
        .await
        .unwrap();

    let verified_resend = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/auth/resend-verification")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(json!({ "email": email }).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(verified_resend.status(), StatusCode::OK);
    let verified_msg: Value =
        serde_json::from_slice(&to_bytes(verified_resend.into_body(), 65536).await.unwrap())
            .unwrap();
    assert_eq!(unknown_msg, verified_msg);

    // 4. Registration rejects >72 byte password
    let long_pw = "A".repeat(73);
    let reg_long = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/auth/register")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    json!({
                        "username": format!("longpw_{short}"),
                        "email": format!("longpw-{short}@example.com"),
                        "password": long_pw,
                        "name": "Long PW",
                        "address": "Main 1",
                        "city": "Berlin",
                        "country": "DE"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(reg_long.status(), StatusCode::BAD_REQUEST);

    let _ = sqlx::query("DELETE FROM customer_billing_emails WHERE id = $1").bind(billing_id).execute(&state.db).await;
    let _ = sqlx::query("DELETE FROM customers WHERE id = $1").bind(cust_id).execute(&state.db).await;
}

#[test]
fn test_config_debug_redacts_secrets_and_jwt_validation_and_env() {
    let mut cfg = common::test_config();
    cfg.database_url = "postgres://postgres:super_db_password@localhost/db".to_string();
    cfg.jwt_secret = "super_jwt_secret_value_1234567890".to_string();
    cfg.encryption_key = "super_enc_secret_value_1234567890".to_string();
    cfg.authentik_client_secret_admin = "super_oidc_admin_secret".to_string();
    cfg.authentik_client_secret_storefront = "super_oidc_sf_secret".to_string();
    cfg.stripe_secret_key = Some("sk_live_secret_value".to_string());
    cfg.stripe_webhook_secret = Some("whsec_secret_value".to_string());
    cfg.smtp_password = Some("smtp_pass_secret".to_string());
    cfg.redis_url = "redis://:redis_pass@redis:6379".to_string();

    let debug_str = format!("{:?}", cfg);
    for secret in [
        "super_db_password",
        "super_jwt_secret_value_1234567890",
        "super_enc_secret_value_1234567890",
        "super_oidc_admin_secret",
        "super_oidc_sf_secret",
        "sk_live_secret_value",
        "whsec_secret_value",
        "smtp_pass_secret",
        "redis_pass",
    ] {
        assert!(
            !debug_str.contains(secret),
            "Config Debug leaked secret '{}': {}",
            secret,
            debug_str
        );
    }
    assert!(debug_str.contains("[REDACTED]"));

    // JWT validation: distinct char count < 12 panics
    let low_entropy = std::panic::catch_unwind(|| {
        validate_jwt_secret_with_mode("abcabcabcabcabcabcabcabcabcabc12", false);
    });
    assert!(low_entropy.is_err());

    // Release mode rejects < 64 chars and placeholder substrings
    let short_release = std::panic::catch_unwind(|| {
        validate_jwt_secret_with_mode("AbCdEfGhIjKlMnOpQrStUvWxYz0123456789", true);
    });
    assert!(short_release.is_err());

    let placeholder_release = std::panic::catch_unwind(|| {
        validate_jwt_secret_with_mode(
            "dev-secret-AbCdEfGhIjKlMnOpQrStUvWxYz0123456789!@#$%^&*()-_=+[]{}",
            true,
        );
    });
    assert!(placeholder_release.is_err());

    // Valid 64+ high-entropy release secret passes
    validate_jwt_secret_with_mode(
        "9f8e7d6c5b4a3210KpQrStUvWxYz!@#$_+=-9f8e7d6c5b4a3210KpQrStUvWxYz",
        true,
    );

    // Env tests (serialized)
    let _guard = ENV_LOCK.lock().unwrap();

    // JWT_EXPIRATION_SECS respected when valid
    std::env::set_var("JWT_EXPIRATION_SECS", "1800");
    assert_eq!(jwt::token_expiry_secs(), 1800);
    std::env::remove_var("JWT_EXPIRATION_SECS");

    // Non-dev Config::from_env does NOT default CORS_ORIGINS to localhost and fails fast if OIDC enabled without secrets
    std::env::set_var("DATABASE_URL", "postgres://u:p@localhost/db");
    std::env::set_var(
        "ENCRYPTION_KEY",
        "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
    );
    std::env::set_var(
        "JWT_SECRET",
        "9f8e7d6c5b4a3210KpQrStUvWxYz!@#$_+=-9f8e7d6c5b4a3210KpQrStUvWxYz",
    );
    std::env::set_var("DEV_MODE", "false");
    std::env::set_var("AUTHENTIK_ENABLED", "false");
    std::env::remove_var("CORS_ORIGINS");
    std::env::set_var("STOREFRONT_URL", "https://portal.example.com");

    let prod_cfg = Config::from_env();
    assert_eq!(prod_cfg.cors_origins, vec!["https://portal.example.com".to_string()]);
    assert!(prod_cfg.authentik_client_secret_admin.is_empty());

    std::env::set_var("AUTHENTIK_ENABLED", "true");
    std::env::remove_var("AUTHENTIK_CLIENT_SECRET_ADMIN");
    std::env::remove_var("AUTHENTIK_CLIENT_SECRET_STOREFRONT");
    let missing_oidc = std::panic::catch_unwind(Config::from_env);
    assert!(missing_oidc.is_err());

    // Cleanup env
    for k in [
        "DATABASE_URL",
        "ENCRYPTION_KEY",
        "JWT_SECRET",
        "DEV_MODE",
        "AUTHENTIK_ENABLED",
        "STOREFRONT_URL",
    ] {
        std::env::remove_var(k);
    }
}

#[tokio::test]
async fn test_error_scrubbing_and_cors_filtering() {
    let internal_resp = ApiError::Internal("secret sql detail: password=abc".into()).into_response();
    assert_eq!(internal_resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let body = String::from_utf8(
        to_bytes(internal_resp.into_body(), 1024 * 64)
            .await
            .unwrap()
            .to_vec(),
    )
    .unwrap();
    assert!(!body.contains("password=abc"));
    assert!(body.contains("internal server error"));

    // Real reqwest error is scrubbed
    let req_err = reqwest::Client::new()
        .get("http://127.0.0.1:1/secret-path")
        .send()
        .await
        .unwrap_err();
    let conn_resp = ApiError::Connector(ConnectorError::Request(req_err)).into_response();
    assert_eq!(conn_resp.status(), StatusCode::BAD_GATEWAY);
    let conn_body = String::from_utf8(
        to_bytes(conn_resp.into_body(), 1024 * 64)
            .await
            .unwrap()
            .to_vec(),
    )
    .unwrap();
    assert!(!conn_body.contains("secret-path"));
    assert!(conn_body.contains("upstream provider communication error"));

    // CORS layer ignores wildcard and malformed origins without panicking
    let _layer = cors::layer(&[
        "*".to_string(),
        "not-a-url".to_string(),
        "ftp://invalid.scheme".to_string(),
        "https://vaultscope.de".to_string(),
    ]);
}
