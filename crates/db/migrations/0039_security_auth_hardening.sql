-- Forward-only migration 0039: Auth hardening columns (staff TOTP secret, verification token expirations)

SET LOCAL lock_timeout = '3s';
SET LOCAL statement_timeout = '15s';

ALTER TABLE staff
    ADD COLUMN IF NOT EXISTS totp_secret TEXT;

ALTER TABLE customers
    ADD COLUMN IF NOT EXISTS verification_token_expires_at TIMESTAMPTZ;

ALTER TABLE customer_billing_emails
    ADD COLUMN IF NOT EXISTS verification_token_expires_at TIMESTAMPTZ;

DO $$
BEGIN
    IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'vaultscope') THEN
        GRANT SELECT, INSERT, UPDATE, DELETE ON TABLE
            staff,
            customers,
            customer_billing_emails
        TO vaultscope;
    END IF;
END
$$;
