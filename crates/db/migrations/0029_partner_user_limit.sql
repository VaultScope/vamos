-- Migration: 0029_partner_user_limit.sql
-- Add limit_one_per_user column to partners table

ALTER TABLE partners ADD COLUMN IF NOT EXISTS limit_one_per_user BOOLEAN NOT NULL DEFAULT true;
