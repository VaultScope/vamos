# Stage 1 Migration `0036` Index Strategy (`idx_invoices_stripe_payment_intent_id`)

> **Status: NOT ACTIVE (`crates/db/migrations/` contains migrations through `0035` only)**  
> Neither Option A nor Option B has been placed into active [`crates/db/migrations/`](../migrations/) yet.  
> Promotion into active migrations requires:
> 1. Read-only live duplicate preflight (`0` duplicate non-null `stripe_payment_intent_id` rows in `public.invoices`),
> 2. Verified pre-Stage-1 `pg_dump --format=custom` backup & restore test,
> 3. Restored staging-clone (`vaultscope_staging_test`) timing & bloat measurements, and
> 4. Explicit operator sign-off selecting Option A or Option B.

---

## 1. Candidate Comparison & Trade-Offs

| Dimension | Option A — Transactional `CREATE UNIQUE INDEX` | Option B — `CREATE UNIQUE INDEX CONCURRENTLY` (`-- no-transaction`) |
| :--- | :--- | :--- |
| **Candidate File** | [`0036_idx_invoices_stripe_pi_transactional.sql`](option_a_transactional/0036_idx_invoices_stripe_pi_transactional.sql) | [`0036_idx_invoices_stripe_pi_concurrent.sql`](option_b_concurrent/0036_idx_invoices_stripe_pi_concurrent.sql) |
| **Locking Behavior** | Acquires `SHARE` lock on `public.invoices`: permits concurrent `SELECT`s, **blocks `INSERT`/`UPDATE`/`DELETE`** (checkout invoice writes, webhook status updates, billing runner) for the entire build duration. Bounded by `SET LOCAL lock_timeout = '2s'`. | Acquires `SHARE UPDATE EXCLUSIVE` across two online snapshot scans: allows concurrent reads and writes without blocking checkout or webhook traffic. Must wait for concurrent transactions holding older snapshots to finish. |
| **SQLx Execution (`0.8.6`)** | Wrapped inside `BEGIN ... COMMIT` alongside `INSERT INTO _sqlx_migrations`. Atomic: either both index creation and bookkeeping commit together, or both roll back cleanly. | Uses `-- no-transaction` (`sqlx-core`: `sql.starts_with("-- no-transaction")` -> `no_tx = true`). Executes the single DDL statement **before** `INSERT INTO _sqlx_migrations`, with a small timing window if the process disconnects right after DDL succeeds. |
| **Failure Mode** | Duplicate values, timeout, or lock timeout abort atomically inside PostgreSQL with no residual invalid index or `_sqlx_migrations` row. | Duplicate values or interruptions leave a broken **`indisvalid = false`** index entry (while `version = 36` remains absent from `_sqlx_migrations`). Subsequent `sqlx migrate` runs fail with `42P07` (`relation already exists`) until cleaned up with `DROP INDEX CONCURRENTLY`. |
| **Why `IF NOT EXISTS` Is Omitted** | Failing fast ensures an unexpectedly pre-existing index never silently passes with wrong uniqueness or predicate. | Critical: `CREATE UNIQUE INDEX CONCURRENTLY IF NOT EXISTS` silently succeeds even if the existing same-name index has `indisvalid = false` or is non-unique! Omitting `IF NOT EXISTS` forces fail-closed catalog inspection. |
| **Operational Window** | **Requires scheduled write-maintenance pause** on `vaultscope-api` & billing workers so no blocked checkout transactions stack behind `SHARE` or trigger `lock_timeout`. | No write downtime required; compatible with zero-downtime rolling API deployments once schema `0034`–`0035` compatibility is live. Recommended default if active checkout/webhook traffic must continue uninterrupted. |

---

## 2. Verified `sqlx-postgres v0.8.6` `no_tx` Behavior

Verified against [`VaultScope-API/Cargo.lock`](../../../Cargo.lock) (`sqlx-core = 0.8.6`, `sqlx-postgres = 0.8.6`):

1. `sqlx-core` sets `no_tx = sql.starts_with("-- no-transaction")` (must begin at byte offset `0`, no leading BOM or whitespace).
2. `PgConnection::apply` acquires a DB-wide `pg_advisory_lock(lock_id)`.
3. When `no_tx == true`, `execute_migration(self, migration)` calls `conn.execute(&*migration.sql)` outside any transaction **first**.
4. Only after `conn.execute` returns `Ok` does SQLx execute:
   ```sql
   INSERT INTO _sqlx_migrations (version, description, success, checksum, execution_time)
   VALUES ($1, $2, TRUE, $3, -1)
   ```
   followed by `UPDATE _sqlx_migrations SET execution_time = ... WHERE version = $2`.
5. **Crucial consequence:** A failed PostgreSQL `-- no-transaction` migration in SQLx `0.8.6` does **not** record a `success = FALSE` row in `_sqlx_migrations`—`version = 36` is simply **absent**. Conversely, if the runner dies or disconnects between step 3 and step 4, PostgreSQL holds a **valid, working unique index** while `version = 36` is still absent from `_sqlx_migrations`.
6. **Advisory lock caveat on failed runs (`Migrator::run_direct`):** In `sqlx-core 0.8.6`, `Migrator::run_direct` calls `conn.lock().await?` at entry and only calls `conn.unlock().await?` after the migration loop completes. On `Err(...)`, the function returns early without releasing the session-level `pg_advisory_lock`. A separate CLI invocation or pod restart cleanly closes the failed TCP session, whereas an in-process retry loop reusing a multi-connection `PgPool` would self-deadlock on `pg_advisory_lock` unless the failed connection is discarded or `pg_advisory_unlock_all()` is executed.

*(Line-ending operational warning: SQLx calculates `checksum` as raw `SHA-384` over the migration file bytes. Always compute expected `_sqlx_migrations` checksums from the exact committed/container-packaged LF bytes via `sha384sum` or SQLx's resolved `Migration.checksum`—never mix CRLF vs. LF checksums.)*

---

## 3. Mandatory Pre-Flight & Catalog Diagnostic Queries

Run before deploying Option A or Option B and before any recovery action:

```sql
-- 1. Duplicate PaymentIntent preflight (MUST return 0 rows)
SELECT stripe_payment_intent_id,
       count(*) AS dup_count,
       array_agg(id ORDER BY created_at) AS invoice_ids,
       array_agg(status::text ORDER BY created_at) AS statuses
FROM public.invoices
WHERE stripe_payment_intent_id IS NOT NULL
GROUP BY stripe_payment_intent_id
HAVING count(*) > 1;

-- 2. Catalog inspection: validity, readiness, uniqueness, columns & canonical definition
SELECT c.relname AS index_name,
       t.relname AS table_name,
       i.indisvalid,
       i.indisready,
       i.indisunique,
       array_agg(a.attname ORDER BY k.ord) AS indexed_columns,
       pg_get_indexdef(i.indexrelid) AS index_def
FROM pg_index i
JOIN pg_class c ON c.oid = i.indexrelid
JOIN pg_class t ON t.oid = i.indrelid
JOIN pg_namespace n ON n.oid = c.relnamespace
LEFT JOIN unnest(i.indkey) WITH ORDINALITY AS k(attnum, ord) ON k.attnum > 0
LEFT JOIN pg_attribute a ON a.attrelid = t.oid AND a.attnum = k.attnum
WHERE n.nspname = 'public'
  AND c.relname = 'idx_invoices_stripe_payment_intent_id'
GROUP BY c.relname, t.relname, i.indisvalid, i.indisready, i.indisunique, i.indexrelid;

-- 3. SQLx migration table state
SELECT version, description, success, encode(checksum, 'hex') AS sha384_hex, execution_time
FROM public._sqlx_migrations
WHERE version >= 34
ORDER BY version;
```

---

## 4. Controlled Forward-Only Recovery Runbook (Option B — 4 Cases)

### Case 1 — Concurrent build failed & left invalid index (`indisvalid = false`, `version = 36` absent)
1. Run Query 1 (duplicates). If `> 0`, remediate duplicate rows with accounting review (never altering issued amounts/dates/numbers) and re-confirm `0` rows.
2. Remove only the broken invalid index online:
   ```sql
   DROP INDEX CONCURRENTLY public.idx_invoices_stripe_payment_intent_id;
   ```
3. Confirm Query 2 returns `0` rows, then re-run `sqlx migrate run` (or `Migrator::run`). Verify `indisvalid = true AND indisready = true AND indisunique = true` and `version = 36` in `_sqlx_migrations`.

### Case 2 — Concurrent build succeeded, but process disconnected before `_sqlx_migrations` insert (`indisvalid = true`, `indisready = true`, `indisunique = true`, `version = 36` absent)
**Never drop a valid working unique index** (that would reopen a `PaymentIntent` replay race window). Run this guarded, transactional PL/pgSQL reconciliation block supplying `<EXACT_FILE_SHA384_HEX>` from the deployed binary/artifact:

```sql
BEGIN;
DO $$
DECLARE
    v_table TEXT;
    v_valid BOOLEAN;
    v_ready BOOLEAN;
    v_unique BOOLEAN;
    v_cols NAME[];
    v_def TEXT;
    v_dups BIGINT;
    v_checksum BYTEA := decode('<EXACT_FILE_SHA384_HEX>', 'hex');
BEGIN
    IF octet_length(v_checksum) <> 48 THEN
        RAISE EXCEPTION 'Invalid SHA-384 checksum length';
    END IF;

    IF EXISTS (SELECT 1 FROM public._sqlx_migrations WHERE version = 36) THEN
        RAISE EXCEPTION 'Migration 36 already recorded in _sqlx_migrations';
    END IF;

    SELECT count(*) INTO v_dups
    FROM (
        SELECT 1
        FROM public.invoices
        WHERE stripe_payment_intent_id IS NOT NULL
        GROUP BY stripe_payment_intent_id
        HAVING count(*) > 1
    ) d;
    IF v_dups > 0 THEN
        RAISE EXCEPTION 'Cannot reconcile: duplicate stripe_payment_intent_id rows exist';
    END IF;

    SELECT t.relname, i.indisvalid, i.indisready, i.indisunique,
           array_agg(a.attname ORDER BY k.ord),
           pg_get_indexdef(i.indexrelid)
    INTO v_table, v_valid, v_ready, v_unique, v_cols, v_def
    FROM pg_index i
    JOIN pg_class c ON c.oid = i.indexrelid
    JOIN pg_class t ON t.oid = i.indrelid
    JOIN pg_namespace n ON n.oid = c.relnamespace
    LEFT JOIN unnest(i.indkey) WITH ORDINALITY AS k(attnum, ord) ON k.attnum > 0
    LEFT JOIN pg_attribute a ON a.attrelid = t.oid AND a.attnum = k.attnum
    WHERE n.nspname = 'public'
      AND c.relname = 'idx_invoices_stripe_payment_intent_id'
    GROUP BY t.relname, i.indisvalid, i.indisready, i.indisunique, i.indexrelid;

    IF v_table IS DISTINCT FROM 'invoices'
       OR v_valid IS DISTINCT FROM TRUE
       OR v_ready IS DISTINCT FROM TRUE
       OR v_unique IS DISTINCT FROM TRUE
       OR v_cols IS DISTINCT FROM ARRAY['stripe_payment_intent_id']::name[]
       OR v_def IS DISTINCT FROM
          'CREATE UNIQUE INDEX idx_invoices_stripe_payment_intent_id ON public.invoices USING btree (stripe_payment_intent_id) WHERE (stripe_payment_intent_id IS NOT NULL)'
    THEN
        RAISE EXCEPTION 'Catalog definition mismatch: table=%, valid=%, ready=%, unique=%, cols=%, def=%',
            v_table, v_valid, v_ready, v_unique, v_cols, v_def;
    END IF;

    INSERT INTO public._sqlx_migrations (version, description, success, checksum, execution_time)
    VALUES (36, 'idx invoices stripe pi concurrent', TRUE, v_checksum, 0);
END
$$;
COMMIT;
```

### Case 3 — Existing same-name index has wrong definition (`indisunique = false`, wrong column, or wrong `WHERE` predicate; `version = 36` absent)
1. Confirm Query 1 returns `0` duplicate `stripe_payment_intent_id` rows.
2. Build valid replacement online under a temporary name:
   ```sql
   CREATE UNIQUE INDEX CONCURRENTLY idx_invoices_stripe_payment_intent_id_new
       ON public.invoices (stripe_payment_intent_id)
       WHERE stripe_payment_intent_id IS NOT NULL;
   ```
3. Verify the `_new` index has `indisvalid = true AND indisready = true AND indisunique = true`.
4. Swap online:
   ```sql
   DROP INDEX CONCURRENTLY public.idx_invoices_stripe_payment_intent_id;
   ALTER INDEX public.idx_invoices_stripe_payment_intent_id_new
       RENAME TO idx_invoices_stripe_payment_intent_id;
   ```
5. Execute Case 2 guarded reconciliation so `_sqlx_migrations` matches the exact committed `0036` bytes.

### Case 4 — `version = 36` is recorded in `_sqlx_migrations`, but checksum (`VersionMismatch(36)`) or live DB index mismatches
**Never delete or mutate `_sqlx_migrations` rows, and never rewrite committed `0036` bytes after deployment.**
1. If build/deploy artifact bytes drifted from the committed migration (causing `MigrateError::VersionMismatch(36)`), restore `0036` bytes from the exact git release tag/commit (check LF vs CRLF line endings).
2. If the database index itself is missing, invalid (`indisvalid = false`), or misdefined despite `version = 36` being present in `_sqlx_migrations`, leave `0036` and `_sqlx_migrations` untouched and ship a **new forward corrective `-- no-transaction` migration `0037_repair_invoices_stripe_pi_index.sql`**:
   - For Option B recovery: create `idx_invoices_stripe_payment_intent_id_repair` `CONCURRENTLY` (or drop invalid index `CONCURRENTLY` -> create valid index `CONCURRENTLY` -> atomic `RENAME`). Note that because PostgreSQL rejects multiple statements with `CREATE INDEX CONCURRENTLY` inside one multi-statement string or if a single `no_tx` file mixes DDL statements (`CONCURRENTLY` cannot run inside an implicit multi-statement transaction block in Postgres!), SQLx `no_tx` multi-statement behavior must be respected carefully: split each `CONCURRENTLY` command into its own migration version or separate `psql` / single-statement `-- no-transaction` migrations! *(Verified in local `stage1_migrations_test.rs`: issuing a single statement per `-- no-transaction` migration or using sequential `-- no-transaction` repair files avoids PostgreSQL `25001: CREATE INDEX CONCURRENTLY cannot run inside a transaction block` caused by Postgres implicit transaction wrapping around multi-statement protocol strings!)*

---

## 5. Staging Promotion Checklist (Blocked Pending Sign-Off)

1. Run live duplicate preflight on production (read-only).
2. Create verified `pg_dump --format=custom` backup and test-restore to `vaultscope_staging_test`.
3. Benchmark chosen candidate (`Option A` with maintenance window vs. `Option B` `CONCURRENTLY`) on `vaultscope_staging_test` under realistic load.
4. Obtain explicit operator sign-off choosing Option A or Option B.
5. Copy the chosen single file into `crates/db/migrations/` **only after** approval, preserving exact LF/UTF-8 bytes used for checksum verification.
