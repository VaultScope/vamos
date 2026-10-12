-- Migration 0023: Add customer_billing_emails table for business multi-email invoice CC
CREATE TABLE IF NOT EXISTS customer_billing_emails (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    customer_id UUID NOT NULL REFERENCES customers(id) ON DELETE CASCADE,
    email TEXT NOT NULL,
    verified BOOLEAN NOT NULL DEFAULT false,
    verification_token TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    verified_at TIMESTAMPTZ,
    CONSTRAINT uq_customer_billing_email UNIQUE (customer_id, email)
);

CREATE INDEX IF NOT EXISTS idx_customer_billing_emails_cust ON customer_billing_emails(customer_id);
CREATE INDEX IF NOT EXISTS idx_customer_billing_emails_token ON customer_billing_emails(verification_token);
