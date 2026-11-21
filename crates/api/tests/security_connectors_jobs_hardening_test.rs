mod common;

use std::collections::HashSet;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::body::{to_bytes, Body};
use axum::http::{header, Request, StatusCode};
use axum::routing::post;
use axum::{Json, Router};
use chrono::{NaiveDate, Utc};
use rust_decimal::Decimal;
use serde_json::{json, Value};
use std::str::FromStr;
use tokio::net::TcpListener;
use tower::ServiceExt;
use uuid::Uuid;

use vaultscope_api::auth::{jwt, Audience, Claims};
use vaultscope_api::mailer::sanitize_email_header;
use vaultscope_api::pdf::generate_invoice_pdf;
use vaultscope_api::reliablesite_catalog::sanitize_service_for_storefront;
use vaultscope_api::routes::admin::coupons::validate_coupon_code;
use vaultscope_api::routes::admin::dashboard::csv_escape;
use vaultscope_api::routes::admin::internal_tools::validate_tool_url;
use vaultscope_api::routes::admin::settings::{
    is_empty_or_masked_secret, is_secret_setting_key, mask_settings_for_response,
};
use vaultscope_common::crypto::{
    decrypt_secret_string, encrypt_secret_string, generate_random_secret,
};
use vaultscope_common::types::{
    PowerAction, ProvisionParams, ProvisionResult, ReinstallParams, ResourceStatus,
};
use vaultscope_connectors::reliablesite::inventory_soap::DEFAULT_INVENTORY_URL;
use vaultscope_connectors::reliablesite::order_api::OrderApiClient;
use vaultscope_connectors::reliablesite::ReliableSite;
use vaultscope_connectors::sftpgo::{generate_sftpgo_username, validate_resource_id, SftpGo};
use vaultscope_connectors::{ConnectorResult, ProvisioningProvider};
use vaultscope_db::models::{
    Customer, CustomerStatus, Invoice, InvoiceBillingSnapshot, InvoiceLineItem, InvoiceStatus,
    Service, ServiceStatus, Setting,
};
use vaultscope_jobs::runner::{
    extract_provision_password, strip_provision_passwords, ConnectorRegistry, JobRunner,
};
use vaultscope_jobs::BillingRunner;

fn make_token(sub: Uuid, email: &str, role: &str, audience: Audience, secret: &str) -> String {
    let claims = Claims {
        sub,
        email: email.to_string(),
        role: role.to_string(),
        audience,
        iat: Utc::now().timestamp(),
        exp: jwt::new_expiry(),
        auth_version: Some(1),
        jti: None,
    };
    jwt::sign(&claims, secret).unwrap()
}

async fn get_csrf_cookie_and_token(app: &Router) -> (String, String) {
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
    let json_body: Value = serde_json::from_slice(&bytes).unwrap();
    let token = json_body["token"].as_str().unwrap().to_string();
    (set_cookie, token)
}

async fn insert_test_customer(pool: &sqlx::PgPool, label: &str) -> Uuid {
    let cust_id = Uuid::now_v7();
    let short = cust_id.simple().to_string();
    sqlx::query(
        "INSERT INTO customers (id, external_id, username, name, email, status, email_verified) VALUES ($1, $2, $3, $4, $5, 'active', true)",
    )
    .bind(cust_id)
    .bind(format!("cust_{label}_{short}"))
    .bind(format!("u_{label}_{}", &short[20..]))
    .bind(format!("Test Customer {label}"))
    .bind(format!("{label}_{}@test.local", &short[20..]))
    .execute(pool)
    .await
    .unwrap();
    cust_id
}

async fn insert_test_admin(pool: &sqlx::PgPool, label: &str) -> Uuid {
    let staff_id = Uuid::now_v7();
    let short = staff_id.simple().to_string();
    let role_id: Uuid = sqlx::query_scalar("SELECT id FROM roles WHERE '*' = ANY(permissions) LIMIT 1")
        .fetch_one(pool)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO staff (id, external_id, username, first_name, last_name, email, email_verified, role_id) VALUES ($1, $2, $3, 'Sec', 'Admin', $4, true, $5)",
    )
    .bind(staff_id)
    .bind(format!("adm_{label}_{short}"))
    .bind(format!("a_{label}_{}", &short[20..]))
    .bind(format!("adm_{label}_{}@test.local", &short[20..]))
    .bind(role_id)
    .execute(pool)
    .await
    .unwrap();
    staff_id
}

#[test]
fn unit_csprng_secret_entropy_and_sftpgo_username_safety() {
    let mut seen = HashSet::new();
    for _ in 0..50 {
        let s = generate_random_secret(24);
        assert_eq!(s.len(), 24);
        assert!(s.bytes().all(|b| b.is_ascii_alphanumeric()));
        assert!(seen.insert(s), "CSPRNG generated duplicate 24-char secret");
    }

    // SFTPGo username generation handles short, unknown, non-ASCII, and UUID without panics or collisions
    assert!(generate_sftpgo_username(Some("unknown")).starts_with("vs_nknown_"));
    assert!(generate_sftpgo_username(None).starts_with("vs_nknown_"));
    assert!(generate_sftpgo_username(Some("x")).starts_with("vs_x_"));
    assert!(generate_sftpgo_username(Some("äöüß---")).starts_with("vs_unknwn_"));

    let cust_uuid = "01948a2b-1234-7000-8000-abcdef123456";
    let mut usernames = HashSet::new();
    for _ in 0..20 {
        let u = generate_sftpgo_username(Some(cust_uuid));
        assert!(u.starts_with("vs_123456_"));
        assert!(validate_resource_id(&u).is_ok());
        assert!(usernames.insert(u), "SFTPGo usernames must be collision-free");
    }

    // Traversal & invalid resource_id checks
    for bad in ["", ".", "..", "../root", "u/1", "u?x=1", "u#frag", "u\n1", "u 1"] {
        assert!(validate_resource_id(bad).is_err(), "Should reject resource_id {bad:?}");
    }
}

#[tokio::test]
async fn unit_sftpgo_provision_uses_csprng_and_unique_username() {
    let captured_users: Arc<Mutex<Vec<Value>>> = Arc::new(Mutex::new(Vec::new()));
    let cap_ref = captured_users.clone();

    let mock_sftpgo = Router::new()
        .route(
            "/api/v2/token",
            axum::routing::get(|| async { Json(json!({ "access_token": "tok_mock" })) }),
        )
        .route(
            "/api/v2/users",
            post(move |Json(body): Json<Value>| {
                let cap = cap_ref.clone();
                async move {
                    cap.lock().unwrap().push(body.clone());
                    Json(json!({ "id": 1, "username": body["username"] }))
                }
            }),
        );

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, mock_sftpgo).await.unwrap();
    });

    let connector = SftpGo::new(
        format!("http://{addr}"),
        "admin".to_string(),
        "admin_pass".to_string(),
    );

    let params = ProvisionParams {
        server_type: "sftp".to_string(),
        location: "fra1".to_string(),
        image: "".to_string(),
        ssh_keys: vec![],
        name: "sftp.vaultscope.de".to_string(),
        extra: json!({ "customer_id": "01948a2b-1234-7000-8000-abcdef987654" }),
    };

    let r1 = connector.provision(&params).await.unwrap();
    let r2 = connector.provision(&params).await.unwrap();

    assert_ne!(r1.resource_id, r2.resource_id);
    assert!(r1.resource_id.starts_with("vs_987654_"));

    let pw1 = r1.raw_response["password"].as_str().unwrap();
    let pw2 = r2.raw_response["password"].as_str().unwrap();
    assert_eq!(pw1.len(), 24);
    assert_eq!(pw2.len(), 24);
    assert!(pw1.bytes().all(|b| b.is_ascii_alphanumeric()));
    assert_ne!(pw1, pw2);

    // Path traversal blocked before HTTP request in power_action/status/delete/modify
    assert!(connector.power_action("../admin", PowerAction::Stop).await.is_err());
    assert!(connector.status("user?foo=bar").await.is_err());
    assert!(connector.modify("..", &json!({"quota_size": 1})).await.is_err());
}

#[tokio::test]
async fn unit_reliablesite_hardening_scrubs_apikey_uses_https_and_csprng_ipmi() {
    assert!(
        DEFAULT_INVENTORY_URL.starts_with("https://"),
        "DEFAULT_INVENTORY_URL must use HTTPS"
    );

    // Bind and immediately drop a TCP listener to obtain an unused closed port
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let unused_addr = listener.local_addr().unwrap();
    drop(listener);

    let secret_api_key = "rs_super_secret_apikey_must_never_leak_999";
    let client = OrderApiClient::new(
        reqwest::Client::new(),
        format!("http://{unused_addr}"),
        secret_api_key.to_string(),
    );

    let err = client.login_token().await.unwrap_err();
    let err_msg = err.to_string();
    assert!(
        !err_msg.contains(secret_api_key) && !err_msg.contains("apiKey="),
        "Transport error leaked apiKey: {err_msg}"
    );
    assert!(err_msg.contains("ReliableSite Order API request failed"));

    let rs = ReliableSite::new("".to_string(), "https://api.reliablesite.net".to_string());
    let s1 = rs.request_ipmi_session("ds-nl-123456", Some("198.51.100.1")).await.unwrap();
    let s2 = rs.request_ipmi_session("ds-nl-123456", Some("198.51.100.1")).await.unwrap();
    let p1 = s1["ipmi_password"].as_str().unwrap();
    let p2 = s2["ipmi_password"].as_str().unwrap();
    assert_eq!(p1.len(), 16);
    assert_ne!(p1, p2);
    assert!(!p1.starts_with("Vs#Kvm_"));
}

#[test]
fn unit_mailer_header_sanitization_and_service_sanitization() {
    let injected = "Hello\r\nBcc: attacker@evil.com\nX-Test: 1\0";
    assert_eq!(sanitize_email_header(injected), "HelloBcc: attacker@evil.comX-Test: 1");

    let svc = Service {
        id: Uuid::now_v7(),
        customer_id: Uuid::now_v7(),
        product_id: Uuid::now_v7(),
        name: "SFTP Vault".to_string(),
        status: ServiceStatus::Running,
        provider_resource_id: "vs_abc_123".to_string(),
        ip: "176.9.0.171".to_string(),
        hostname: "sftp.vaultscope.de".to_string(),
        config: json!({
            "password": "top-secret-password",
            "root_password": "root-secret-password",
            "provider": "reliablesite",
            "reliablesite_product_id": 428,
            "raw_response": {
                "password": "nested-secret-password",
                "root_password": "nested-root-password",
                "sftpgo_user": {
                    "username": "vs_abc_123",
                    "password": "sftpgo-user-password"
                }
            }
        }),
        price: Decimal::new(1200, 2),
        next_due: None,
        created_at: Utc::now(),
        updated_at: Utc::now(),
        cancel_at_period_end: false,
        cancellation_reason: None,
        cancellation_feedback: None,
        stripe_payment_intent_id: None,
        is_trial: false,
        trial_ends_at: None,
        partner_id: None,
        quota_gb: Some(150),
        order_state: "active".to_string(),
        order_state_note: None,
        order_state_updated_at: None,
    };

    let sanitized = sanitize_service_for_storefront(svc);
    let serialized = serde_json::to_string(&sanitized).unwrap();
    for secret in [
        "top-secret-password",
        "root-secret-password",
        "nested-secret-password",
        "nested-root-password",
        "sftpgo-user-password",
    ] {
        assert!(!serialized.contains(secret), "Service sanitization leaked {secret}");
    }
}

#[test]
fn unit_pdf_uses_billing_snapshot_and_actual_tax_rate() {
    let cust_id = Uuid::now_v7();
    let customer = Customer {
        id: cust_id,
        external_id: "ext_pdf".to_string(),
        name: "Live Name Should Be Overridden".to_string(),
        email: "pdf@example.com".to_string(),
        phone: String::new(),
        company: "Live Company Should Be Overridden".to_string(),
        address: "Old Street 1".to_string(),
        postal_code: "10000".to_string(),
        city: "OldCity".to_string(),
        country: "OldCountry".to_string(),
        vat_id: "DE000000000".to_string(),
        status: CustomerStatus::Active,
        two_factor_enabled: false,
        email_verified: true,
        notes: String::new(),
        stripe_customer_id: None,
        created_at: Utc::now(),
        last_login: None,
        updated_at: Utc::now(),
        password_hash: None,
        verification_token: None,
        totp_secret: None,
        vat_verified: true,
        username: "pdfuser".to_string(),
    };

    let snapshot = InvoiceBillingSnapshot {
        name: "Frozen Snapshot Person".to_string(),
        company: "Frozen Snapshot GmbH".to_string(),
        address_line1: "Snapshot Strasse 42".to_string(),
        postal_code: "74343".to_string(),
        city: "Sachsenheim".to_string(),
        country: "Deutschland".to_string(),
        vat_id: "DE999888777".to_string(),
        snapshot_complete: true,
        missing_fields: vec![],
        snapshot_source: "test".to_string(),
        captured_at: Some(Utc::now()),
    };

    let invoice = Invoice {
        id: Uuid::now_v7(),
        invoice_number: "INV-202611-001001".to_string(),
        customer_id: Some(cust_id),
        status: InvoiceStatus::Paid,
        subtotal: Decimal::new(10000, 2),
        tax_rate: Decimal::new(0, 2),
        tax_amount: Decimal::new(0, 2),
        total: Decimal::new(10000, 2),
        issue_date: NaiveDate::from_ymd_opt(2026, 11, 19).unwrap(),
        due_date: NaiveDate::from_ymd_opt(2026, 12, 3).unwrap(),
        paid_at: Some(Utc::now()),
        stripe_payment_intent_id: None,
        notes: String::new(),
        billing_snapshot: Some(sqlx::types::Json(snapshot)),
        tax_regime: "kleinunternehmer_19_ustg".to_string(),
        retention_until: None,
        retention_hold: false,
        corrects_invoice_id: None,
        retention_resolution_approved: false,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };

    let items = vec![InvoiceLineItem {
        id: Uuid::now_v7(),
        invoice_id: invoice.id,
        description: "Storage Plan".to_string(),
        quantity: 1,
        unit_price: Decimal::new(10000, 2),
        total: Decimal::new(10000, 2),
        service_id: None,
        sort_order: 1,
    }];

    let pdf_bytes = generate_invoice_pdf(&invoice, &customer, &items);
    let pdf_str = String::from_utf8_lossy(&pdf_bytes);

    assert!(pdf_str.contains("Frozen Snapshot GmbH"));
    assert!(pdf_str.contains("Frozen Snapshot Person"));
    assert!(pdf_str.contains("Snapshot Strasse 42"));
    assert!(pdf_str.contains("74343 Sachsenheim"));
    assert!(pdf_str.contains("DE999888777"));
    assert!(!pdf_str.contains("Live Company Should Be Overridden"));
    assert!(!pdf_str.contains("Old Street 1"));
    assert!(pdf_str.contains("USt. (Par. 19 UStG):"));
    assert!(pdf_str.contains("Paragraph 19 UStG"));
    assert!(!pdf_str.contains("zzgl. 19% USt. / VAT:"));

    // Verify both 0.19 and 19.00 tax_rate representations render "19%"
    let mut inv_frac = invoice.clone();
    inv_frac.tax_rate = Decimal::new(19, 2);
    let pdf_frac = generate_invoice_pdf(&inv_frac, &customer, &items);
    assert!(String::from_utf8_lossy(&pdf_frac).contains("zzgl. 19% USt. / VAT:"));

    let mut inv_pct = invoice;
    inv_pct.tax_rate = Decimal::new(1900, 2);
    let pdf_pct = generate_invoice_pdf(&inv_pct, &customer, &items);
    assert!(String::from_utf8_lossy(&pdf_pct).contains("zzgl. 19% USt. / VAT:"));
}

#[test]
fn unit_admin_validators_csv_url_coupon_and_settings_masking() {
    // CSV formula escaping
    assert_eq!(csv_escape("safe@example.com"), "safe@example.com");
    assert_eq!(csv_escape("=1+1"), "'=1+1");
    assert_eq!(csv_escape("+cmd"), "'+cmd");
    assert_eq!(csv_escape("-cmd"), "'-cmd");
    assert_eq!(csv_escape("@SUM(1)"), "'@SUM(1)");
    assert_eq!(csv_escape("\t=1"), "'\t=1");
    assert_eq!(csv_escape("\r=1"), "\"'\r=1\"");
    assert_eq!(csv_escape("=1,2"), "\"'=1,2\"");

    // Internal tool URL validation
    assert!(validate_tool_url("https://grafana.vaultscope.de/dash").is_ok());
    assert!(validate_tool_url("http://10.0.0.5:3000").is_ok());
    for bad_url in [
        "",
        "   ",
        "javascript:alert(1)",
        "data:text/html,<script>alert(1)</script>",
        "file:///etc/passwd",
        "ftp://example.com/a",
        "https://example.com/bad\nline",
        "https://example.com/has space",
        "https://",
    ] {
        assert!(validate_tool_url(bad_url).is_err(), "Expected rejection for URL {bad_url:?}");
    }

    // Coupon code validation
    assert_eq!(validate_coupon_code(" save_20-eu ").unwrap(), "SAVE_20-EU");
    for bad_code in ["", "   ", "BAD CODE", "BAD/CODE", "BAD?X=1", "../evil"] {
        assert!(validate_coupon_code(bad_code).is_err(), "Expected rejection for code {bad_code:?}");
    }

    // Settings secret detection & masking
    for sk in [
        "smtpPass",
        "smtp_password",
        "oidcClientSecret",
        "stripe_live_secret_key",
        "stripe_secret_key",
        "stripe_webhook_secret",
        "stripe_sandbox_secret_key",
        "custom_api_key",
        "auth_token",
    ] {
        assert!(is_secret_setting_key(sk), "Expected {sk} to be classified as secret");
    }
    for pk in ["companyName", "stripe_live_public_key", "stripe_sandbox_public_key", "smtpHost"] {
        assert!(!is_secret_setting_key(pk), "Expected {pk} to be non-secret");
    }

    assert!(is_empty_or_masked_secret(&json!("")));
    assert!(is_empty_or_masked_secret(&json!("   ")));
    assert!(is_empty_or_masked_secret(&json!("••••••")));
    assert!(is_empty_or_masked_secret(&json!("***")));
    assert!(is_empty_or_masked_secret(&json!("__UNCHANGED__")));
    assert!(is_empty_or_masked_secret(&Value::Null));
    assert!(!is_empty_or_masked_secret(&json!("sk_live_actual_secret")));

    let now = Utc::now();
    let masked = mask_settings_for_response(vec![
        Setting {
            key: "companyName".to_string(),
            value: json!("VaultScope"),
            updated_at: now,
        },
        Setting {
            key: "smtpPass".to_string(),
            value: json!("real-smtp-pass"),
            updated_at: now,
        },
        Setting {
            key: "oidcClientSecret".to_string(),
            value: json!(""),
            updated_at: now,
        },
    ]);
    let find_val = |k: &str| masked.iter().find(|s| s.key == k).map(|s| s.value.clone());
    assert_eq!(find_val("companyName"), Some(json!("VaultScope")));
    assert_eq!(find_val("smtpPass"), Some(json!("")));
    assert_eq!(find_val("smtpPass_configured"), Some(json!(true)));
    assert_eq!(find_val("oidcClientSecret"), Some(json!("")));
    assert_eq!(find_val("oidcClientSecret_configured"), Some(json!(false)));
}

struct MockSftpProvisionProvider {
    password: String,
    username: String,
}

#[async_trait::async_trait]
impl ProvisioningProvider for MockSftpProvisionProvider {
    fn name(&self) -> &'static str {
        "sftpgo"
    }
    async fn test_connection(&self) -> ConnectorResult<()> {
        Ok(())
    }
    async fn provision(&self, _params: &ProvisionParams) -> ConnectorResult<ProvisionResult> {
        Ok(ProvisionResult {
            resource_id: self.username.clone(),
            ip: "176.9.0.171".to_string(),
            hostname: "sftp.vaultscope.de".to_string(),
            raw_response: json!({
                "password": self.password,
                "sftpgo_user": { "username": self.username, "password": self.password }
            }),
        })
    }
    async fn power_action(&self, _resource_id: &str, _action: PowerAction) -> ConnectorResult<()> {
        Ok(())
    }
    async fn status(&self, _resource_id: &str) -> ConnectorResult<ResourceStatus> {
        Ok(ResourceStatus::Running)
    }
    async fn delete(&self, _resource_id: &str) -> ConnectorResult<()> {
        Ok(())
    }
    async fn reinstall(&self, _resource_id: &str, _params: &ReinstallParams) -> ConnectorResult<()> {
        Ok(())
    }
    async fn modify(&self, _resource_id: &str, _params: &Value) -> ConnectorResult<()> {
        Ok(())
    }
}

struct MockRegistry(Arc<dyn ProvisioningProvider>);

#[async_trait::async_trait]
impl ConnectorRegistry for MockRegistry {
    async fn get(&self, provider: &str) -> Option<Arc<dyn ProvisioningProvider>> {
        if provider == "sftpgo" {
            Some(self.0.clone())
        } else {
            None
        }
    }
}

#[tokio::test]
async fn db_job_runner_encrypts_password_strips_plaintext_and_credentials_endpoint_decrypts() {
    let state = common::test_app_state().await;
    let owner_id = insert_test_customer(&state.db, "cred_owner").await;
    let other_id = insert_test_customer(&state.db, "cred_other").await;

    let sftp_prod_id = Uuid::from_str("d9d9d9d9-d9d9-d9d9-d9d9-d9d9d9d9d9d9").unwrap();
    let svc_id = Uuid::now_v7();
    let job_id = Uuid::now_v7();
    let expected_pw = "PlaintextSecretSftpPw24X";
    let expected_user = format!("vs_test_{}", &svc_id.simple().to_string()[24..]);

    sqlx::query(
        "INSERT INTO services (id, customer_id, product_id, name, status, price) VALUES ($1, $2, $3, 'SFTP Vault Test', 'pending', 0.00)",
    )
    .bind(svc_id)
    .bind(owner_id)
    .bind(sftp_prod_id)
    .execute(&state.db)
    .await
    .unwrap();

    sqlx::query(
        r#"INSERT INTO jobs (id, task, target_api, customer_id, service_id, status, request_payload, scheduled_at)
           VALUES ($1, 'provision', 'sftpgo', $2, $3, 'queued', $4, now() - interval '100 years')"#,
    )
    .bind(job_id)
    .bind(owner_id)
    .bind(svc_id)
    .bind(json!({
        "server_type": "sftp",
        "location": "fra1",
        "image": "",
        "ssh_keys": [],
        "name": "sftp.vaultscope.de",
        "extra": { "customer_id": owner_id.to_string() }
    }))
    .execute(&state.db)
    .await
    .unwrap();

    let provider = Arc::new(MockSftpProvisionProvider {
        password: expected_pw.to_string(),
        username: expected_user.clone(),
    });
    let registry: Arc<dyn ConnectorRegistry> = Arc::new(MockRegistry(provider));

    // Also test helper functions directly
    let sample_resp = json!({
        "resource_id": expected_user,
        "raw_response": { "password": expected_pw, "root_password": "RootPw123" }
    });
    assert_eq!(extract_provision_password(&sample_resp).as_deref(), Some(expected_pw));
    let stripped = strip_provision_passwords(&sample_resp);
    assert!(!stripped.to_string().contains(expected_pw));
    assert!(!stripped.to_string().contains("RootPw123"));

    let runner = JobRunner::new(state.db.clone(), registry, Duration::from_secs(60))
        .with_encryption_key(state.config.encryption_key_bytes());
    let ran = runner.run_one_inline().await.unwrap();
    assert!(ran, "JobRunner should claim and execute the queued provision job");

    // Verify DB: jobs.response_payload and services.config must NOT contain plaintext password; services.encrypted_password MUST be encrypted
    let job_resp: Option<Value> = sqlx::query_scalar("SELECT response_payload FROM jobs WHERE id = $1")
        .bind(job_id)
        .fetch_one(&state.db)
        .await
        .unwrap();
    let job_resp_str = serde_json::to_string(&job_resp).unwrap();
    assert!(
        !job_resp_str.contains(expected_pw),
        "jobs.response_payload must not store plaintext password"
    );

    let (svc_config, enc_pw): (Value, Option<String>) =
        sqlx::query_as("SELECT config, encrypted_password FROM services WHERE id = $1")
            .bind(svc_id)
            .fetch_one(&state.db)
            .await
            .unwrap();
    assert!(
        !svc_config.to_string().contains(expected_pw),
        "services.config must not store plaintext password"
    );
    let enc_pw_val = enc_pw.expect("services.encrypted_password must be populated");
    assert!(!enc_pw_val.contains(expected_pw));
    assert_eq!(
        decrypt_secret_string(&state.config.encryption_key_bytes(), &enc_pw_val).unwrap(),
        expected_pw
    );

    let owner_jwt = make_token(
        owner_id,
        "owner@test.local",
        "customer",
        Audience::Storefront,
        &state.config.jwt_secret,
    );
    let other_jwt = make_token(
        other_id,
        "other@test.local",
        "customer",
        Audience::Storefront,
        &state.config.jwt_secret,
    );
    let app = vaultscope_api::routes::build_router(state.clone());

    // GET /api/storefront/services/{id}/credentials returns decrypted credentials with Cache-Control: no-store
    let cred_resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/api/storefront/services/{svc_id}/credentials"))
                .header(header::AUTHORIZATION, format!("Bearer {owner_jwt}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(cred_resp.status(), StatusCode::OK);
    assert_eq!(
        cred_resp
            .headers()
            .get(header::CACHE_CONTROL)
            .and_then(|v| v.to_str().ok()),
        Some("no-store")
    );
    let cred_json: Value =
        serde_json::from_slice(&to_bytes(cred_resp.into_body(), 1024 * 1024).await.unwrap())
            .unwrap();
    assert_eq!(cred_json["username"], expected_user);
    assert_eq!(cred_json["password"], expected_pw);

    // IDOR check: another customer gets 404
    let idor_resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/api/storefront/services/{svc_id}/credentials"))
                .header(header::AUTHORIZATION, format!("Bearer {other_jwt}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(idor_resp.status(), StatusCode::NOT_FOUND);

    // Legacy fallback check: encrypted_password = NULL, password in config
    let legacy_svc_id = Uuid::now_v7();
    let legacy_pw = "LegacyPlainPw99";
    sqlx::query(
        "INSERT INTO services (id, customer_id, product_id, name, status, provider_resource_id, config, price) VALUES ($1, $2, $3, 'Legacy SFTP', 'running', 'vs_legacy_1', $4, 0.00)",
    )
    .bind(legacy_svc_id)
    .bind(owner_id)
    .bind(sftp_prod_id)
    .bind(json!({ "raw_response": { "password": legacy_pw } }))
    .execute(&state.db)
    .await
    .unwrap();

    // GET /api/storefront/services/{id} must strip legacy plaintext password
    let get_svc = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/api/storefront/services/{legacy_svc_id}"))
                .header(header::AUTHORIZATION, format!("Bearer {owner_jwt}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(get_svc.status(), StatusCode::OK);
    let svc_bytes = to_bytes(get_svc.into_body(), 1024 * 1024).await.unwrap();
    assert!(
        !String::from_utf8_lossy(&svc_bytes).contains(legacy_pw),
        "GET /services/{{id}} must strip legacy plaintext password"
    );

    // GET /api/storefront/services/{id}/credentials falls back to legacy password cleanly
    let leg_cred = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/api/storefront/services/{legacy_svc_id}/credentials"))
                .header(header::AUTHORIZATION, format!("Bearer {owner_jwt}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(leg_cred.status(), StatusCode::OK);
    let leg_json: Value =
        serde_json::from_slice(&to_bytes(leg_cred.into_body(), 1024 * 1024).await.unwrap())
            .unwrap();
    assert_eq!(leg_json["username"], "vs_legacy_1");
    assert_eq!(leg_json["password"], legacy_pw);

    // Also verify encrypt_secret_string & decrypt_secret_string compatibility
    let direct_enc = encrypt_secret_string(&state.config.encryption_key_bytes(), "Check123").unwrap();
    assert_eq!(
        decrypt_secret_string(&state.config.encryption_key_bytes(), &direct_enc).unwrap(),
        "Check123"
    );

    let _ = sqlx::query("DELETE FROM jobs WHERE id = $1").bind(job_id).execute(&state.db).await;
    let _ = sqlx::query("DELETE FROM services WHERE id IN ($1, $2)")
        .bind(svc_id)
        .bind(legacy_svc_id)
        .execute(&state.db)
        .await;
    let _ = sqlx::query("DELETE FROM customers WHERE id IN ($1, $2)")
        .bind(owner_id)
        .bind(other_id)
        .execute(&state.db)
        .await;
}

#[tokio::test]
async fn db_billing_runner_atomic_renewals_seq_numbering_and_tax_rounding() {
    let pool = common::setup_test_db().await;
    let cust_id = insert_test_customer(&pool, "billing").await;
    let prod_id = Uuid::now_v7();
    let svc_id = Uuid::now_v7();

    sqlx::query(
        r#"INSERT INTO products (id, name, category, provider, target, specs, cost, price, setup_fee, stock, user_limit, billing_cycle, hidden, quota_gb)
           VALUES ($1, 'SFTP Custom Price Test', 'sftp', 'sftpgo', 'sftp', '{}'::jsonb, 5.00, 19.99, 0.00, -1, 0, 'monthly', true, 250)"#,
    )
    .bind(prod_id)
    .execute(&pool)
    .await
    .unwrap();

    sqlx::query(
        r#"INSERT INTO services (id, customer_id, product_id, name, status, provider_resource_id, price, next_due, is_trial, quota_gb)
           VALUES ($1, $2, $3, 'SFTP Custom Renewal', 'running', 'vs_bill_test', 0.00, now() - interval '1 hour', true, 25)"#,
    )
    .bind(svc_id)
    .bind(cust_id)
    .bind(prod_id)
    .execute(&pool)
    .await
    .unwrap();

    let runner = BillingRunner::new(pool.clone(), Duration::from_secs(3600));
    runner.process_renewals().await.unwrap();

    // Service upgraded to product price (19.99, NOT hardcoded 12.00) and quota_gb 250
    let (new_price, is_trial, quota_gb): (Decimal, bool, Option<i32>) = sqlx::query_as(
        "SELECT price, is_trial, quota_gb FROM services WHERE id = $1",
    )
    .bind(svc_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(new_price, Decimal::from_str("19.99").unwrap());
    assert!(!is_trial);
    assert_eq!(quota_gb, Some(250));

    // Invoice uses INV-YYYYMM-NNNNNN sequence and safe fallback § 19 UStG 0% VAT (tax=0.00, total=19.99)
    let (inv_num, sub, tax_rate, tax, total): (String, Decimal, Decimal, Decimal, Decimal) = sqlx::query_as(
        "SELECT invoice_number, subtotal, tax_rate, tax_amount, total FROM invoices WHERE customer_id = $1",
    )
    .bind(cust_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    let expected_prefix = format!("INV-{}-", Utc::now().format("%Y%m"));
    assert!(
        inv_num.starts_with(&expected_prefix) && inv_num.len() >= expected_prefix.len() + 6,
        "Expected sequential INV-YYYYMM-NNNNNN invoice number, got {inv_num}"
    );
    assert_eq!(sub, Decimal::from_str("19.99").unwrap());
    assert_eq!(tax_rate, Decimal::from_str("0.00").unwrap());
    assert_eq!(tax, Decimal::from_str("0.00").unwrap());
    assert_eq!(total, Decimal::from_str("19.99").unwrap());

    // Running again must NOT duplicate invoice
    runner.process_renewals().await.unwrap();
    let inv_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM invoices WHERE customer_id = $1")
        .bind(cust_id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(inv_count, 1);

    let _ = sqlx::query("DELETE FROM invoice_line_items WHERE invoice_id IN (SELECT id FROM invoices WHERE customer_id = $1)").bind(cust_id).execute(&pool).await;
    let _ = sqlx::query("DELETE FROM invoices WHERE customer_id = $1").bind(cust_id).execute(&pool).await;

    let _ = sqlx::query("DELETE FROM jobs WHERE service_id = $1").bind(svc_id).execute(&pool).await;
    let _ = sqlx::query("DELETE FROM services WHERE id = $1").bind(svc_id).execute(&pool).await;
    let _ = sqlx::query("DELETE FROM products WHERE id = $1").bind(prod_id).execute(&pool).await;
    let _ = sqlx::query("DELETE FROM customers WHERE id = $1").bind(cust_id).execute(&pool).await;
}

#[tokio::test]
async fn db_admin_endpoints_and_storefront_partners_hardening() {
    let state = common::test_app_state().await;
    let admin_id = insert_test_admin(&state.db, "hardening").await;
    let cust_id = insert_test_customer(&state.db, "admin_inv").await;

    let admin_jwt = make_token(
        admin_id,
        "admin@test.local",
        "admin",
        Audience::Admin,
        &state.config.jwt_secret,
    );
    let app = vaultscope_api::routes::build_router(state.clone());
    let (cookie, csrf) = get_csrf_cookie_and_token(&app).await;

    // 1. Admin invoices: create with Decimal math & sequence, block non-draft update, mark_paid 404 & void guard, list clamp
    let create_inv = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/admin/invoices")
                .header(header::AUTHORIZATION, format!("Bearer {admin_jwt}"))
                .header(header::COOKIE, &cookie)
                .header("x-csrf-token", &csrf)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    json!({
                        "customer_id": cust_id,
                        "due_date": "2026-12-31",
                        "tax_rate": 19,
                        "notes": "Draft test",
                        "line_items": [
                            { "description": "Node A", "quantity": 1, "unit_price": 8.50 }
                        ]
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(create_inv.status(), StatusCode::OK);
    let inv_json: Value =
        serde_json::from_slice(&to_bytes(create_inv.into_body(), 1024 * 1024).await.unwrap())
            .unwrap();
    let inv_id = Uuid::from_str(inv_json["id"].as_str().unwrap()).unwrap();
    assert!(inv_json["invoice_number"]
        .as_str()
        .unwrap()
        .starts_with(&format!("INV-{}-", Utc::now().format("%Y%m"))));
    assert_eq!(inv_json["subtotal"], "8.50");
    assert_eq!(inv_json["tax_amount"], "1.62");
    assert_eq!(inv_json["total"], "10.12");

    // Draft update succeeds and transitions to pending
    let upd_draft = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri(format!("/api/admin/invoices/{inv_id}"))
                .header(header::AUTHORIZATION, format!("Bearer {admin_jwt}"))
                .header(header::COOKIE, &cookie)
                .header("x-csrf-token", &csrf)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(json!({ "status": "pending", "notes": "Finalized" }).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(upd_draft.status(), StatusCode::OK);

    // Updating non-draft invoice MUST fail with 400 BadRequest
    let upd_nondraft = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri(format!("/api/admin/invoices/{inv_id}"))
                .header(header::AUTHORIZATION, format!("Bearer {admin_jwt}"))
                .header(header::COOKIE, &cookie)
                .header("x-csrf-token", &csrf)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(json!({ "notes": "Tamper" }).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(upd_nondraft.status(), StatusCode::BAD_REQUEST);

    // mark_paid on missing invoice returns 404
    let missing_id = Uuid::now_v7();
    let paid_404 = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/admin/invoices/{missing_id}/mark-paid"))
                .header(header::AUTHORIZATION, format!("Bearer {admin_jwt}"))
                .header(header::COOKIE, &cookie)
                .header("x-csrf-token", &csrf)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(paid_404.status(), StatusCode::NOT_FOUND);

    // mark_paid on voided invoice returns 400
    let void_inv_id = Uuid::now_v7();
    sqlx::query(
        r#"INSERT INTO invoices (id, invoice_number, customer_id, status, subtotal, tax_rate, tax_amount, total, issue_date, due_date)
           VALUES ($1, $2, $3, 'void', 10.00, 19.00, 1.90, 11.90, CURRENT_DATE, CURRENT_DATE + 14)"#,
    )
    .bind(void_inv_id)
    .bind(format!("INV-VOID-{}", &void_inv_id.simple().to_string()[20..]))
    .bind(cust_id)
    .execute(&state.db)
    .await
    .unwrap();

    let paid_void = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/admin/invoices/{void_inv_id}/mark-paid"))
                .header(header::AUTHORIZATION, format!("Bearer {admin_jwt}"))
                .header(header::COOKIE, &cookie)
                .header("x-csrf-token", &csrf)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(paid_void.status(), StatusCode::BAD_REQUEST);

    // Admin invoices list & activity limit clamping (negative limits don't 500)
    let list_inv = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/admin/invoices?limit=-5&offset=-10")
                .header(header::AUTHORIZATION, format!("Bearer {admin_jwt}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(list_inv.status(), StatusCode::OK);

    let list_act = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/admin/activity?limit=-5")
                .header(header::AUTHORIZATION, format!("Bearer {admin_jwt}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(list_act.status(), StatusCode::OK);

    // 2. Admin blog: empty title must fail with 400 BadRequest (regression for swapped validate_required_string args)
    let bad_blog = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/admin/blog")
                .header(header::AUTHORIZATION, format!("Bearer {admin_jwt}"))
                .header(header::COOKIE, &cookie)
                .header("x-csrf-token", &csrf)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    json!({
                        "title": "   ",
                        "slug": "valid-slug",
                        "excerpt": "Valid excerpt",
                        "content": "Valid content"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(bad_blog.status(), StatusCode::BAD_REQUEST);

    // 3. Admin internal tools: reject javascript: URLs, accept https://
    let bad_tool = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/admin/internal-tools")
                .header(header::AUTHORIZATION, format!("Bearer {admin_jwt}"))
                .header(header::COOKIE, &cookie)
                .header("x-csrf-token", &csrf)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    json!({
                        "name": "Evil XSS Link",
                        "url": "javascript:alert(document.cookie)",
                        "required_permission": "staff.view"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(bad_tool.status(), StatusCode::BAD_REQUEST);

    let ok_tool = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/admin/internal-tools")
                .header(header::AUTHORIZATION, format!("Bearer {admin_jwt}"))
                .header(header::COOKIE, &cookie)
                .header("x-csrf-token", &csrf)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    json!({
                        "name": "Grafana Test",
                        "url": "https://grafana.vaultscope.de",
                        "required_permission": "staff.view"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(ok_tool.status(), StatusCode::OK);
    let tool_json: Value =
        serde_json::from_slice(&to_bytes(ok_tool.into_body(), 1024 * 1024).await.unwrap())
            .unwrap();
    let tool_id = Uuid::from_str(tool_json["id"].as_str().unwrap()).unwrap();

    let bad_tool_upd = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri(format!("/api/admin/internal-tools/{tool_id}"))
                .header(header::AUTHORIZATION, format!("Bearer {admin_jwt}"))
                .header(header::COOKIE, &cookie)
                .header("x-csrf-token", &csrf)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(json!({ "url": "data:text/html,<h1>x</h1>" }).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(bad_tool_upd.status(), StatusCode::BAD_REQUEST);

    // 4. Storefront partners: hidden product returns 404; visible product is sanitized (cost=0, white-labeled)
    let hidden_prod_id = Uuid::now_v7();
    let visible_rs_prod_id = Uuid::now_v7();
    sqlx::query(
        r#"INSERT INTO products (id, name, category, provider, target, specs, cost, price, setup_fee, stock, user_limit, billing_cycle, hidden)
           VALUES ($1, 'Hidden Partner Product', 'dedicated', 'reliablesite', '428', '{"reliablesite_product_id": 428}'::jsonb, 180.00, 299.00, 0.00, 5, 0, 'monthly', true),
                  ($2, 'Visible Partner Product', 'dedicated', 'reliablesite', '428', '{"reliablesite_product_id": 428}'::jsonb, 180.00, 299.00, 0.00, 5, 0, 'monthly', false)"#,
    )
    .bind(hidden_prod_id)
    .bind(visible_rs_prod_id)
    .execute(&state.db)
    .await
    .unwrap();

    let hidden_partner_id = Uuid::now_v7();
    let hidden_slug = format!("hidden-p-{}", &hidden_partner_id.simple().to_string()[20..]);
    let visible_partner_id = Uuid::now_v7();
    let visible_slug = format!("visible-p-{}", &visible_partner_id.simple().to_string()[20..]);

    sqlx::query(
        "INSERT INTO partners (id, slug, name, product_id, active) VALUES ($1, $2, 'Hidden Partner', $3, true), ($4, $5, 'Visible Partner', $6, true)",
    )
    .bind(hidden_partner_id)
    .bind(&hidden_slug)
    .bind(hidden_prod_id)
    .bind(visible_partner_id)
    .bind(&visible_slug)
    .bind(visible_rs_prod_id)
    .execute(&state.db)
    .await
    .unwrap();

    let hidden_p_resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/api/storefront/partners/{hidden_slug}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(hidden_p_resp.status(), StatusCode::NOT_FOUND);

    let visible_p_resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/api/storefront/partners/{visible_slug}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(visible_p_resp.status(), StatusCode::OK);
    let p_bytes = to_bytes(visible_p_resp.into_body(), 1024 * 1024).await.unwrap();
    let p_str = String::from_utf8_lossy(&p_bytes).to_ascii_lowercase();
    assert!(!p_str.contains("reliablesite"), "Partner response leaked upstream provider");
    let p_json: Value = serde_json::from_slice(&p_bytes).unwrap();
    assert_eq!(p_json["product"]["provider"], "vaultscope_dedicated");
    assert_eq!(p_json["product"]["cost"], "0");

    // 5. Admin settings: secret masking in GET and preserve-on-empty/masked in PUT
    let test_secret_key = format!("test_hardening_api_key_{}", &admin_id.simple().to_string()[20..]);
    let test_public_key = format!("test_hardening_label_{}", &admin_id.simple().to_string()[20..]);

    let put_initial = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri("/api/admin/settings")
                .header(header::AUTHORIZATION, format!("Bearer {admin_jwt}"))
                .header(header::COOKIE, &cookie)
                .header("x-csrf-token", &csrf)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    json!([
                        { "key": test_secret_key, "value": "real-secret-value-xyz" },
                        { "key": test_public_key, "value": "initial-label" }
                    ])
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(put_initial.status(), StatusCode::OK);

    // Blank/masked save MUST preserve existing secret in DB
    let put_blank = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri("/api/admin/settings")
                .header(header::AUTHORIZATION, format!("Bearer {admin_jwt}"))
                .header(header::COOKIE, &cookie)
                .header("x-csrf-token", &csrf)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    json!([
                        { "key": test_secret_key, "value": "" },
                        { "key": test_public_key, "value": "updated-label" }
                    ])
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(put_blank.status(), StatusCode::OK);

    let put_masked = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri("/api/admin/settings")
                .header(header::AUTHORIZATION, format!("Bearer {admin_jwt}"))
                .header(header::COOKIE, &cookie)
                .header("x-csrf-token", &csrf)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    json!([
                        { "key": test_secret_key, "value": "__UNCHANGED__" }
                    ])
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(put_masked.status(), StatusCode::OK);

    let stored_secret: Value = sqlx::query_scalar("SELECT value FROM settings WHERE key = $1")
        .bind(&test_secret_key)
        .fetch_one(&state.db)
        .await
        .unwrap();
    assert_eq!(stored_secret, json!("real-secret-value-xyz"));

    // GET /api/admin/settings must mask secret value and return <key>_configured: true
    let get_settings = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/admin/settings")
                .header(header::AUTHORIZATION, format!("Bearer {admin_jwt}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(get_settings.status(), StatusCode::OK);
    let settings_bytes = to_bytes(get_settings.into_body(), 2 * 1024 * 1024).await.unwrap();
    assert!(
        !String::from_utf8_lossy(&settings_bytes).contains("real-secret-value-xyz"),
        "GET /admin/settings leaked plaintext secret"
    );
    let settings_arr: Vec<Value> = serde_json::from_slice(&settings_bytes).unwrap();
    let masked_entry = settings_arr.iter().find(|s| s["key"] == test_secret_key).unwrap();
    assert_eq!(masked_entry["value"], "");
    let conf_entry = settings_arr
        .iter()
        .find(|s| s["key"] == format!("{test_secret_key}_configured"))
        .unwrap();
    assert_eq!(conf_entry["value"], json!(true));
    let pub_entry = settings_arr.iter().find(|s| s["key"] == test_public_key).unwrap();
    assert_eq!(pub_entry["value"], "updated-label");

    // Cleanup synthetic test records
    let _ = sqlx::query("DELETE FROM settings WHERE key IN ($1, $2)")
        .bind(&test_secret_key)
        .bind(&test_public_key)
        .execute(&state.db)
        .await;
    let _ = sqlx::query("DELETE FROM partners WHERE id IN ($1, $2)")
        .bind(hidden_partner_id)
        .bind(visible_partner_id)
        .execute(&state.db)
        .await;
    let _ = sqlx::query("DELETE FROM products WHERE id IN ($1, $2)")
        .bind(hidden_prod_id)
        .bind(visible_rs_prod_id)
        .execute(&state.db)
        .await;
    let _ = sqlx::query("DELETE FROM internal_tools WHERE id = $1")
        .bind(tool_id)
        .execute(&state.db)
        .await;
    let _ = sqlx::query("DELETE FROM staff WHERE id = $1").bind(admin_id).execute(&state.db).await;
    let _ = sqlx::query("DELETE FROM customers WHERE id = $1").bind(cust_id).execute(&state.db).await;
}
