-- Stage 1 Forward-Only Migration 0034: Lifecycle, Tax & Compliance DDL
-- Fast metadata-only DDL, NOT VALID foreign keys (validated online in 0035),
-- and fail-closed trigger protection for statutory tax invoices.
--
-- INVARIANTS:
--   - Never modify migrations 0001-0033.
--   - Forward-only recovery in production; never execute destructive rollbacks on prod.
--   - Keeps lock_timeout / statement_timeout bounded to avoid production pileups.

SET LOCAL lock_timeout = '3s';
SET LOCAL statement_timeout = '15s';

-- 1. Customers: monotonic auth revocation + deletion/tax-hold lifecycle metadata
ALTER TABLE customers
    ADD COLUMN IF NOT EXISTS lifecycle_state TEXT NOT NULL DEFAULT 'active';

DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint WHERE conname = 'customers_lifecycle_state_check'
    ) THEN
        ALTER TABLE customers
            ADD CONSTRAINT customers_lifecycle_state_check
            CHECK (lifecycle_state IN ('active', 'deletion_pending', 'restricted_tax_hold', 'deleted'));
    END IF;
END
$$;

ALTER TABLE customers
    ADD COLUMN IF NOT EXISTS deletion_requested_at TIMESTAMPTZ;

ALTER TABLE customers
    ADD COLUMN IF NOT EXISTS auth_version BIGINT NOT NULL DEFAULT 1;

DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint WHERE conname = 'customers_auth_version_check'
    ) THEN
        ALTER TABLE customers
            ADD CONSTRAINT customers_auth_version_check
            CHECK (auth_version >= 1);
    END IF;
END
$$;

-- 2. Staff: monotonic auth revocation version
ALTER TABLE staff
    ADD COLUMN IF NOT EXISTS auth_version BIGINT NOT NULL DEFAULT 1;

DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint WHERE conname = 'staff_auth_version_check'
    ) THEN
        ALTER TABLE staff
            ADD CONSTRAINT staff_auth_version_check
            CHECK (auth_version >= 1);
    END IF;
END
$$;

-- 3. Services: lifecycle/grace state + versioned encrypted credential column
ALTER TABLE services
    ADD COLUMN IF NOT EXISTS lifecycle_state TEXT NOT NULL DEFAULT 'active';

DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint WHERE conname = 'services_lifecycle_state_check'
    ) THEN
        ALTER TABLE services
            ADD CONSTRAINT services_lifecycle_state_check
            CHECK (lifecycle_state IN ('provisioning', 'active', 'grace_period', 'deletion_pending', 'deleted'));
    END IF;
END
$$;

ALTER TABLE services
    ADD COLUMN IF NOT EXISTS grace_ends_at TIMESTAMPTZ;

ALTER TABLE services
    ADD COLUMN IF NOT EXISTS encrypted_password TEXT;

-- 4. Invoices: nullable customer_id (for Scenario A unlinking once snapshot is complete),
--    frozen billing_snapshot, tax_regime state, statutory retention metadata, and § 31(5) UStDV link
ALTER TABLE invoices
    ALTER COLUMN customer_id DROP NOT NULL;

ALTER TABLE invoices
    ADD COLUMN IF NOT EXISTS billing_snapshot JSONB;

ALTER TABLE invoices
    ADD COLUMN IF NOT EXISTS tax_regime TEXT NOT NULL DEFAULT 'unverified_pending';

DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint WHERE conname = 'invoices_tax_regime_check'
    ) THEN
        ALTER TABLE invoices
            ADD CONSTRAINT invoices_tax_regime_check
            CHECK (tax_regime IN ('unverified_pending', 'kleinunternehmer_19_ustg', 'standard_vat_19'));
    END IF;
END
$$;

-- NOTE: retention_until intentionally remains NULL for existing historical invoices until
-- external tax-advisor (Steuerberater) confirmation of BEG IV / Art. 97 § 19a EGAO
-- transitional calculation and § 147(3) S. 5 AO Ablaufhemmung.
ALTER TABLE invoices
    ADD COLUMN IF NOT EXISTS retention_until DATE;

ALTER TABLE invoices
    ADD COLUMN IF NOT EXISTS retention_hold BOOLEAN NOT NULL DEFAULT false;

ALTER TABLE invoices
    ADD COLUMN IF NOT EXISTS corrects_invoice_id UUID REFERENCES invoices(id);

ALTER TABLE invoices
    ADD COLUMN IF NOT EXISTS retention_resolution_approved BOOLEAN NOT NULL DEFAULT false;

-- 5. Replace dangerous/restrictive FKs with NOT VALID ON DELETE SET NULL (validated in 0035)
ALTER TABLE invoices DROP CONSTRAINT IF EXISTS invoices_customer_id_fkey;
ALTER TABLE invoices
    ADD CONSTRAINT invoices_customer_id_fkey
    FOREIGN KEY (customer_id) REFERENCES customers(id)
    ON DELETE SET NULL NOT VALID;

ALTER TABLE invoice_line_items DROP CONSTRAINT IF EXISTS invoice_line_items_service_id_fkey;
ALTER TABLE invoice_line_items
    ADD CONSTRAINT invoice_line_items_service_id_fkey
    FOREIGN KEY (service_id) REFERENCES services(id)
    ON DELETE SET NULL NOT VALID;

ALTER TABLE tickets DROP CONSTRAINT IF EXISTS tickets_related_service_id_fkey;
ALTER TABLE tickets
    ADD CONSTRAINT tickets_related_service_id_fkey
    FOREIGN KEY (related_service_id) REFERENCES services(id)
    ON DELETE SET NULL NOT VALID;

ALTER TABLE jobs DROP CONSTRAINT IF EXISTS jobs_service_id_fkey;
ALTER TABLE jobs
    ADD CONSTRAINT jobs_service_id_fkey
    FOREIGN KEY (service_id) REFERENCES services(id)
    ON DELETE SET NULL NOT VALID;

-- 6. New lifecycle, webhook idempotency, § 312k BGB cancellation, consent & recovery tables
CREATE TABLE IF NOT EXISTS deletion_workflows (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    target_type TEXT NOT NULL CHECK (target_type IN ('customer_account', 'service')),
    customer_id UUID REFERENCES customers(id) ON DELETE SET NULL,
    service_id UUID REFERENCES services(id) ON DELETE SET NULL,
    provider_resource_id TEXT,
    state TEXT NOT NULL DEFAULT 'queued' CHECK (state IN (
        'queued',
        'revoking_auth',
        'disabling_sftpgo',
        'deleting_filesystem',
        'verifying_purge',
        'unlinking_db',
        'awaiting_tax_snapshot_review',
        'completed',
        'failed_attention'
    )),
    trigger_reason TEXT NOT NULL CHECK (trigger_reason IN (
        'user_account_deletion',
        'admin_customer_deletion',
        'service_cancellation',
        'grace_expired_payment',
        'grace_expired_subscription',
        'trial_expired_unconverted'
    )),
    attempts INTEGER NOT NULL DEFAULT 0,
    max_attempts INTEGER NOT NULL DEFAULT 5,
    next_retry_at TIMESTAMPTZ,
    last_error_code TEXT CHECK (
        last_error_code IS NULL OR last_error_code IN (
            'SFTPGO_UNREACHABLE',
            'SFTPGO_TIMEOUT',
            'SFTPGO_AUTH_REJECTED',
            'SFTPGO_RATE_LIMITED',
            'SFTPGO_HTTP_5XX',
            'SFTPGO_UNEXPECTED_4XX',
            'ACTIVE_SESSION_DROP_FAILED',
            'JANITOR_UNREACHABLE',
            'JANITOR_AUTH_REJECTED',
            'FILESYSTEM_SYMLINK_VIOLATION',
            'FILESYSTEM_DELETE_FAILED',
            'FILESYSTEM_VERIFY_NONEMPTY',
            'SNAPSHOT_MISSING_OR_INCOMPLETE',
            'DB_TX_CONFLICT'
        )
    ),
    last_http_status SMALLINT,
    retained_voucher_count INTEGER NOT NULL DEFAULT 0,
    sftp_user_removed_at TIMESTAMPTZ,
    storage_purged_at TIMESTAMPTZ,
    db_unlinked_at TIMESTAMPTZ,
    completed_at TIMESTAMPTZ,
    metadata_expires_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_deletion_workflows_state_retry
    ON deletion_workflows(state, next_retry_at);
CREATE INDEX IF NOT EXISTS idx_deletion_workflows_customer
    ON deletion_workflows(customer_id) WHERE customer_id IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx_deletion_workflows_service
    ON deletion_workflows(service_id) WHERE service_id IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx_deletion_workflows_metadata_expiry
    ON deletion_workflows(metadata_expires_at) WHERE metadata_expires_at IS NOT NULL;

CREATE TABLE IF NOT EXISTS stripe_webhook_events (
    event_id TEXT PRIMARY KEY,
    event_type TEXT NOT NULL,
    payment_intent_id TEXT,
    processing_status TEXT NOT NULL DEFAULT 'received'
        CHECK (processing_status IN ('received', 'processed', 'ignored', 'failed')),
    received_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    processed_at TIMESTAMPTZ
);

CREATE INDEX IF NOT EXISTS idx_stripe_webhook_events_pi
    ON stripe_webhook_events(payment_intent_id) WHERE payment_intent_id IS NOT NULL;

CREATE TABLE IF NOT EXISTS cancellation_requests (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    customer_id UUID REFERENCES customers(id) ON DELETE SET NULL,
    service_id UUID REFERENCES services(id) ON DELETE SET NULL,
    contract_reference TEXT NOT NULL DEFAULT '',
    requester_name TEXT NOT NULL DEFAULT '',
    requester_email TEXT NOT NULL DEFAULT '',
    cancellation_type TEXT NOT NULL DEFAULT 'ordinary'
        CHECK (cancellation_type IN ('ordinary', 'extraordinary', 'withdrawal')),
    requested_effective_date DATE,
    reason TEXT,
    status TEXT NOT NULL DEFAULT 'submitted'
        CHECK (status IN ('submitted', 'confirmed', 'completed', 'rejected')),
    confirmation_reference TEXT NOT NULL UNIQUE,
    durable_confirmation_sent_at TIMESTAMPTZ,
    retention_until DATE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_cancellation_requests_customer
    ON cancellation_requests(customer_id) WHERE customer_id IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx_cancellation_requests_status
    ON cancellation_requests(status, created_at DESC);

CREATE TABLE IF NOT EXISTS customer_consents (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    customer_id UUID REFERENCES customers(id) ON DELETE CASCADE,
    service_id UUID REFERENCES services(id) ON DELETE SET NULL,
    invoice_id UUID REFERENCES invoices(id) ON DELETE SET NULL,
    consent_type TEXT NOT NULL,
    document_version TEXT NOT NULL,
    accepted BOOLEAN NOT NULL DEFAULT true,
    ip_address TEXT NOT NULL DEFAULT '',
    user_agent TEXT NOT NULL DEFAULT '',
    consented_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_customer_consents_customer
    ON customer_consents(customer_id, consent_type, consented_at DESC);

CREATE TABLE IF NOT EXISTS customer_mfa_recovery_codes (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    customer_id UUID NOT NULL REFERENCES customers(id) ON DELETE CASCADE,
    code_hash TEXT NOT NULL,
    used_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_customer_mfa_recovery_codes_cust
    ON customer_mfa_recovery_codes(customer_id) WHERE used_at IS NULL;

-- 7. Fail-closed BEFORE DELETE trigger preventing raw deletion of non-draft statutory invoices
CREATE OR REPLACE FUNCTION prevent_statutory_invoice_deletion()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
BEGIN
    IF OLD.retention_hold THEN
        RAISE EXCEPTION 'Deletion forbidden: invoice % (%) is under statutory retention_hold (§ 147 Abs. 3 Satz 5 AO)',
            OLD.id, OLD.invoice_number
            USING ERRCODE = 'restrict_violation';
    END IF;

    IF OLD.status <> 'draft'::invoice_status THEN
        IF OLD.retention_until IS NULL THEN
            RAISE EXCEPTION 'Deletion forbidden: statutory invoice % (%) has unresolved retention_until (fail-closed pending tax-advisor confirmation)',
                OLD.id, OLD.invoice_number
                USING ERRCODE = 'restrict_violation';
        END IF;

        IF CURRENT_DATE <= OLD.retention_until THEN
            RAISE EXCEPTION 'Deletion forbidden: statutory invoice % (%) is retained until % (§ 147 AO / § 14b UStG)',
                OLD.id, OLD.invoice_number, OLD.retention_until
                USING ERRCODE = 'restrict_violation';
        END IF;
    END IF;

    RETURN OLD;
END;
$$;

DROP TRIGGER IF EXISTS trg_prevent_statutory_invoice_deletion ON invoices;
CREATE TRIGGER trg_prevent_statutory_invoice_deletion
    BEFORE DELETE ON invoices
    FOR EACH ROW
    EXECUTE FUNCTION prevent_statutory_invoice_deletion();

-- 8. Preserve runtime role grants if the 'vaultscope' database role exists
DO $$
BEGIN
    IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'vaultscope') THEN
        GRANT ALL PRIVILEGES ON TABLE
            deletion_workflows,
            stripe_webhook_events,
            cancellation_requests,
            customer_consents,
            customer_mfa_recovery_codes
        TO vaultscope;
    END IF;
END
$$;
