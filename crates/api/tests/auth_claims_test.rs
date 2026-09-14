use uuid::Uuid;
use vaultscope_api::auth::{jwt, Audience, Claims};

#[test]
fn test_claims_serializes_to_correct_json_fields() {
    let sub = Uuid::nil();
    let claims = Claims {
        sub,
        email: "test@test.com".to_string(),
        role: "admin".to_string(),
        audience: Audience::Admin,
        iat: 1000,
        exp: 2000,
    };
    let json = serde_json::to_value(&claims).unwrap();
    assert_eq!(json["sub"], sub.to_string());
    assert_eq!(json["email"], "test@test.com");
    assert_eq!(json["role"], "admin");
    assert_eq!(json["audience"], "admin");
    assert_eq!(json["iat"], 1000);
    assert_eq!(json["exp"], 2000);
}

#[test]
fn test_audience_admin_serializes_to_admin() {
    let json = serde_json::to_string(&Audience::Admin).unwrap();
    assert_eq!(json, "\"admin\"");
}

#[test]
fn test_audience_storefront_serializes_to_storefront() {
    let json = serde_json::to_string(&Audience::Storefront).unwrap();
    assert_eq!(json, "\"storefront\"");
}

#[test]
fn test_audience_deserialization() {
    let admin: Audience = serde_json::from_str("\"admin\"").unwrap();
    assert_eq!(admin, Audience::Admin);
    let sf: Audience = serde_json::from_str("\"storefront\"").unwrap();
    assert_eq!(sf, Audience::Storefront);
}

#[test]
fn test_audience_invalid_deserialization_fails() {
    let result = serde_json::from_str::<Audience>("\"unknown\"");
    assert!(result.is_err());
}

#[test]
fn test_claims_deserialization_from_json() {
    let sub = Uuid::nil();
    let json = serde_json::json!({
        "sub": sub.to_string(),
        "email": "user@example.com",
        "role": "customer",
        "audience": "storefront",
        "iat": 1000,
        "exp": 2000
    });
    let claims: Claims = serde_json::from_value(json).unwrap();
    assert_eq!(claims.sub, sub);
    assert_eq!(claims.email, "user@example.com");
    assert_eq!(claims.role, "customer");
    assert_eq!(claims.audience, Audience::Storefront);
    assert_eq!(claims.iat, 1000);
    assert_eq!(claims.exp, 2000);
}

#[test]
fn test_similar_secrets_fail_verification() {
    let claims = Claims {
        sub: Uuid::now_v7(),
        email: "test@test.com".to_string(),
        role: "admin".to_string(),
        audience: Audience::Admin,
        iat: chrono::Utc::now().timestamp(),
        exp: jwt::new_expiry(),
    };
    let token = jwt::sign(&claims, "key1").unwrap();
    let result = jwt::verify(&token, "key1a", Audience::Admin);
    assert!(result.is_err(), "Similar but different secret must fail");
}

#[test]
fn test_secret_prefix_fails_verification() {
    let claims = Claims {
        sub: Uuid::now_v7(),
        email: "test@test.com".to_string(),
        role: "admin".to_string(),
        audience: Audience::Admin,
        iat: chrono::Utc::now().timestamp(),
        exp: jwt::new_expiry(),
    };
    let token = jwt::sign(&claims, "my_secret_key_long").unwrap();
    let result = jwt::verify(&token, "my_secret_key", Audience::Admin);
    assert!(result.is_err(), "Prefix of secret must fail verification");
}
