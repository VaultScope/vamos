-- no-transaction
-- Candidate 0036 (Option B — Non-Blocking Concurrent Unique Index via SQLx 0.8.6 no_tx)
-- STATUS: NOT ACTIVE in crates/db/migrations/. Requires staging-clone benchmark + operator approval.
--
-- IMPORTANT SQLx 0.8.6 RULE:
--   "-- no-transaction" MUST start at byte offset 0 so sqlx-core parses no_tx = true.
--   PostgreSQL runs this single statement outside a transaction block (online Phase 1 & Phase 2).
--   IF NOT EXISTS is intentionally omitted so PostgreSQL never silently skips an invalid
--   (indisvalid = false) or differently defined same-name index.
CREATE UNIQUE INDEX CONCURRENTLY idx_invoices_stripe_payment_intent_id
    ON public.invoices (stripe_payment_intent_id)
    WHERE stripe_payment_intent_id IS NOT NULL;
