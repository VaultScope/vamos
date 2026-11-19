-- Forward-only migration 0038: Stripe PaymentIntent uniqueness & sequential invoice numbering

SET LOCAL lock_timeout = '3s';
SET LOCAL statement_timeout = '15s';

CREATE SEQUENCE IF NOT EXISTS invoice_number_seq START WITH 1000;

CREATE UNIQUE INDEX IF NOT EXISTS idx_invoices_stripe_payment_intent_id
    ON invoices (stripe_payment_intent_id)
    WHERE stripe_payment_intent_id IS NOT NULL;

CREATE UNIQUE INDEX IF NOT EXISTS idx_services_stripe_payment_intent_id
    ON services (stripe_payment_intent_id)
    WHERE stripe_payment_intent_id IS NOT NULL;

DO $$
BEGIN
    IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'vaultscope') THEN
        GRANT USAGE, SELECT, UPDATE ON SEQUENCE invoice_number_seq TO vaultscope;
    END IF;
END
$$;
