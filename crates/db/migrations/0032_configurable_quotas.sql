-- Migration: 0032_configurable_quotas.sql
-- Add configurable storage quota (GB) for trial and post-trial to products, partners, and services

ALTER TABLE products ADD COLUMN IF NOT EXISTS trial_quota_gb INT DEFAULT NULL;
ALTER TABLE products ADD COLUMN IF NOT EXISTS quota_gb INT DEFAULT NULL;

ALTER TABLE partners ADD COLUMN IF NOT EXISTS trial_quota_gb_override INT DEFAULT NULL;
ALTER TABLE partners ADD COLUMN IF NOT EXISTS quota_gb_override INT DEFAULT NULL;

ALTER TABLE services ADD COLUMN IF NOT EXISTS partner_id UUID REFERENCES partners(id) ON DELETE SET NULL;
ALTER TABLE services ADD COLUMN IF NOT EXISTS quota_gb INT DEFAULT NULL;

-- Default existing SFTP products to 5 GB trial and 150 GB regular
UPDATE products
SET trial_quota_gb = 5, quota_gb = 150
WHERE (category ILIKE '%sftp%' OR category ILIKE '%storage%' OR name ILIKE '%sftp%') 
  AND (trial_quota_gb IS NULL OR quota_gb IS NULL);
