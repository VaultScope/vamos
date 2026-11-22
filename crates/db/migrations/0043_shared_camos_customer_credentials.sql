-- 0043_shared_camos_customer_credentials.sql
-- Ensure enrolled CAMOS (staff) accounts share password credentials, email verification,
-- and 2FA (TOTP secret + mfa_enabled) with their matching VaultScope customer account.

UPDATE staff s
SET password_hash = COALESCE(c.password_hash, s.password_hash),
    email_verified = s.email_verified OR c.email_verified,
    mfa_enabled = s.mfa_enabled OR c.two_factor_enabled,
    totp_secret = CASE
        WHEN c.two_factor_enabled AND c.totp_secret IS NOT NULL AND length(trim(c.totp_secret)) > 0
            THEN c.totp_secret
        ELSE COALESCE(NULLIF(trim(s.totp_secret), ''), NULLIF(trim(c.totp_secret), ''))
    END,
    updated_at = now()
FROM customers c
WHERE LOWER(s.email) = LOWER(c.email)
   OR s.external_id = ('customer_' || c.id::text);
