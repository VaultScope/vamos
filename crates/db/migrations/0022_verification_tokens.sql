-- Migration 0022: Add verification_token to customers
ALTER TABLE customers ADD COLUMN IF NOT EXISTS verification_token TEXT;
CREATE INDEX IF NOT EXISTS idx_customers_verification_token ON customers(verification_token);
