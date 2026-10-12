-- Migration 0024: Add TOTP secret, Stripe payment intent tracking on services, VAT verification, and cancellation feedback
ALTER TABLE customers ADD COLUMN IF NOT EXISTS totp_secret TEXT;
ALTER TABLE customers ADD COLUMN IF NOT EXISTS vat_verified BOOLEAN NOT NULL DEFAULT false;

ALTER TABLE services ADD COLUMN IF NOT EXISTS stripe_payment_intent_id TEXT;
CREATE UNIQUE INDEX IF NOT EXISTS idx_services_stripe_payment_intent_id ON services(stripe_payment_intent_id) WHERE stripe_payment_intent_id IS NOT NULL;
ALTER TABLE services ADD COLUMN IF NOT EXISTS cancellation_feedback TEXT;
