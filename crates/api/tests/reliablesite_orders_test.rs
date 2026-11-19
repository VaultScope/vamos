mod common;

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex};

use axum::body::{to_bytes, Body};
use axum::extract::Query;
use axum::http::{header, HeaderMap, Request, StatusCode};
use axum::routing::{get, post};
use axum::{Json, Router};
use rust_decimal::Decimal;
use serde_json::{json, Value};
use std::str::FromStr;
use tokio::net::TcpListener;
use tower::ServiceExt;
use uuid::Uuid;

use vaultscope_api::auth::{jwt, Audience, Claims};
use vaultscope_api::reliablesite_catalog::{
    DedicatedProductOptionsResponse, DedicatedQuote,
};
use vaultscope_connectors::reliablesite::ReliableSite;
use vaultscope_connectors::ProvisioningProvider;
use vaultscope_db::models::{Job, JobStatus, Product, ReliableSiteServer};
use vaultscope_jobs::runner::ConnectorRegistry;

fn make_token(sub: Uuid, email: &str, role: &str, audience: Audience, secret: &str) -> String {
    let claims = Claims {
        sub,
        email: email.to_string(),
        role: role.to_string(),
        audience,
        iat: chrono::Utc::now().timestamp(),
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

#[tokio::test]
async fn test_migration_0037_seeds_real_nl_servers_and_addons() {
    let pool = common::setup_test_db().await;

    let nl_servers: Vec<ReliableSiteServer> = sqlx::query_as(
        "SELECT * FROM reliablesite_servers WHERE data_center = 'NL' ORDER BY product_id ASC",
    )
    .fetch_all(&pool)
    .await
    .expect("query reliablesite_servers");

    assert_eq!(nl_servers.len(), 6, "Expected all 6 NL servers from ReliableSite");
    let pids: Vec<i32> = nl_servers.iter().map(|s| s.product_id).collect();
    assert_eq!(pids, vec![424, 425, 426, 427, 428, 444]);

    let s428 = nl_servers.iter().find(|s| s.product_id == 428).unwrap();
    assert_eq!(
        s428.description,
        "Rapid Deploy Server - AMD Epyc 4545P 128GB (NL)"
    );
    assert_eq!(
        s428.recurring_1_month,
        Decimal::from_str("299.0000").unwrap()
    );
    assert_eq!(
        s428.recurring_3_month,
        Decimal::from_str("-1.0000").unwrap()
    );
    assert_eq!(
        s428.recurring_6_month,
        Decimal::from_str("1704.3000").unwrap()
    );
    assert_eq!(
        s428.recurring_12_month,
        Decimal::from_str("3229.2000").unwrap()
    );
    assert_eq!(
        s428.recurring_24_month,
        Decimal::from_str("5740.8000").unwrap()
    );
    assert_eq!(
        s428.recurring_36_month,
        Decimal::from_str("-1.0000").unwrap()
    );

    let group_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM reliablesite_addon_groups")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(group_count, 13, "Expected all 13 real addon groups");

    let addon_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM reliablesite_addons")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(addon_count, 51, "Expected all 51 real addons");

    // Verify per-product group link counts (428 omits Memory 661; 444 omits GPU 653 & Memory 661)
    let g428: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM reliablesite_product_addon_groups WHERE product_id = 428",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    let g444: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM reliablesite_product_addon_groups WHERE product_id = 444",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    let g426: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM reliablesite_product_addon_groups WHERE product_id = 426",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(g428, 12);
    assert_eq!(g444, 11);
    assert_eq!(g426, 13);

    // Verify real setup fee on One Time Migration (2232)
    let setup_migration: Decimal = sqlx::query_scalar(
        "SELECT setup_1_month FROM reliablesite_addons WHERE addon_id = 2232",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(setup_migration, Decimal::from_str("29.0000").unwrap());

    // Verify corresponding VaultScope catalog products exist
    let rs_products: Vec<Product> = sqlx::query_as(
        "SELECT * FROM products WHERE provider = 'reliablesite' ORDER BY target ASC",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(rs_products.len(), 6);
    assert!(rs_products.iter().all(|p| p.category == "dedicated"));
}

#[tokio::test]
async fn test_storefront_reliablesite_options_quote_and_validation() {
    let state = common::test_app_state().await;
    let cust_id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO customers (id, external_id, username, name, email, status, email_verified) VALUES ($1, $2, $3, 'RS Test Buyer', $4, 'active', true)",
    )
    .bind(cust_id)
    .bind(format!("cust_{}", cust_id.simple()))
    .bind(format!("rsbuyer_{}", &cust_id.simple().to_string()[20..]))
    .bind(format!("rsbuyer_{}@test.local", &cust_id.simple().to_string()[20..]))
    .execute(&state.db)
    .await
    .unwrap();

    let jwt_token = make_token(
        cust_id,
        "rsbuyer@test.local",
        "customer",
        Audience::Storefront,
        &state.config.jwt_secret,
    );
    let app = vaultscope_api::routes::build_router(state.clone());
    let (cookie, csrf) = get_csrf_cookie_and_token(&app).await;

    let prod_428_id = "10000000-0000-4000-8000-000000000428";

    // 1. Fetch options for 428 via white-label storefront endpoint
    let opts_resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/api/storefront/dedicated/products/{prod_428_id}/options"))
                .header(header::AUTHORIZATION, format!("Bearer {jwt_token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(opts_resp.status(), StatusCode::OK);
    let opts_bytes = to_bytes(opts_resp.into_body(), 2 * 1024 * 1024).await.unwrap();
    let opts_raw = String::from_utf8_lossy(&opts_bytes).to_ascii_lowercase();
    assert!(
        !opts_raw.contains("reliablesite"),
        "Storefront options response leaked upstream vendor name"
    );
    let opts: DedicatedProductOptionsResponse = serde_json::from_slice(&opts_bytes).unwrap();

    assert_eq!(opts.server.data_center, "NL");
    assert_eq!(opts.groups.len(), 12);
    let cycles: Vec<&str> = opts
        .available_cycles
        .iter()
        .map(|c| c.billing_cycle.as_str())
        .collect();
    assert_eq!(cycles, vec!["monthly", "semiannually", "annually", "biennially"]);

    // 1b. Verify Storefront /catalog is white-labeled (masks provider, target, cost, reliablesite_product_id)
    let cat_resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/storefront/catalog")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(cat_resp.status(), StatusCode::OK);
    let cat_bytes = to_bytes(cat_resp.into_body(), 2 * 1024 * 1024).await.unwrap();
    let cat_str = String::from_utf8_lossy(&cat_bytes).to_ascii_lowercase();
    assert!(
        !cat_str.contains("reliablesite"),
        "Storefront /catalog leaked upstream vendor name"
    );
    let cat_products: Vec<Product> = serde_json::from_slice(&cat_bytes).unwrap();
    let cat_428 = cat_products
        .iter()
        .find(|p| p.id.to_string() == prod_428_id)
        .expect("Dedicated NL server 428 present in storefront catalog");
    assert_eq!(cat_428.provider, "vaultscope_dedicated");
    assert_eq!(cat_428.cost, Decimal::ZERO);
    assert_eq!(cat_428.specs["configurable_options"], json!(true));
    assert!(cat_428.specs.get("reliablesite_product_id").is_none());

    // 2. Quote with paid bandwidth (2392 = +99.00/mo), Windows 2022 (2345 = +40.00/mo), One Time Migration (2232 = +29.00 setup)
    let mut selected = opts.default_config_options.clone();
    selected.insert("659".to_string(), 2392);
    selected.insert("655".to_string(), 2345);
    selected.insert("643".to_string(), 2232);

    let quote_resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/storefront/dedicated/products/{prod_428_id}/quote"))
                .header(header::AUTHORIZATION, format!("Bearer {jwt_token}"))
                .header(header::COOKIE, &cookie)
                .header("x-csrf-token", &csrf)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    json!({
                        "billing_cycle": "monthly",
                        "config_options": selected
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(quote_resp.status(), StatusCode::OK);
    let quote_bytes = to_bytes(quote_resp.into_body(), 1024 * 1024).await.unwrap();
    assert!(
        !String::from_utf8_lossy(&quote_bytes)
            .to_ascii_lowercase()
            .contains("reliablesite"),
        "Storefront /quote leaked upstream vendor name"
    );
    let quote: DedicatedQuote = serde_json::from_slice(&quote_bytes).unwrap();
    assert_eq!(quote.recurring_net, Decimal::from_str("438.00").unwrap());
    assert_eq!(quote.setup_fee_net, Decimal::from_str("29.00").unwrap());
    assert_eq!(quote.subtotal_net, Decimal::from_str("467.00").unwrap());
    assert_eq!(quote.vat_amount, Decimal::from_str("88.73").unwrap());
    assert_eq!(quote.total_gross, Decimal::from_str("555.73").unwrap());

    // 2b. Verify 6-month (semiannually) pricing uses real 6-month rates (1704.30 + 564.30 + 29.00 setup)
    let mut semi_map = opts.default_config_options.clone();
    semi_map.insert("659".to_string(), 2392);
    semi_map.insert("643".to_string(), 2232);
    let semi_resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/storefront/dedicated/products/{prod_428_id}/quote"))
                .header(header::AUTHORIZATION, format!("Bearer {jwt_token}"))
                .header(header::COOKIE, &cookie)
                .header("x-csrf-token", &csrf)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    json!({
                        "billing_cycle": "semiannually",
                        "config_options": semi_map
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(semi_resp.status(), StatusCode::OK);
    let semi_quote: DedicatedQuote =
        serde_json::from_slice(&to_bytes(semi_resp.into_body(), 1024 * 1024).await.unwrap())
            .unwrap();
    assert_eq!(semi_quote.period_days, 180);
    assert_eq!(semi_quote.recurring_net, Decimal::from_str("2268.60").unwrap());
    assert_eq!(semi_quote.setup_fee_net, Decimal::from_str("29.00").unwrap());

    // 3. Reject unavailable billing cycle ('quarterly' = -1 on NL)
    let bad_cycle = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/storefront/dedicated/products/{prod_428_id}/quote"))
                .header(header::AUTHORIZATION, format!("Bearer {jwt_token}"))
                .header(header::COOKIE, &cookie)
                .header("x-csrf-token", &csrf)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    json!({ "billing_cycle": "quarterly", "config_options": selected }).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(bad_cycle.status(), StatusCode::BAD_REQUEST);

    // 4. Reject invalid Group_Id (661 Memory is not offered on product 428)
    let mut bad_group_map = opts.default_config_options.clone();
    bad_group_map.insert("661".to_string(), 2421);
    let bad_group = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/storefront/dedicated/products/{prod_428_id}/quote"))
                .header(header::AUTHORIZATION, format!("Bearer {jwt_token}"))
                .header(header::COOKIE, &cookie)
                .header("x-csrf-token", &csrf)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    json!({ "billing_cycle": "monthly", "config_options": bad_group_map })
                        .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(bad_group.status(), StatusCode::BAD_REQUEST);

    // 5. Reject Addon_Id belonging to wrong group
    let mut wrong_addon_map = opts.default_config_options.clone();
    wrong_addon_map.insert("659".to_string(), 2345);
    let wrong_addon = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/storefront/dedicated/products/{prod_428_id}/quote"))
                .header(header::AUTHORIZATION, format!("Bearer {jwt_token}"))
                .header(header::COOKIE, &cookie)
                .header("x-csrf-token", &csrf)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    json!({ "billing_cycle": "monthly", "config_options": wrong_addon_map })
                        .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(wrong_addon.status(), StatusCode::BAD_REQUEST);

    // 6. Reject partial map missing required group
    let mut partial_map = BTreeMap::new();
    partial_map.insert("659".to_string(), 2391u32);
    let missing_group = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/storefront/dedicated/products/{prod_428_id}/quote"))
                .header(header::AUTHORIZATION, format!("Bearer {jwt_token}"))
                .header(header::COOKIE, &cookie)
                .header("x-csrf-token", &csrf)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    json!({ "billing_cycle": "monthly", "config_options": partial_map }).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(missing_group.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn test_orders_out_of_stock_and_sftp_regression_and_admin_sync() {
    let mut state = common::test_app_state().await;

    let cust_id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO customers (id, external_id, username, name, email, status, email_verified) VALUES ($1, $2, $3, 'SFTP & RS Customer', $4, 'active', true)",
    )
    .bind(cust_id)
    .bind(format!("cust_reg_{}", cust_id.simple()))
    .bind(format!("reg_{}", &cust_id.simple().to_string()[20..]))
    .bind(format!("reg_{}@test.local", &cust_id.simple().to_string()[20..]))
    .execute(&state.db)
    .await
    .unwrap();

    // Ensure staff user exists for admin sync test
    let staff_id = Uuid::now_v7();
    let role_id: Uuid = sqlx::query_scalar("SELECT id FROM roles LIMIT 1")
        .fetch_one(&state.db)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO staff (id, external_id, username, first_name, last_name, email, email_verified, role_id) VALUES ($1, $2, $3, 'Admin', 'Tester', $4, true, $5)",
    )
    .bind(staff_id)
    .bind(format!("ext_{}", staff_id.simple()))
    .bind(format!("adm_{}", &staff_id.simple().to_string()[20..]))
    .bind(format!("adm_{}@test.local", &staff_id.simple().to_string()[20..]))
    .bind(role_id)
    .execute(&state.db)
    .await
    .unwrap();

    // Spawn mock SOAP server with isolated test product_id 999428 so NL tests never conflict concurrently
    let mock_soap_app = Router::new().route(
        "/inventory",
        post(|body: String| async move {
            if body.contains("<ServersList") {
                r#"<s:Envelope xmlns:s="http://schemas.xmlsoap.org/soap/envelope/"><s:Body><ServersListResponse xmlns="http://tempuri.org/"><ServersListResult xmlns:a="http://schemas.datacontract.org/2004/07/InventoryServiceLibrary"><a:Result>true</a:Result><a:ServerDetailsList><a:Server_Details><a:Data_Center>TESTNL</a:Data_Center><a:Description>Sync Test NL Dedicated Server</a:Description><a:Detail>AMD Epyc Test&lt;br /&gt;128 GB DDR5 RAM&lt;br /&gt;2 TB NVMe&lt;br /&gt;Amsterdam, NL Data Center</a:Detail><a:Product_Id>999428</a:Product_Id><a:Recurring_1_Hour>0</a:Recurring_1_Hour><a:Recurring_1_Month>299.0000</a:Recurring_1_Month><a:Recurring_3_Month>-1.0000</a:Recurring_3_Month><a:Recurring_6_Month>1704.3000</a:Recurring_6_Month><a:Recurring_12_Month>3229.2000</a:Recurring_12_Month><a:Recurring_24_Month>5740.8000</a:Recurring_24_Month><a:Recurring_36_Month>-1.0000</a:Recurring_36_Month><a:Setup_1_Hour>0</a:Setup_1_Hour><a:Setup_1_Month>0.0000</a:Setup_1_Month><a:Setup_3_Month>0.0000</a:Setup_3_Month><a:Setup_6_Month>0.0000</a:Setup_6_Month><a:Setup_12_Month>0.0000</a:Setup_12_Month><a:Setup_24_Month>0.0000</a:Setup_24_Month><a:Setup_36_Month>0.0000</a:Setup_36_Month><a:Stock>4</a:Stock></a:Server_Details></a:ServerDetailsList></ServersListResult></ServersListResponse></s:Body></s:Envelope>"#.to_string()
            } else if body.contains("<AddonGroupsList") {
                r#"<s:Envelope xmlns:s="http://schemas.xmlsoap.org/soap/envelope/"><s:Body><AddonGroupsListResponse xmlns="http://tempuri.org/"><AddonGroupsListResult xmlns:a="http://schemas.datacontract.org/2004/07/InventoryServiceLibrary"><a:Result>true</a:Result><a:AddonGroupsDetailsList><a:AddonGroups_Details><a:Description>Port Speed</a:Description><a:Group_Id>565</a:Group_Id><a:Sort_Order>1</a:Sort_Order></a:AddonGroups_Details></a:AddonGroupsDetailsList></AddonGroupsListResult></AddonGroupsListResponse></s:Body></s:Envelope>"#.to_string()
            } else {
                r#"<s:Envelope xmlns:s="http://schemas.xmlsoap.org/soap/envelope/"><s:Body><AddonsListResponse xmlns="http://tempuri.org/"><AddonsListResult xmlns:a="http://schemas.datacontract.org/2004/07/InventoryServiceLibrary"><a:Result>true</a:Result><a:AddonsDetailsList><a:Addons_Details><a:AddonDescription>Dedicated/Unshared Port</a:AddonDescription><a:Addon_Id>1751</a:Addon_Id><a:Group_Id>565</a:Group_Id><a:Recurring_1_Hour>0</a:Recurring_1_Hour><a:Recurring_1_Month>0.0000</a:Recurring_1_Month><a:Recurring_3_Month>0.0000</a:Recurring_3_Month><a:Recurring_6_Month>0.0000</a:Recurring_6_Month><a:Recurring_12_Month>0.0000</a:Recurring_12_Month><a:Recurring_24_Month>0.0000</a:Recurring_24_Month><a:Recurring_36_Month>0.0000</a:Recurring_36_Month><a:Setup_1_Hour>0</a:Setup_1_Hour><a:Setup_1_Month>0.0000</a:Setup_1_Month><a:Setup_3_Month>0.0000</a:Setup_3_Month><a:Setup_6_Month>0.0000</a:Setup_6_Month><a:Setup_12_Month>0.0000</a:Setup_12_Month><a:Setup_24_Month>0.0000</a:Setup_24_Month><a:Setup_36_Month>0.0000</a:Setup_36_Month><a:Sort_Order>1</a:Sort_Order></a:Addons_Details></a:AddonsDetailsList></AddonsListResult></AddonsListResponse></s:Body></s:Envelope>"#.to_string()
            }
        }),
    );
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let mock_addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, mock_soap_app).await.unwrap();
    });

    let mut cfg = (*state.config).clone();
    cfg.reliablesite_inventory_url = Some(format!("http://{mock_addr}/inventory"));
    state.config = Arc::new(cfg);

    let cust_jwt = make_token(
        cust_id,
        "reg@test.local",
        "customer",
        Audience::Storefront,
        &state.config.jwt_secret,
    );
    let admin_jwt = make_token(
        staff_id,
        "adm@test.local",
        "admin",
        Audience::Admin,
        &state.config.jwt_secret,
    );

    let app = vaultscope_api::routes::build_router(state.clone());
    let (cookie, csrf) = get_csrf_cookie_and_token(&app).await;

    // 1. Out-of-stock ReliableSite server (444, stock=0) blocked at payment-intent & order
    let prod_444_id = "10000000-0000-4000-8000-000000000444";
    let pi_oos = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/storefront/orders/create-payment-intent")
                .header(header::AUTHORIZATION, format!("Bearer {cust_jwt}"))
                .header(header::COOKIE, &cookie)
                .header("x-csrf-token", &csrf)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(json!({ "product_id": prod_444_id }).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(pi_oos.status(), StatusCode::BAD_REQUEST);

    // 2. Paid ReliableSite server (428) without payment_intent_id fails closed
    let prod_428_id = "10000000-0000-4000-8000-000000000428";
    let rs_unpaid = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/storefront/orders")
                .header(header::AUTHORIZATION, format!("Bearer {cust_jwt}"))
                .header(header::COOKIE, &cookie)
                .header("x-csrf-token", &csrf)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    json!({
                        "product_id": prod_428_id,
                        "hostname": "ams01.vaultscope.de",
                        "billing_cycle": "monthly"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(rs_unpaid.status(), StatusCode::BAD_REQUEST);

    // 3. SFTP free order regression check (same /api/storefront/orders endpoint!)
    let sftp_prod_id = "d9d9d9d9-d9d9-d9d9-d9d9-d9d9d9d9d9d9";
    let sftp_order = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/storefront/orders")
                .header(header::AUTHORIZATION, format!("Bearer {cust_jwt}"))
                .header(header::COOKIE, &cookie)
                .header("x-csrf-token", &csrf)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(json!({ "product_id": sftp_prod_id }).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(sftp_order.status(), StatusCode::OK);
    let sftp_body: Value =
        serde_json::from_slice(&to_bytes(sftp_order.into_body(), 1024 * 1024).await.unwrap())
            .unwrap();
    assert_eq!(sftp_body["status"], "pending");

    // 4. Admin ReliableSite inventory sync against mock SOAP endpoint (using isolated TESTNL DC)
    let sync_resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/admin/reliablesite/sync")
                .header(header::AUTHORIZATION, format!("Bearer {admin_jwt}"))
                .header(header::COOKIE, &cookie)
                .header("x-csrf-token", &csrf)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(json!({ "data_center": "TESTNL" }).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(sync_resp.status(), StatusCode::OK);
    let sync_json: Value =
        serde_json::from_slice(&to_bytes(sync_resp.into_body(), 1024 * 1024).await.unwrap())
            .unwrap();
    assert_eq!(sync_json["data_center"], "TESTNL");
    assert_eq!(sync_json["servers_synced"], 1);
    assert_eq!(sync_json["addons_synced"], 1);

    // 4b. VPS (hetzner_cloud) regression check on /api/storefront/orders
    let vps_prod_id = Uuid::now_v7();
    sqlx::query(
        r#"INSERT INTO products (id, name, category, provider, target, specs, cost, price, setup_fee, stock, user_limit, billing_cycle, hidden)
        VALUES ($1, 'CX22 Regression Test', 'vps', 'hetzner_cloud', 'vps', '{"server_type":"cx22","location":"fsn1","image":"ubuntu-24.04"}'::jsonb, 0.00, 0.00, 0.00, -1, 0, 'monthly', false)"#,
    )
    .bind(vps_prod_id)
    .execute(&state.db)
    .await
    .unwrap();

    let vps_order = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/storefront/orders")
                .header(header::AUTHORIZATION, format!("Bearer {cust_jwt}"))
                .header(header::COOKIE, &cookie)
                .header("x-csrf-token", &csrf)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    json!({ "product_id": vps_prod_id, "hostname": "vps-reg.vaultscope.de" })
                        .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(vps_order.status(), StatusCode::OK);
    let vps_body: Value =
        serde_json::from_slice(&to_bytes(vps_order.into_body(), 1024 * 1024).await.unwrap())
            .unwrap();
    let vps_job_id: Uuid = Uuid::from_str(vps_body["job_id"].as_str().unwrap()).unwrap();
    let vps_job_row: (String, Value) = sqlx::query_as(
        "SELECT target_api, request_payload FROM jobs WHERE id = $1",
    )
    .bind(vps_job_id)
    .fetch_one(&state.db)
    .await
    .unwrap();
    assert_eq!(vps_job_row.0, "hetzner_cloud");
    assert_eq!(vps_job_row.1["server_type"], "cx22");
    assert_eq!(vps_job_row.1["location"], "fsn1");

    // 5. Cleanup isolated synthetic test rows
    let _ = sqlx::query("DELETE FROM reliablesite_servers WHERE product_id = 999428")
        .execute(&state.db)
        .await;
    let _ = sqlx::query("DELETE FROM products WHERE target = '999428' AND provider = 'reliablesite'")
        .execute(&state.db)
        .await;
    let _ = sqlx::query("DELETE FROM jobs WHERE customer_id = $1")
        .bind(cust_id)
        .execute(&state.db)
        .await;
    let _ = sqlx::query("DELETE FROM services WHERE customer_id = $1")
        .bind(cust_id)
        .execute(&state.db)
        .await;
    let _ = sqlx::query("DELETE FROM products WHERE id = $1")
        .bind(vps_prod_id)
        .execute(&state.db)
        .await;
    let _ = sqlx::query("DELETE FROM customers WHERE id = $1")
        .bind(cust_id)
        .execute(&state.db)
        .await;
    let _ = sqlx::query("DELETE FROM staff WHERE id = $1")
        .bind(staff_id)
        .execute(&state.db)
        .await;
}

struct SingleReliableSiteRegistry(Arc<dyn ProvisioningProvider>);

#[async_trait::async_trait]
impl ConnectorRegistry for SingleReliableSiteRegistry {
    async fn get(&self, provider: &str) -> Option<Arc<dyn ProvisioningProvider>> {
        if provider == "reliablesite" {
            Some(self.0.clone())
        } else {
            None
        }
    }
}

#[tokio::test]
async fn test_job_execute_reliablesite_order_with_mock_api() {
    let captured_order: Arc<Mutex<Option<(String, Value)>>> = Arc::new(Mutex::new(None));
    let captured_ref = captured_order.clone();

    let mock_order_app = Router::new()
        .route(
            "/v1/Login/Token",
            post(|Query(q): Query<HashMap<String, String>>| async move {
                assert_eq!(q.get("apiKey").map(String::as_str), Some("test_rs_key_123"));
                Json(json!({ "status": true, "message": "tok_e2e_123" }))
            }),
        )
        .route(
            "/v1/Products/{product_id}/Stock",
            get(|headers: HeaderMap| async move {
                assert_eq!(
                    headers.get(header::AUTHORIZATION).and_then(|v| v.to_str().ok()),
                    Some("Bearer tok_e2e_123")
                );
                Json(json!({
                    "productId": 428,
                    "stock": 7,
                    "stockControlled": true
                }))
            }),
        )
        .route(
            "/v1/PaymentMethods",
            get(|| async {
                Json(json!({
                    "paymentMethods": [
                        { "module": "banktransfer", "displayName": "Bank Transfer" },
                        { "module": "paypalbilling", "displayName": "PayPal" }
                    ]
                }))
            }),
        )
        .route(
            "/v1/Orders",
            post(move |headers: HeaderMap, Json(body): Json<Value>| {
                let captured = captured_ref.clone();
                async move {
                    let idem = headers
                        .get("Idempotency-Key")
                        .and_then(|v| v.to_str().ok())
                        .unwrap_or_default()
                        .to_string();
                    assert_eq!(
                        headers.get(header::AUTHORIZATION).and_then(|v| v.to_str().ok()),
                        Some("Bearer tok_e2e_123")
                    );
                    *captured.lock().unwrap() = Some((idem, body));
                    Json(json!({
                        "orderId": 88156,
                        "invoiceId": 588048,
                        "productIds": "91022",
                        "paymentMethod": "banktransfer",
                        "productId": 428,
                        "replayed": false
                    }))
                }
            }),
        );

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, mock_order_app).await.unwrap();
    });

    let base_url = format!("http://{addr}");
    let connector: Arc<dyn ProvisioningProvider> = Arc::new(ReliableSite::with_endpoints(
        "test_rs_key_123".to_string(),
        base_url.clone(),
        format!("{base_url}/inventory"),
        None,
    ));
    let registry: Arc<dyn ConnectorRegistry> = Arc::new(SingleReliableSiteRegistry(connector));

    let job_id = Uuid::now_v7();
    let job = Job {
        id: job_id,
        task: "provision".to_string(),
        target_api: "reliablesite".to_string(),
        customer_id: Some(Uuid::now_v7()),
        service_id: Some(Uuid::now_v7()),
        status: JobStatus::Queued,
        error: None,
        request_payload: Some(json!({
            "server_type": "428",
            "location": "NL",
            "image": "ubuntu-24.04",
            "ssh_keys": [],
            "name": "ams-e2e.vaultscope.de",
            "extra": {
                "reliablesite_product_id": 428,
                "billing_cycle": "semiannually",
                "config_options": {
                    "622": 2325,
                    "565": 1751
                },
                "promo_code": "RESELLERNL",
                "order_notes": "Rack in AMS01"
            }
        })),
        response_payload: None,
        attempts: 1,
        max_attempts: 3,
        scheduled_at: chrono::Utc::now(),
        started_at: None,
        completed_at: None,
        created_at: chrono::Utc::now(),
    };

    let result = vaultscope_jobs::tasks::execute(&job, &registry)
        .await
        .expect("ReliableSite provision job execution should succeed");

    assert_eq!(result["resource_id"], "ds-ord-88156");
    assert_eq!(result["hostname"], "ams-e2e.vaultscope.de");
    assert_eq!(result["raw_response"]["orderId"], 88156);
    assert_eq!(result["raw_response"]["invoiceId"], 588048);
    assert_eq!(result["raw_response"]["productIds"], "91022");
    assert_eq!(result["raw_response"]["billingCycle"], "semiannually");

    let (sent_idem, sent_body) = captured_order
        .lock()
        .unwrap()
        .clone()
        .expect("POST /v1/Orders must be called");
    assert_eq!(sent_idem, job_id.to_string());
    assert_eq!(sent_body["productId"], 428);
    assert_eq!(sent_body["billingCycle"], "semiannually");
    assert_eq!(sent_body["hostname"], "ams-e2e.vaultscope.de");
    assert_eq!(sent_body["paymentMethod"], "banktransfer");
    assert_eq!(sent_body["configOptions"]["622"], 2325);
    assert_eq!(sent_body["configOptions"]["565"], 1751);
    assert_eq!(sent_body["promoCode"], "RESELLERNL");
    assert_eq!(sent_body["orderNotes"], "Rack in AMS01");
}
