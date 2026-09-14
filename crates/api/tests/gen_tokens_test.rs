use uuid::Uuid;
use vaultscope_api::auth::{jwt, Audience, Claims};

#[test]
fn print_test_tokens() {
    let secret = "dev-secret-do-not-use-in-production-replace-me-with-64-bytes";
    let now = chrono::Utc::now().timestamp();

    let admin = Claims {
        sub: Uuid::parse_str("11111111-1111-1111-1111-111111111111").unwrap(),
        email: "admin@vaultscope.de".to_string(),
        role: "admin".to_string(),
        audience: Audience::Admin,
        iat: now,
        exp: now + 86400,
    };
    let admin_token = jwt::sign(&admin, secret).unwrap();
    println!("ADMIN_TOKEN={}", admin_token);

    let customer = Claims {
        sub: Uuid::parse_str("22222222-2222-2222-2222-222222222222").unwrap(),
        email: "max@example.com".to_string(),
        role: "customer".to_string(),
        audience: Audience::Storefront,
        iat: now,
        exp: now + 86400,
    };
    let customer_token = jwt::sign(&customer, secret).unwrap();
    println!("CUSTOMER_TOKEN={}", customer_token);
}
