use crate::error::ApiError;

/// Validation constants
pub const MAX_STRING_LENGTH: usize = 5000;
pub const MAX_SHORT_STRING_LENGTH: usize = 255;
pub const MAX_EMAIL_LENGTH: usize = 320;
pub const MAX_CODE_LENGTH: usize = 100;

/// Validate string length
pub fn validate_string_length(value: &str, field: &str, max: usize) -> Result<(), ApiError> {
    if value.len() > max {
        return Err(ApiError::BadRequest(format!(
            "{} exceeds maximum length of {} characters",
            field, max
        )));
    }
    Ok(())
}

/// Validate required string (non-empty after trim)
pub fn validate_required_string(value: &str, field: &str) -> Result<(), ApiError> {
    if value.trim().is_empty() {
        return Err(ApiError::BadRequest(format!("{} is required", field)));
    }
    Ok(())
}

/// Validate numeric is positive
pub fn validate_positive(value: f64, field: &str) -> Result<(), ApiError> {
    if value < 0.0 {
        return Err(ApiError::BadRequest(format!("{} must be positive", field)));
    }
    Ok(())
}

/// Validate numeric is greater than zero
pub fn validate_greater_than_zero(value: i32, field: &str) -> Result<(), ApiError> {
    if value <= 0 {
        return Err(ApiError::BadRequest(format!("{} must be greater than zero", field)));
    }
    Ok(())
}

/// Validate percentage (0-100)
pub fn validate_percentage(value: f64, field: &str) -> Result<(), ApiError> {
    if value < 0.0 || value > 100.0 {
        return Err(ApiError::BadRequest(format!(
            "{} must be between 0 and 100",
            field
        )));
    }
    Ok(())
}

/// Validate email format (basic check)
pub fn validate_email(email: &str) -> Result<(), ApiError> {
    validate_string_length(email, "Email", MAX_EMAIL_LENGTH)?;

    if !email.contains('@') || !email.contains('.') {
        return Err(ApiError::BadRequest("Invalid email format".into()));
    }

    Ok(())
}

/// Validate discount value based on type
pub fn validate_discount(discount_type: &str, value: f64) -> Result<(), ApiError> {
    validate_positive(value, "Discount value")?;

    if discount_type == "percentage" {
        validate_percentage(value, "Discount percentage")?;
    }

    Ok(())
}
