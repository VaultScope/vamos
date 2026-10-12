-- Migration 0021: Add postal_code to customers
ALTER TABLE customers ADD COLUMN IF NOT EXISTS postal_code TEXT NOT NULL DEFAULT '';
