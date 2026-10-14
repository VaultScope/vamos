-- Migration: 0027_partners.sql
-- Create partners table for domain.com/ps/{PARTNER} campaigns and product linking

CREATE TABLE IF NOT EXISTS partners (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    slug TEXT NOT NULL UNIQUE,
    name TEXT NOT NULL,
    headline TEXT,
    description TEXT,
    badge_text TEXT,
    product_id UUID NOT NULL REFERENCES products(id) ON DELETE CASCADE,
    trial_days_override INT,
    active BOOLEAN NOT NULL DEFAULT true,
    clicks INT NOT NULL DEFAULT 0,
    conversions INT NOT NULL DEFAULT 0,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_partners_slug ON partners(slug);
