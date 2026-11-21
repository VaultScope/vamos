-- Migration 0042: Explicit admin-managed order/provisioning state on services

ALTER TABLE services
    ADD COLUMN IF NOT EXISTS order_state TEXT NOT NULL DEFAULT 'active',
    ADD COLUMN IF NOT EXISTS order_state_note TEXT,
    ADD COLUMN IF NOT EXISTS order_state_updated_at TIMESTAMPTZ;

DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint WHERE conname = 'services_order_state_check'
    ) THEN
        ALTER TABLE services
            ADD CONSTRAINT services_order_state_check
            CHECK (order_state IN (
                'payment_received',
                'provisioning',
                'hardware_setup',
                'active',
                'action_required',
                'cancelled'
            ));
    END IF;
END
$$;

UPDATE services
SET order_state = 'provisioning',
    order_state_updated_at = COALESCE(order_state_updated_at, updated_at, now())
WHERE status = 'pending' AND order_state = 'active';
