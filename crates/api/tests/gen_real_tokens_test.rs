use uuid::Uuid;
use vaultscope_api::auth::{jwt, Audience, Claims};

#[test]
fn print_real_tokens() {
    let secret = "dev-secret-do-not-use-in-production-replace-me-with-64-bytes";
    let now = chrono::Utc::now().timestamp();

    let admin = Claims {
        sub: Uuid::parse_str("01a08142-450d-73d3-9b96-b6804100a89d").unwrap(),
        email: "dev-admin@example.invalid".to_string(),
        role: "Owner".to_string(),
        audience: Audience::Admin,
        iat: now,
        exp: now + 86400,
        auth_version: Some(1),
        jti: None,
    };
    let admin_token = jwt::sign(&admin, secret).unwrap();
    println!("ADMIN_TOKEN={}", admin_token);

    let customer = Claims {
        sub: Uuid::parse_str("01a081be-0b06-7483-8ef8-964179d3fc5c").unwrap(),
        email: "testcustomer1@vaultscope.de".to_string(),
        role: "customer".to_string(),
        audience: Audience::Storefront,
        iat: now,
        exp: now + 86400,
        auth_version: Some(1),
        jti: None,
    };
    let customer_token = jwt::sign(&customer, secret).unwrap();
    println!("CUSTOMER_TOKEN={}", customer_token);
}
