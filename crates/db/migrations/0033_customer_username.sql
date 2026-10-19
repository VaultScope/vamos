-- 0033_customer_username.sql
-- Adds username column to customers for login and community forum posts

ALTER TABLE customers
ADD COLUMN IF NOT EXISTS username TEXT NOT NULL DEFAULT '';

CREATE UNIQUE INDEX IF NOT EXISTS idx_customers_username_unique
ON customers (LOWER(username))
WHERE username <> '';
