-- Migration 0040: ReliableSite USD->EUR + 15% retail markup defaults and safe Kleinunternehmer (§ 19 UStG) fallback

INSERT INTO settings (key, value, updated_at) VALUES
    ('taxMode', '"small_business"'::jsonb, now()),
    ('defaultTaxRate', '0'::jsonb, now()),
    ('reliablesite_usd_to_eur_rate', '0.91'::jsonb, now()),
    ('reliablesite_markup_percent', '15'::jsonb, now())
ON CONFLICT (key) DO NOTHING;

-- Convert seeded ReliableSite dedicated servers from 1:1 USD wholesale price to EUR retail (+15% markup at 0.91 USD->EUR)
-- only when price still equals wholesale cost so admin custom prices are preserved.
UPDATE products
SET
    price = ROUND(cost * 0.91 * 1.15, 2),
    setup_fee = ROUND(setup_fee * 0.91 * 1.15, 2),
    updated_at = now()
WHERE provider = 'reliablesite'
  AND price = cost
  AND cost > 0;
