-- Candidate 0036 (Option A — Transactional Unique Index During Scheduled Write-Maintenance Pause)
-- STATUS: NOT ACTIVE in crates/db/migrations/. Requires staging-clone benchmark + operator approval.
--
-- Prerequisites before promoting to crates/db/migrations/:
--   1. Duplicate Stripe PaymentIntent preflight query on public.invoices returns 0 rows.
--   2. Application writes to public.invoices are paused (maintenance window / workers drained).
--   3. Verified pre-Stage-1 pg_dump backup and staging restore verification are complete.
--
-- Locking behavior: Acquires SHARE lock on public.invoices (allows concurrent SELECTs, blocks
-- INSERT/UPDATE/DELETE during index build; atomic rollback if lock_timeout/duplicate fails).
-- IF NOT EXISTS is intentionally omitted so an existing invalid or misdefined index fails fast.

SET LOCAL lock_timeout = '2s';
SET LOCAL statement_timeout = '15s';

CREATE UNIQUE INDEX idx_invoices_stripe_payment_intent_id
    ON public.invoices (stripe_payment_intent_id)
    WHERE stripe_payment_intent_id IS NOT NULL;
