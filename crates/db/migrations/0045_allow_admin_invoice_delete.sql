-- Migration 0045: Allow authorized CAMOS staff (invoices.delete) to delete invoices via session bypass flag
-- while preserving the fail-closed trigger against accidental raw SQL deletions unless explicitly overridden.

CREATE OR REPLACE FUNCTION prevent_statutory_invoice_deletion()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
BEGIN
    IF current_setting('vaultscope.allow_admin_invoice_delete', true) = 'true' THEN
        RETURN OLD;
    END IF;

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
