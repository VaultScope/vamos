-- Migration 0020: Add cancellation support to services
ALTER TABLE services ADD COLUMN IF NOT EXISTS cancel_at_period_end BOOLEAN NOT NULL DEFAULT false;
ALTER TABLE services ADD COLUMN IF NOT EXISTS cancellation_reason TEXT;
