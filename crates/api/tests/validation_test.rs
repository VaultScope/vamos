use vaultscope_api::validation::*;

#[test]
fn test_validate_required_string() {
    assert!(validate_required_string("hello", "Field").is_ok());
    assert!(validate_required_string("   hello   ", "Field").is_ok());
    assert!(validate_required_string("", "Field").is_err());
    assert!(validate_required_string("   ", "Field").is_err());
}

#[test]
fn test_validate_string_length() {
    assert!(validate_string_length("hello", "Field", 10).is_ok());
    assert!(validate_string_length("hello", "Field", 5).is_ok());
    assert!(validate_string_length("hello world", "Field", 5).is_err());
    // Boundary: exact length is OK
    assert!(validate_string_length("hello", "Field", 5).is_ok());
}

#[test]
fn test_validate_positive() {
    assert!(validate_positive(0.0, "Field").is_ok());
    assert!(validate_positive(10.5, "Field").is_ok());
    assert!(validate_positive(-1.0, "Field").is_err());
    assert!(validate_positive(-0.001, "Field").is_err());
}

#[test]
fn test_validate_greater_than_zero() {
    assert!(validate_greater_than_zero(1, "Field").is_ok());
    assert!(validate_greater_than_zero(100, "Field").is_ok());
    assert!(validate_greater_than_zero(0, "Field").is_err());
    assert!(validate_greater_than_zero(-1, "Field").is_err());
}

#[test]
fn test_validate_percentage() {
    assert!(validate_percentage(0.0, "Field").is_ok());
    assert!(validate_percentage(50.0, "Field").is_ok());
    assert!(validate_percentage(100.0, "Field").is_ok());
    assert!(validate_percentage(-1.0, "Field").is_err());
    assert!(validate_percentage(101.0, "Field").is_err());
    // Boundary values exactly
    assert!(validate_percentage(0.0, "Field").is_ok());
    assert!(validate_percentage(100.0, "Field").is_ok());
}

#[test]
fn test_validate_email() {
    assert!(validate_email("user@example.com").is_ok());
    assert!(validate_email("admin@vaultscope.net").is_ok());
    // Must have @ and .
    assert!(validate_email("notanemail").is_err());
    assert!(validate_email("missing@dot").is_err());
    assert!(validate_email("nodomain.com").is_err());
    // Too long emails should fail
    let long_email = format!("{}@example.com", "a".repeat(320));
    assert!(validate_email(&long_email).is_err());
}
