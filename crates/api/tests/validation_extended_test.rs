use vaultscope_api::validation::*;

#[test]
fn test_validate_discount_percentage_valid() {
    assert!(validate_discount("percentage", 50.0).is_ok());
    assert!(validate_discount("percentage", 0.0).is_ok());
    assert!(validate_discount("percentage", 100.0).is_ok());
}

#[test]
fn test_validate_discount_percentage_invalid() {
    assert!(validate_discount("percentage", 101.0).is_err());
    assert!(validate_discount("percentage", -5.0).is_err());
}

#[test]
fn test_validate_discount_fixed_allows_large_values() {
    assert!(validate_discount("fixed", 500.0).is_ok());
    assert!(validate_discount("fixed", 99999.0).is_ok());
}

#[test]
fn test_validate_discount_fixed_rejects_negative() {
    assert!(validate_discount("fixed", -1.0).is_err());
}

#[test]
fn test_validate_email_with_plus_addressing() {
    assert!(validate_email("user+tag@example.com").is_ok());
}

#[test]
fn test_validate_email_with_subdomain() {
    assert!(validate_email("admin@mail.vaultscope.net").is_ok());
}

#[test]
fn test_validate_email_empty_string() {
    assert!(validate_email("").is_err());
}

#[test]
fn test_validate_email_only_at_sign() {
    assert!(validate_email("@").is_err());
}

#[test]
fn test_validate_email_only_dot() {
    assert!(validate_email(".").is_err());
}

#[test]
fn test_validate_string_length_empty_string() {
    assert!(validate_string_length("", "Field", 0).is_ok());
    assert!(validate_string_length("", "Field", 10).is_ok());
}

#[test]
fn test_validate_string_length_at_max_boundary() {
    let s = "x".repeat(MAX_SHORT_STRING_LENGTH);
    assert!(validate_string_length(&s, "Field", MAX_SHORT_STRING_LENGTH).is_ok());
    let s2 = "x".repeat(MAX_SHORT_STRING_LENGTH + 1);
    assert!(validate_string_length(&s2, "Field", MAX_SHORT_STRING_LENGTH).is_err());
}

#[test]
fn test_validate_required_string_tab_and_newline() {
    assert!(validate_required_string("\t\n", "Field").is_err());
    assert!(validate_required_string("\thello\n", "Field").is_ok());
}

#[test]
fn test_validate_positive_zero_is_ok() {
    assert!(validate_positive(0.0, "Field").is_ok());
}

#[test]
fn test_validate_positive_very_small_negative() {
    assert!(validate_positive(-f64::MIN_POSITIVE, "Field").is_err());
}

#[test]
fn test_validate_greater_than_zero_max_int() {
    assert!(validate_greater_than_zero(i32::MAX, "Field").is_ok());
}

#[test]
fn test_validate_percentage_boundaries_exact() {
    assert!(validate_percentage(0.0, "Field").is_ok());
    assert!(validate_percentage(100.0, "Field").is_ok());
    assert!(validate_percentage(100.001, "Field").is_err());
    assert!(validate_percentage(-0.001, "Field").is_err());
}

#[test]
fn test_constants() {
    assert_eq!(MAX_STRING_LENGTH, 5000);
    assert_eq!(MAX_SHORT_STRING_LENGTH, 255);
    assert_eq!(MAX_EMAIL_LENGTH, 320);
    assert_eq!(MAX_CODE_LENGTH, 100);
}
