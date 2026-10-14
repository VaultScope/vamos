-- Migration: 0028_blog_tags.sql
-- Add tags array column to blog_posts for categorization and filtering

ALTER TABLE blog_posts
ADD COLUMN IF NOT EXISTS tags TEXT[] NOT NULL DEFAULT '{}';

CREATE INDEX IF NOT EXISTS idx_blog_posts_tags ON blog_posts USING GIN(tags);
