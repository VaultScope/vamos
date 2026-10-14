-- Migration: 0026_trials.sql
-- Add trial_days to products and trial tracking to services

ALTER TABLE products ADD COLUMN IF NOT EXISTS trial_days INT DEFAULT NULL;
ALTER TABLE services ADD COLUMN IF NOT EXISTS is_trial BOOLEAN NOT NULL DEFAULT false;
ALTER TABLE services ADD COLUMN IF NOT EXISTS trial_ends_at TIMESTAMPTZ DEFAULT NULL;

-- Ensure SFTP free tier has trial_days configured
UPDATE products 
SET trial_days = 30 
WHERE id = 'd9d9d9d9-d9d9-d9d9-d9d9-d9d9d9d9d9d9';
