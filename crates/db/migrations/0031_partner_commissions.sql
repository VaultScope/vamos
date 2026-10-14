-- Migration: 0031_partner_commissions.sql
-- Add commission cut columns to partners table for internal admin tracking

ALTER TABLE partners ADD COLUMN IF NOT EXISTS commission_type VARCHAR(16) DEFAULT 'none';
ALTER TABLE partners ADD COLUMN IF NOT EXISTS commission_rate NUMERIC(5,2);
ALTER TABLE partners ADD COLUMN IF NOT EXISTS commission_fixed_eur NUMERIC(10,2);
