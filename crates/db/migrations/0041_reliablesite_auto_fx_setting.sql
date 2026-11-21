-- Migration 0041: Enable live ECB USD->EUR auto-refresh setting for ReliableSite pricing
INSERT INTO settings (key, value, updated_at) VALUES
    ('reliablesite_auto_fx_enabled', 'true'::jsonb, now())
ON CONFLICT (key) DO NOTHING;
