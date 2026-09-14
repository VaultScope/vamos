mod common;

use uuid::Uuid;
use vaultscope_api::auth::{jwt, Audience, Claims};

/// Ensure that a token signed for Admin audience cannot be verified as Storefront.
#[test]
fn test_jwt_audience_separation() {
    let secret = "test_secret_for_audience";

    let admin_claims = Claims {
        sub: Uuid::now_v7(),
        email: "admin@example.com".to_string(),
        role: "admin".to_string(),
        audience: Audience::Admin,
        iat: chrono::Utc::now().timestamp(),
        exp: jwt::new_expiry(),
    };

    let storefront_claims = Claims {
        sub: Uuid::now_v7(),
        email: "customer@example.com".to_string(),
        role: "customer".to_string(),
        audience: Audience::Storefront,
        iat: chrono::Utc::now().timestamp(),
        exp: jwt::new_expiry(),
    };

    let admin_token = jwt::sign(&admin_claims, secret).expect("sign admin");
    let storefront_token = jwt::sign(&storefront_claims, secret).expect("sign storefront");

    // Both should be verifiable with matching audience
    let decoded_admin = jwt::verify(&admin_token, secret, Audience::Admin).expect("verify admin");
    let decoded_storefront = jwt::verify(&storefront_token, secret, Audience::Storefront).expect("verify storefront");

    // Audiences must be preserved correctly
    assert_eq!(decoded_admin.audience, Audience::Admin);
    assert_eq!(decoded_storefront.audience, Audience::Storefront);

    // Cross-audience verification should FAIL (SEC-003 protection)
    assert!(jwt::verify(&admin_token, secret, Audience::Storefront).is_err(), "Admin token should fail storefront audience check");
    assert!(jwt::verify(&storefront_token, secret, Audience::Admin).is_err(), "Storefront token should fail admin audience check");
}

/// Ensure JWTs with future expiry verify but past expiry fail.
#[test]
fn test_jwt_expiry() {
    let secret = "test_secret_expiry";

    // Create a token with already-expired timestamp (exp in the past)
    let expired_claims = Claims {
        sub: Uuid::now_v7(),
        email: "test@example.com".to_string(),
        role: "admin".to_string(),
        audience: Audience::Admin,
        iat: chrono::Utc::now().timestamp() - 7200,
        exp: chrono::Utc::now().timestamp() - 3600, // expired 1 hour ago
    };

    let expired_token = jwt::sign(&expired_claims, secret).expect("sign expired");
    let result = jwt::verify(&expired_token, secret, Audience::Admin);
    assert!(result.is_err(), "Expired token should not verify");

    // Create a token with valid future expiry
    let valid_claims = Claims {
        sub: Uuid::now_v7(),
        email: "test@example.com".to_string(),
        role: "admin".to_string(),
        audience: Audience::Admin,
        iat: chrono::Utc::now().timestamp(),
        exp: jwt::new_expiry(),
    };

    let valid_token = jwt::sign(&valid_claims, secret).expect("sign valid");
    let result = jwt::verify(&valid_token, secret, Audience::Admin);
    assert!(result.is_ok(), "Non-expired token should verify");
}

/// Ensure that a correct round-trip preserves all claim fields.
#[test]
fn test_jwt_claims_roundtrip() {
    let secret = "roundtrip_secret";
    let original_sub = Uuid::now_v7();

    let claims = Claims {
        sub: original_sub,
        email: "roundtrip@example.com".to_string(),
        role: "operator".to_string(),
        audience: Audience::Admin,
        iat: chrono::Utc::now().timestamp(),
        exp: jwt::new_expiry(),
    };

    let token = jwt::sign(&claims, secret).unwrap();
    let decoded = jwt::verify(&token, secret, Audience::Admin).unwrap();

    assert_eq!(decoded.sub, original_sub);
    assert_eq!(decoded.email, "roundtrip@example.com");
    assert_eq!(decoded.role, "operator");
    assert_eq!(decoded.audience, Audience::Admin);
}
