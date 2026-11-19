use std::net::IpAddr;

use chrono::Utc;
use uuid::Uuid;
use vaultscope_api::routes::admin::mailboxes::{
    is_forbidden_smtp_ip, validate_smtp_host, validate_smtp_port,
};
use vaultscope_api::validation::validate_email;
use vaultscope_db::models::{
    Customer, CustomerBillingEmail, CustomerStatus, Mailbox, Staff,
};

#[test]
fn test_staff_serialization_redacts_password_hash() {
    let secret_hash = "$2b$12$SuperSecretStaffBcryptHashValue123456";
    let staff = Staff {
        id: Uuid::now_v7(),
        external_id: "staff_ext_1".to_string(),
        username: "secadmin".to_string(),
        first_name: "Sec".to_string(),
        last_name: "Admin".to_string(),
        email: "secadmin@vaultscope.de".to_string(),
        email_verified: true,
        avatar_url: String::new(),
        role_id: Uuid::now_v7(),
        mfa_enabled: true,
        last_login: None,
        last_login_ip: "198.51.100.10".to_string(),
        created_at: Utc::now(),
        updated_at: Utc::now(),
        password_hash: Some(secret_hash.to_string()),
    };

    let val = serde_json::to_value(&staff).expect("Staff should serialize");
    let obj = val.as_object().expect("Staff JSON must be an object");

    assert!(
        !obj.contains_key("password_hash"),
        "Staff JSON must never contain password_hash"
    );
    assert_eq!(obj.get("email").and_then(|v| v.as_str()), Some("secadmin@vaultscope.de"));

    let raw = serde_json::to_string(&staff).expect("Staff should serialize to string");
    assert!(!raw.contains(secret_hash));

    // Deserializing serialized output without password_hash must succeed cleanly
    let roundtrip: Staff =
        serde_json::from_value(val).expect("Staff should deserialize without password_hash");
    assert_eq!(roundtrip.password_hash, None);
}

#[test]
fn test_customer_serialization_redacts_all_secrets() {
    let secret_hash = "$2b$12$SuperSecretCustomerHashValue654321";
    let secret_verif = "verif_token_9f8e7d6c5b4a";
    let secret_totp = "JBSWY3DPEHPK3PXPSECRET";

    let customer = Customer {
        id: Uuid::now_v7(),
        external_id: "cust_ext_1".to_string(),
        name: "Alice Example".to_string(),
        email: "alice@example.com".to_string(),
        phone: "+49123456789".to_string(),
        company: "Example GmbH".to_string(),
        address: "Main St 1".to_string(),
        postal_code: "10115".to_string(),
        city: "Berlin".to_string(),
        country: "Germany".to_string(),
        vat_id: "DE123456789".to_string(),
        status: CustomerStatus::Active,
        two_factor_enabled: true,
        email_verified: false,
        notes: String::new(),
        stripe_customer_id: None,
        created_at: Utc::now(),
        last_login: None,
        updated_at: Utc::now(),
        password_hash: Some(secret_hash.to_string()),
        verification_token: Some(secret_verif.to_string()),
        totp_secret: Some(secret_totp.to_string()),
        vat_verified: false,
        username: "alice".to_string(),
    };

    let val = serde_json::to_value(&customer).expect("Customer should serialize");
    let obj = val.as_object().expect("Customer JSON must be an object");

    for secret_key in ["password_hash", "verification_token", "totp_secret"] {
        assert!(
            !obj.contains_key(secret_key),
            "Customer JSON must never contain {secret_key}"
        );
    }
    assert_eq!(obj.get("email").and_then(|v| v.as_str()), Some("alice@example.com"));

    let raw = serde_json::to_string(&customer).expect("Customer should serialize to string");
    assert!(!raw.contains(secret_hash));
    assert!(!raw.contains(secret_verif));
    assert!(!raw.contains(secret_totp));

    let roundtrip: Customer =
        serde_json::from_value(val).expect("Customer should deserialize without secret fields");
    assert_eq!(roundtrip.password_hash, None);
    assert_eq!(roundtrip.verification_token, None);
    assert_eq!(roundtrip.totp_secret, None);
}

#[test]
fn test_customer_billing_email_serialization_redacts_verification_token() {
    let secret_tok = "billing_verif_tok_secret_12345";
    let billing_email = CustomerBillingEmail {
        id: Uuid::now_v7(),
        customer_id: Uuid::now_v7(),
        email: "billing@example.com".to_string(),
        verified: false,
        verification_token: Some(secret_tok.to_string()),
        created_at: Utc::now(),
        verified_at: None,
    };

    let val =
        serde_json::to_value(&billing_email).expect("CustomerBillingEmail should serialize");
    let obj = val
        .as_object()
        .expect("CustomerBillingEmail JSON must be an object");

    assert!(
        !obj.contains_key("verification_token"),
        "CustomerBillingEmail JSON must never contain verification_token"
    );
    assert_eq!(obj.get("email").and_then(|v| v.as_str()), Some("billing@example.com"));

    let raw = serde_json::to_string(&billing_email).expect("Should serialize to string");
    assert!(!raw.contains(secret_tok));

    let roundtrip: CustomerBillingEmail = serde_json::from_value(val)
        .expect("CustomerBillingEmail should deserialize without verification_token");
    assert_eq!(roundtrip.verification_token, None);
}

#[test]
fn test_mailbox_serialization_redacts_password() {
    let secret_pass = "smtp-super-secret-password-999";
    let mailbox = Mailbox {
        id: Uuid::now_v7(),
        name: "Primary SMTP".to_string(),
        host: "smtp.vaultscope.de".to_string(),
        port: 587,
        user_name: "noreply@vaultscope.de".to_string(),
        password: secret_pass.to_string(),
        is_default: true,
        created_at: Utc::now(),
    };

    let val = serde_json::to_value(&mailbox).expect("Mailbox should serialize");
    let obj = val.as_object().expect("Mailbox JSON must be an object");

    assert!(
        !obj.contains_key("password"),
        "Mailbox JSON must never contain password"
    );
    assert_eq!(obj.get("host").and_then(|v| v.as_str()), Some("smtp.vaultscope.de"));

    let raw = serde_json::to_string(&mailbox).expect("Mailbox should serialize to string");
    assert!(!raw.contains(secret_pass));

    let roundtrip: Mailbox =
        serde_json::from_value(val).expect("Mailbox should deserialize without password");
    assert!(roundtrip.password.is_empty());
}

#[test]
fn test_validate_email_rejects_sql_wildcards_and_malformed_inputs() {
    for bad in [
        "%@%.%",
        "%@example.com",
        "user%@example.com",
        "_@example.com",
        "user_name@example.com",
        "user@foo..com",
        "user@.example.com",
        "user@example.com.",
        " user@example.com",
        "user@example.com ",
        "user\n@example.com",
        "user\\@example.com",
        "a@b@example.com",
        "user@127.0.0.1",
    ] {
        assert!(
            validate_email(bad).is_err(),
            "Expected validate_email({:?}) to reject dangerous input",
            bad
        );
    }

    for good in [
        "user@example.com",
        "user+tag@mail.vaultscope.net",
        "admin1@test.local",
    ] {
        assert!(
            validate_email(good).is_ok(),
            "Expected validate_email({:?}) to succeed",
            good
        );
    }
}

#[test]
fn test_ssrf_ip_validator_rejects_internal_and_accepts_public() {
    let forbidden = [
        // Loopback
        "127.0.0.1",
        "127.0.0.2",
        "127.255.255.254",
        "::1",
        // RFC1918
        "10.0.0.1",
        "10.255.255.254",
        "172.16.0.1",
        "172.31.255.254",
        "192.168.0.1",
        "192.168.1.100",
        // Link-local & AWS/GCP/Azure metadata
        "169.254.169.254",
        "169.254.0.1",
        "fe80::1",
        "febf:ffff::1",
        // Unspecified & 0.0.0.0/8
        "0.0.0.0",
        "0.10.20.30",
        "::",
        // CGNAT 100.64.0.0/10
        "100.64.0.1",
        "100.127.255.254",
        // ULA fc00::/7
        "fc00::1",
        "fd12:3456:789a::1",
        // IPv4-mapped IPv6 bypass attempts
        "::ffff:127.0.0.1",
        "::ffff:10.0.0.1",
        "::ffff:172.16.0.1",
        "::ffff:192.168.1.1",
        "::ffff:169.254.169.254",
        "::ffff:100.64.0.1",
        "::ffff:0.0.0.0",
        // Multicast / broadcast
        "224.0.0.1",
        "255.255.255.255",
        "ff02::1",
    ];

    for ip_str in forbidden {
        let ip: IpAddr = ip_str.parse().unwrap();
        assert!(
            is_forbidden_smtp_ip(ip),
            "Expected IP {ip_str} to be rejected by SSRF IP validator"
        );
    }

    let allowed = [
        "1.1.1.1",
        "8.8.8.8",
        "93.184.216.34",
        "172.15.255.255",
        "172.32.0.1",
        "100.63.255.255",
        "100.128.0.1",
        "192.0.1.1",
        "2606:4700:4700::1111",
        "2001:4860:4860::8888",
    ];

    for ip_str in allowed {
        let ip: IpAddr = ip_str.parse().unwrap();
        assert!(
            !is_forbidden_smtp_ip(ip),
            "Expected public IP {ip_str} to be accepted by SSRF IP validator"
        );
    }
}

#[tokio::test]
async fn test_ssrf_host_validator_and_dev_mailpit_exception() {
    for blocked in [
        "127.0.0.1",
        "::1",
        "[::1]",
        "10.0.0.1",
        "172.16.0.1",
        "192.168.1.1",
        "169.254.169.254",
        "100.64.0.1",
        "0.0.0.0",
        "[fe80::1]",
        "[fc00::1]",
        "[::ffff:169.254.169.254]",
        "localhost",
        "sub.localhost",
        "metadata.google.internal",
        "redis",
        "127.1",
        "0177.0.0.1",
        "2130706433",
        "",
        "smtp.example.com:587",
    ] {
        assert!(
            validate_smtp_host(blocked, 587, false).await.is_err(),
            "Expected host {blocked:?} to be rejected (dev_mode=false)"
        );
        assert!(
            validate_smtp_host(blocked, 587, true).await.is_err(),
            "Expected host {blocked:?} to be rejected (dev_mode=true)"
        );
    }

    // mailpit is only permitted in dev_mode
    assert!(validate_smtp_host("mailpit", 1025, false).await.is_err());
    assert!(validate_smtp_host("mailpit", 1025, true).await.is_ok());

    // Valid public IP hosts are permitted
    assert!(validate_smtp_host("93.184.216.34", 587, false).await.is_ok());
    assert!(validate_smtp_host("1.1.1.1", 587, false).await.is_ok());
    assert!(validate_smtp_host("[2606:4700:4700::1111]", 587, false).await.is_ok());

    // Port validation bounds
    assert!(validate_smtp_port(0).is_err());
    assert!(validate_smtp_port(-1).is_err());
    assert!(validate_smtp_port(65536).is_err());
    assert_eq!(validate_smtp_port(587).unwrap(), 587);
}
