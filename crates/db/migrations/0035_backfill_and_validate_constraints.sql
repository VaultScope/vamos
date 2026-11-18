-- Stage 1 Forward-Only Migration 0035: Honest Billing Snapshot Backfill & Online FK Validation
--
-- SAFETY & COMPLIANCE INVARIANTS:
--   1. Existing historical invoices preserve invoice_number, issue_date, due_date, subtotal,
--      tax_rate, tax_amount, total, status, paid_at, stripe_payment_intent_id, and notes verbatim.
--   2. Zero fabrication: missing customer address fields remain empty ('') and are recorded in
--      missing_fields with snapshot_complete = false.
--   3. UNRESOLVED TAX / RETENTION ASSUMPTION:
--      invoices.retention_until is deliberately left NULL in 0035 pending external Steuerberater
--      review of BEG IV (8-year invoice retention), Art. 97 § 19a EGAO transitional rules, and
--      § 147 Abs. 3 Satz 5 AO Ablaufhemmung, and pending Finanzamt § 19 UStG status confirmation.
--      Do NOT invent or finalize a statutory retention cutoff date before tax-advisor sign-off.
--   4. prevent_statutory_invoice_deletion() (installed by 0034) fails closed whenever
--      retention_until IS NULL on non-draft invoices.
--   5. Idempotent: re-running only affects rows where billing_snapshot IS NULL; VALIDATE CONSTRAINT
--      is natively idempotent and acquires only SHARE UPDATE EXCLUSIVE.

SET LOCAL lock_timeout = '5s';
SET LOCAL statement_timeout = '60s';

UPDATE invoices AS i
SET billing_snapshot = CASE
    WHEN src.customer_exists THEN
        jsonb_build_object(
            'name', btrim(COALESCE(src.name, '')),
            'company', btrim(COALESCE(src.company, '')),
            'address_line1', btrim(COALESCE(src.address, '')),
            'postal_code', btrim(COALESCE(src.postal_code, '')),
            'city', btrim(COALESCE(src.city, '')),
            'country', btrim(COALESCE(src.country, '')),
            'vat_id', btrim(COALESCE(src.vat_id, '')),
            'snapshot_complete', (
                btrim(COALESCE(src.name, '')) <> ''
                AND btrim(COALESCE(src.address, '')) <> ''
                AND btrim(COALESCE(src.postal_code, '')) <> ''
                AND btrim(COALESCE(src.city, '')) <> ''
                AND btrim(COALESCE(src.country, '')) <> ''
            ),
            'missing_fields', to_jsonb(ARRAY_REMOVE(ARRAY[
                CASE WHEN btrim(COALESCE(src.name, '')) = '' THEN 'name' END,
                CASE WHEN btrim(COALESCE(src.address, '')) = '' THEN 'address_line1' END,
                CASE WHEN btrim(COALESCE(src.postal_code, '')) = '' THEN 'postal_code' END,
                CASE WHEN btrim(COALESCE(src.city, '')) = '' THEN 'city' END,
                CASE WHEN btrim(COALESCE(src.country, '')) = '' THEN 'country' END
            ], NULL)),
            'snapshot_source', 'backfill_0035',
            'captured_at', now()
        )
    ELSE
        jsonb_build_object(
            'name', '',
            'company', '',
            'address_line1', '',
            'postal_code', '',
            'city', '',
            'country', '',
            'vat_id', '',
            'snapshot_complete', false,
            'missing_fields', jsonb_build_array(
                'customer_record', 'name', 'address_line1', 'postal_code', 'city', 'country'
            ),
            'snapshot_source', 'backfill_0035_missing_customer',
            'captured_at', now()
        )
END
FROM (
    SELECT
        i2.id AS invoice_id,
        (c2.id IS NOT NULL) AS customer_exists,
        c2.name,
        c2.company,
        c2.address,
        c2.postal_code,
        c2.city,
        c2.country,
        c2.vat_id
    FROM invoices AS i2
    LEFT JOIN customers AS c2 ON c2.id = i2.customer_id
    WHERE i2.billing_snapshot IS NULL
) AS src
WHERE i.id = src.invoice_id
  AND i.billing_snapshot IS NULL;

-- Online validation of foreign keys added as NOT VALID in 0034 (SHARE UPDATE EXCLUSIVE)
ALTER TABLE invoices VALIDATE CONSTRAINT invoices_customer_id_fkey;
ALTER TABLE invoice_line_items VALIDATE CONSTRAINT invoice_line_items_service_id_fkey;
ALTER TABLE tickets VALIDATE CONSTRAINT tickets_related_service_id_fkey;
ALTER TABLE jobs VALIDATE CONSTRAINT jobs_service_id_fkey;
