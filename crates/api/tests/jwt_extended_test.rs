use uuid::Uuid;
use vaultscope_api::auth::{jwt, Audience, Claims};

#[test]
fn test_sign_with_empty_secret() {
    let claims = Claims {
        sub: Uuid::now_v7(),
        email: "test@example.com".to_string(),
        role: "admin".to_string(),
        audience: Audience::Admin,
        iat: chrono::Utc::now().timestamp(),
        exp: jwt::new_expiry(),
    };
    let result = jwt::sign(&claims, "");
    assert!(result.is_ok(), "Empty secret should still produce a token");
}

#[test]
fn test_verify_with_wrong_secret() {
    let claims = Claims {
        sub: Uuid::now_v7(),
        email: "test@example.com".to_string(),
        role: "admin".to_string(),
        audience: Audience::Admin,
        iat: chrono::Utc::now().timestamp(),
        exp: jwt::new_expiry(),
    };
    let token = jwt::sign(&claims, "correct_secret").unwrap();
    let result = jwt::verify(&token, "wrong_secret", Audience::Admin);
    assert!(result.is_err(), "Wrong secret must fail verification");
}

#[test]
fn test_verify_garbage_token() {
    let result = jwt::verify("not.a.jwt", "secret", Audience::Admin);
    assert!(result.is_err());
}

#[test]
fn test_verify_empty_token() {
    let result = jwt::verify("", "secret", Audience::Admin);
    assert!(result.is_err());
}

#[test]
fn test_new_expiry_is_in_future() {
    let now = chrono::Utc::now().timestamp();
    let exp = jwt::new_expiry();
    assert!(exp > now, "Expiry must be in the future");
    // Should be roughly 24 hours from now (within 5-second tolerance)
    let expected = now + 24 * 3600;
    assert!((exp - expected).abs() < 5, "Expiry should be ~24h from now");
}

#[test]
fn test_claims_preserve_unicode_email() {
    let secret = "test_unicode";
    let claims = Claims {
        sub: Uuid::now_v7(),
        email: "user+tag@exämple.com".to_string(),
        role: "customer".to_string(),
        audience: Audience::Storefront,
        iat: chrono::Utc::now().timestamp(),
        exp: jwt::new_expiry(),
    };
    let token = jwt::sign(&claims, secret).unwrap();
    let decoded = jwt::verify(&token, secret, Audience::Storefront).unwrap();
    assert_eq!(decoded.email, "user+tag@exämple.com");
}

#[test]
fn test_audience_serialization_roundtrip() {
    let secret = "serde_test";
    for audience in [Audience::Admin, Audience::Storefront] {
        let claims = Claims {
            sub: Uuid::now_v7(),
            email: "test@test.com".to_string(),
            role: "test".to_string(),
            audience: audience.clone(),
            iat: chrono::Utc::now().timestamp(),
            exp: jwt::new_expiry(),
        };
        let token = jwt::sign(&claims, secret).unwrap();
        let decoded = jwt::verify(&token, secret, audience.clone()).unwrap();
        assert_eq!(decoded.audience, audience);
    }
}

#[test]
fn test_different_tokens_for_same_claims() {
    let secret = "determinism_test";
    let sub = Uuid::now_v7();
    let iat = chrono::Utc::now().timestamp();
    let exp = jwt::new_expiry();

    let claims = Claims {
        sub,
        email: "test@test.com".to_string(),
        role: "admin".to_string(),
        audience: Audience::Admin,
        iat,
        exp,
    };

    let token1 = jwt::sign(&claims, secret).unwrap();
    let token2 = jwt::sign(&claims, secret).unwrap();
    // Same claims + same secret => same token (JWT HS256 is deterministic)
    assert_eq!(token1, token2);
}
