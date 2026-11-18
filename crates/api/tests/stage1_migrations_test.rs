use std::fs;
use std::path::{Path, PathBuf};

use chrono::NaiveDate;
use rust_decimal::Decimal;
use sha2::{Digest, Sha384};
use sqlx::migrate::{MigrateError, Migrator};
use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;
use uuid::Uuid;

use vaultscope_db::models::Invoice;

mod common;

struct TempMigrationDir {
    path: PathBuf,
}

impl TempMigrationDir {
    fn new(label: &str) -> Self {
        let path = std::env::temp_dir().join(format!("vs_{label}_{}", Uuid::now_v7().simple()));
        fs::create_dir_all(&path).expect("create temp migration dir");
        Self { path }
    }

    fn path(&self) -> &Path {
        &self.path
    }

    fn repo_migrations_dir() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../db/migrations")
    }

    fn repo_candidates_dir() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../db/migrations_candidates")
    }

    fn copy_repo_migrations_up_to(&self, max_version: i64) {
        let src_dir = Self::repo_migrations_dir();
        for entry in fs::read_dir(&src_dir).expect("read migrations dir") {
            let entry = entry.expect("dir entry");
            let file_path = entry.path();
            if !file_path.is_file() {
                continue;
            }
            let name = entry.file_name();
            let name_str = name.to_string_lossy();
            let Some((ver_str, rest)) = name_str.split_once('_') else {
                continue;
            };
            if !rest.ends_with(".sql") {
                continue;
            }
            let ver: i64 = ver_str.parse().expect("valid migration version");
            if ver <= max_version {
                fs::copy(&file_path, self.path.join(&name)).expect("copy migration sql");
            }
        }
    }

    fn copy_candidate(&self, subpath: &str, filename: &str) -> PathBuf {
        let src = Self::repo_candidates_dir().join(subpath).join(filename);
        let dst = self.path.join(filename);
        fs::copy(&src, &dst).expect("copy candidate migration");
        dst
    }

    fn write_file(&self, filename: &str, contents: &str) -> PathBuf {
        let dst = self.path.join(filename);
        fs::write(&dst, contents).expect("write migration file");
        dst
    }
}

impl Drop for TempMigrationDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn sha384_hex(path: &Path) -> String {
    let bytes = fs::read(path).expect("read migration bytes for sha384");
    hex::encode(Sha384::digest(&bytes))
}

fn test_db_urls() -> (String, String) {
    let base_url = std::env::var("TEST_DATABASE_URL")
        .unwrap_or_else(|_| "postgres://postgres:postgres@localhost/vaultscope_test".to_string());

    let (without_query, query_suffix) = match base_url.split_once('?') {
        Some((left, q)) => (left, format!("?{q}")),
        None => (base_url.as_str(), String::new()),
    };
    let (prefix, db_name) = without_query
        .rsplit_once('/')
        .expect("TEST_DATABASE_URL must contain /<database>");

    assert_ne!(
        db_name, "vaultscope",
        "Safety refusal: Stage 1 tests must NEVER target the local dev database 'vaultscope'"
    );
    assert!(
        db_name.contains("test"),
        "Safety refusal: TEST_DATABASE_URL database '{db_name}' must contain 'test'"
    );

    let maint_url = format!("{prefix}/postgres{query_suffix}");
    let scratch_name = format!("vs_stage1_{}", Uuid::now_v7().simple());
    let scratch_url = format!("{prefix}/{scratch_name}{query_suffix}");
    (maint_url, format!("{scratch_name}|{scratch_url}"))
}

async fn run_migrator_isolated(db_url: &str, migrator: &Migrator) -> Result<(), MigrateError> {
    // Each CLI/pod migration attempt uses a fresh 1-connection pool that is explicitly
    // closed afterwards, so an early Err return (before sqlx-core 0.8.6 reaches conn.unlock())
    // closes the Postgres backend session and releases pg_advisory_lock instead of pinning it
    // inside the long-lived scratch_pool.
    let temp_pool = PgPoolOptions::new()
        .max_connections(1)
        .connect(db_url)
        .await?;
    let res = migrator.run(&temp_pool).await;
    temp_pool.close().await;
    res
}

async fn apply_repo_migrations_up_to_35(db_url: &str) {
    let base_dir = TempMigrationDir::new("base_up_to_35");
    base_dir.copy_repo_migrations_up_to(35);
    let base_migrator = Migrator::new(base_dir.path()).await.unwrap();
    run_migrator_isolated(db_url, &base_migrator).await.unwrap();
}

async fn with_scratch_db<F, Fut>(test_fn: F)
where
    F: FnOnce(PgPool, String) -> Fut,
    Fut: std::future::Future<Output = ()> + Send + 'static,
{
    let (maint_url, packed) = test_db_urls();
    let (scratch_name, scratch_url) = packed.split_once('|').unwrap();
    let scratch_name = scratch_name.to_string();
    let scratch_url_str = scratch_url.to_string();

    let maint_pool = PgPoolOptions::new()
        .max_connections(2)
        .connect(&maint_url)
        .await
        .expect("connect to postgres maintenance db");

    sqlx::query(&format!("CREATE DATABASE \"{scratch_name}\""))
        .execute(&maint_pool)
        .await
        .expect("create isolated scratch database");

    let scratch_pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&scratch_url_str)
        .await
        .expect("connect to isolated scratch database");

    let pool_for_cleanup = scratch_pool.clone();
    let fut = test_fn(scratch_pool, scratch_url_str);
    let handle = tokio::spawn(fut);
    let outcome = handle.await;

    pool_for_cleanup.close().await;
    let _ = sqlx::query(&format!(
        "DROP DATABASE IF EXISTS \"{scratch_name}\" WITH (FORCE)"
    ))
    .execute(&maint_pool)
    .await;
    maint_pool.close().await;

    if let Err(join_err) = outcome {
        if let Ok(payload) = join_err.try_into_panic() {
            std::panic::resume_unwind(payload);
        }
        panic!("scratch DB test task cancelled cleanly unexpectedly");
    }
}

async fn index_state(
    pool: &PgPool,
    index_name: &str,
) -> Option<(String, String, bool, bool, bool, Vec<String>, String)> {
    sqlx::query_as(
        r#"
        SELECT
            c.relname::text AS index_name,
            t.relname::text AS table_name,
            i.indisvalid,
            i.indisready,
            i.indisunique,
            array_agg(a.attname ORDER BY k.ord)::text[] AS indexed_columns,
            pg_get_indexdef(i.indexrelid) AS index_def
        FROM pg_index i
        JOIN pg_class c ON c.oid = i.indexrelid
        JOIN pg_class t ON t.oid = i.indrelid
        JOIN pg_namespace n ON n.oid = c.relnamespace
        LEFT JOIN unnest(i.indkey) WITH ORDINALITY AS k(attnum, ord) ON k.attnum > 0
        LEFT JOIN pg_attribute a ON a.attrelid = t.oid AND a.attnum = k.attnum
        WHERE n.nspname = 'public'
          AND c.relname = $1
        GROUP BY c.relname, t.relname, i.indisvalid, i.indisready, i.indisunique, i.indexrelid
        "#,
    )
    .bind(index_name)
    .fetch_optional(pool)
    .await
    .expect("query pg_index state")
}

fn guarded_case2_reconciliation_sql(checksum_hex: &str) -> String {
    format!(
        r#"
DO $$
DECLARE
    v_table TEXT;
    v_valid BOOLEAN;
    v_ready BOOLEAN;
    v_unique BOOLEAN;
    v_cols NAME[];
    v_def TEXT;
    v_dups BIGINT;
    v_checksum BYTEA := decode('{checksum_hex}', 'hex');
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
"#
    )
}

#[tokio::test]
async fn test_vaultscope_test_migrations_through_0035_and_seeded_preservation() {
    let pool = common::setup_test_db().await;

    // 1. Verify active migrations end at 0035 (neither 0036 candidate is active yet)
    let (max_ver, count, failed_count): (Option<i64>, i64, i64) = sqlx::query_as(
        "SELECT MAX(version), COUNT(*), COUNT(*) FILTER (WHERE success = false) FROM _sqlx_migrations",
    )
    .fetch_one(&pool)
    .await
    .expect("query _sqlx_migrations");
    assert_eq!(max_ver, Some(37));
    assert_eq!(count, 35); // 0001..0005, 0007..0035, 0037 (no 0006 or 0036 file in repo)
    assert_eq!(failed_count, 0);

    // 2. Verify index idx_invoices_stripe_payment_intent_id is NOT created prematurely
    assert!(
        index_state(&pool, "idx_invoices_stripe_payment_intent_id")
            .await
            .is_none(),
        "Candidate 0036 must not be active in crates/db/migrations yet"
    );

    // 3. Verify seeded historical invoice INV-2026-0001 is preserved verbatim and deserializes cleanly
    let seeded_id = Uuid::parse_str("00000000-0000-0000-0000-000000000070").unwrap();
    let inv: Invoice = sqlx::query_as("SELECT * FROM invoices WHERE id = $1")
        .bind(seeded_id)
        .fetch_one(&pool)
        .await
        .expect("deserialize seeded Invoice");

    assert_eq!(inv.invoice_number, "INV-2026-0001");
    assert!(inv.customer_id.is_some());
    assert_eq!(inv.subtotal, Decimal::new(400, 2));
    assert_eq!(inv.tax_rate, Decimal::new(1900, 2));
    assert_eq!(inv.tax_amount, Decimal::new(76, 2));
    assert_eq!(inv.total, Decimal::new(476, 2));
    assert_eq!(inv.tax_regime, "unverified_pending");
    assert!(
        inv.retention_until.is_none(),
        "Historical retention_until must stay NULL pending Steuerberater review"
    );
    assert!(!inv.retention_hold);
    assert!(inv.corrects_invoice_id.is_none());
    assert!(!inv.retention_resolution_approved);

    // Seeded customer in 0011 had empty postal_code -> snapshot_complete MUST be false (zero fabrication)
    let snap = inv
        .billing_snapshot
        .as_ref()
        .expect("billing_snapshot backfilled by 0035")
        .0
        .clone();
    assert!(!snap.snapshot_complete);
    assert_eq!(snap.name, "Anton Schmidt");
    assert_eq!(snap.address_line1, "Gottlob-Spiess-Straße 2");
    assert_eq!(snap.postal_code, "");
    assert_eq!(snap.city, "Sachsenheim");
    assert_eq!(snap.country, "Germany");
    assert_eq!(snap.missing_fields, vec!["postal_code".to_string()]);
    assert_eq!(snap.snapshot_source, "backfill_0035");

    // 4. Verify all 4 Stage 1 foreign keys are validated and set to ON DELETE SET NULL ('n')
    let fks: Vec<(String, bool, String)> = sqlx::query_as(
        r#"
        SELECT conname::text, convalidated, confdeltype::text
        FROM pg_constraint
        WHERE conname IN (
            'invoices_customer_id_fkey',
            'invoice_line_items_service_id_fkey',
            'tickets_related_service_id_fkey',
            'jobs_service_id_fkey'
        )
        ORDER BY conname
        "#,
    )
    .fetch_all(&pool)
    .await
    .expect("query pg_constraint");
    assert_eq!(fks.len(), 4);
    for (name, validated, deltype) in fks {
        assert!(validated, "FK {name} must be validated by 0035");
        assert_eq!(deltype, "n", "FK {name} must use ON DELETE SET NULL");
    }

    // 5. Verify fail-closed BEFORE DELETE trigger exists on invoices
    let trigger_exists: bool = sqlx::query_scalar(
        r#"
        SELECT EXISTS (
            SELECT 1
            FROM pg_trigger t
            JOIN pg_class c ON c.oid = t.tgrelid
            WHERE c.relname = 'invoices'
              AND t.tgname = 'trg_prevent_statutory_invoice_deletion'
              AND NOT t.tgisinternal
        )
        "#,
    )
    .fetch_one(&pool)
    .await
    .expect("check trigger existence");
    assert!(trigger_exists);
}

#[tokio::test]
async fn test_stage1_synthetic_backfill_triggers_fks_and_nullable_deserialization() {
    with_scratch_db(|pool, db_url| async move {
        // Apply 0001..0033 first so we can plant pre-0034 synthetic records
        let pre_dir = TempMigrationDir::new("pre0034");
        pre_dir.copy_repo_migrations_up_to(33);
        let pre_migrator = Migrator::new(pre_dir.path()).await.unwrap();
        run_migrator_isolated(&db_url, &pre_migrator).await.unwrap();

        let cust_complete = Uuid::now_v7();
        let cust_incomplete = Uuid::now_v7();
        let cust_draft = Uuid::now_v7();
        let svc_id = Uuid::now_v7();
        let inv_complete_id = Uuid::now_v7();
        let inv_incomplete_id = Uuid::now_v7();
        let inv_draft_id = Uuid::now_v7();
        let ticket_id = Uuid::now_v7();
        let job_id = Uuid::now_v7();

        let prod_id = Uuid::parse_str("00000000-0000-0000-0000-000000000002").unwrap();

        sqlx::query(
            r#"
            INSERT INTO customers (id, external_id, username, name, email, company, address, postal_code, city, country, vat_id, status, email_verified)
            VALUES
                ($1, 'ext_comp', 'erika_comp', 'Erika Musterfrau', 'erika@example.de', 'Muster Studio', 'Hauptstrasse 12', '10115', 'Berlin', 'Deutschland', 'DE999888777', 'active', true),
                ($2, 'ext_inc', 'max_inc', 'Max Incomplete', 'max_inc@example.de', '', '', '', 'Munich', 'Germany', '', 'active', true),
                ($3, 'ext_draft', 'draft_user', 'Draft Customer', 'draft@example.de', '', 'Nebenweg 4', '74343', 'Sachsenheim', 'Germany', '', 'active', true)
            "#,
        )
        .bind(cust_complete)
        .bind(cust_incomplete)
        .bind(cust_draft)
        .execute(&pool)
        .await
        .unwrap();

        sqlx::query(
            "INSERT INTO services (id, customer_id, product_id, name, status, price) VALUES ($1, $2, $3, 'sftp-test-1', 'running', 10.00)",
        )
        .bind(svc_id)
        .bind(cust_complete)
        .bind(prod_id)
        .execute(&pool)
        .await
        .unwrap();

        sqlx::query(
            "INSERT INTO tickets (id, ticket_number, customer_id, category, subject, related_service_id) VALUES ($1, 'TICK-S1-9001', $2, 'support', 'Service ref test', $3)",
        )
        .bind(ticket_id)
        .bind(cust_incomplete)
        .bind(svc_id)
        .execute(&pool)
        .await
        .unwrap();

        sqlx::query(
            "INSERT INTO jobs (id, task, target_api, customer_id, service_id, status) VALUES ($1, 'provision', 'sftpgo', $2, $3, 'completed')",
        )
        .bind(job_id)
        .bind(cust_incomplete)
        .bind(svc_id)
        .execute(&pool)
        .await
        .unwrap();

        let issue_complete = NaiveDate::from_ymd_opt(2026, 1, 15).unwrap();
        let issue_incomplete = NaiveDate::from_ymd_opt(2026, 2, 1).unwrap();

        sqlx::query(
            r#"
            INSERT INTO invoices (id, invoice_number, customer_id, status, subtotal, tax_rate, tax_amount, total, issue_date, due_date, stripe_payment_intent_id, notes)
            VALUES
                ($1, 'INV-SYN-COMPLETE-1', $2, 'paid', 10.00, 0.00, 0.00, 10.00, $4, $4, 'pi_syn_complete_1', 'Original note'),
                ($3, 'INV-SYN-INCOMPLETE-1', $5, 'paid', 12.00, 19.00, 2.28, 14.28, $6, $6, 'pi_syn_incomplete_1', 'Legacy VAT'),
                ($7, 'INV-SYN-DRAFT-1', $8, 'draft', 10.00, 0.00, 0.00, 10.00, $6, $6, NULL, 'Draft note')
            "#,
        )
        .bind(inv_complete_id)
        .bind(cust_complete)
        .bind(inv_incomplete_id)
        .bind(issue_complete)
        .bind(cust_incomplete)
        .bind(issue_incomplete)
        .bind(inv_draft_id)
        .bind(cust_draft)
        .execute(&pool)
        .await
        .unwrap();

        sqlx::query(
            "INSERT INTO invoice_line_items (id, invoice_id, description, quantity, unit_price, total, service_id, sort_order) VALUES ($1, $2, 'SFTP Vault', 1, 10.00, 10.00, $3, 0)",
        )
        .bind(Uuid::now_v7())
        .bind(inv_complete_id)
        .bind(svc_id)
        .execute(&pool)
        .await
        .unwrap();

        // Now run full migrations 0034 + 0035 via workspace Migrator
        vaultscope_db::run_migrations(&pool).await.unwrap();

        // Check complete snapshot + verbatim historical values + NULL retention_until
        let inv_comp: Invoice = sqlx::query_as("SELECT * FROM invoices WHERE id = $1")
            .bind(inv_complete_id)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(inv_comp.subtotal, Decimal::new(1000, 2));
        assert_eq!(inv_comp.tax_rate, Decimal::new(0, 2));
        assert_eq!(inv_comp.tax_amount, Decimal::new(0, 2));
        assert_eq!(inv_comp.total, Decimal::new(1000, 2));
        assert_eq!(inv_comp.issue_date, issue_complete);
        assert!(inv_comp.retention_until.is_none());
        let snap_comp = inv_comp.billing_snapshot.as_ref().unwrap().0.clone();
        assert!(snap_comp.snapshot_complete);
        assert!(snap_comp.missing_fields.is_empty());
        assert_eq!(snap_comp.address_line1, "Hauptstrasse 12");
        assert_eq!(snap_comp.postal_code, "10115");

        // Check incomplete snapshot (no fabrication!)
        let inv_inc: Invoice = sqlx::query_as("SELECT * FROM invoices WHERE id = $1")
            .bind(inv_incomplete_id)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(inv_inc.subtotal, Decimal::new(1200, 2));
        assert_eq!(inv_inc.tax_rate, Decimal::new(1900, 2));
        assert_eq!(inv_inc.tax_amount, Decimal::new(228, 2));
        assert_eq!(inv_inc.total, Decimal::new(1428, 2));
        assert_eq!(inv_inc.issue_date, issue_incomplete);
        assert!(inv_inc.retention_until.is_none());
        let snap_inc = inv_inc.billing_snapshot.as_ref().unwrap().0.clone();
        assert!(!snap_inc.snapshot_complete);
        assert_eq!(snap_inc.address_line1, "");
        assert_eq!(snap_inc.postal_code, "");
        assert_eq!(
            snap_inc.missing_fields,
            vec!["address_line1".to_string(), "postal_code".to_string()]
        );

        // Verify 0035 backfill SQL is strictly idempotent (touches 0 existing rows)
        let migration_0035 =
            fs::read_to_string(TempMigrationDir::repo_migrations_dir().join("0035_backfill_and_validate_constraints.sql"))
                .unwrap();
        let update_only_end = migration_0035.find("-- Online validation").unwrap();
        sqlx::raw_sql(&migration_0035[..update_only_end])
            .execute(&pool)
            .await
            .unwrap();
        let inv_comp_after: Invoice = sqlx::query_as("SELECT * FROM invoices WHERE id = $1")
            .bind(inv_complete_id)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(
            inv_comp_after.billing_snapshot.as_ref().unwrap().0.captured_at,
            snap_comp.captured_at
        );

        // Edge case: unlinked invoice inserted with NULL customer_id and NULL billing_snapshot
        // before backfill gets backfill_0035_missing_customer cleanly without failing
        let inv_orphan_id = Uuid::now_v7();
        sqlx::query(
            "INSERT INTO invoices (id, invoice_number, customer_id, status, subtotal, tax_rate, tax_amount, total, issue_date, due_date) VALUES ($1, 'INV-ORPHAN-1', NULL, 'paid', 5.00, 0.00, 0.00, 5.00, $2, $2)",
        )
        .bind(inv_orphan_id)
        .bind(issue_complete)
        .execute(&pool)
        .await
        .unwrap();
        sqlx::raw_sql(&migration_0035[..update_only_end])
            .execute(&pool)
            .await
            .unwrap();
        let inv_orphan: Invoice = sqlx::query_as("SELECT * FROM invoices WHERE id = $1")
            .bind(inv_orphan_id)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert!(inv_orphan.customer_id.is_none());
        let orphan_snap = inv_orphan.billing_snapshot.as_ref().unwrap().0.clone();
        assert!(!orphan_snap.snapshot_complete);
        assert_eq!(orphan_snap.snapshot_source, "backfill_0035_missing_customer");
        assert!(orphan_snap.missing_fields.contains(&"customer_record".to_string()));

        // Trigger tests: prevent_statutory_invoice_deletion()
        // A. Paid invoice with unresolved retention_until IS NULL -> MUST fail closed
        let del_unresolved = sqlx::query("DELETE FROM invoices WHERE id = $1")
            .bind(inv_complete_id)
            .execute(&pool)
            .await;
        assert!(del_unresolved.is_err(), "Non-draft with NULL retention_until must be blocked");

        // B. Future retention_until -> MUST fail
        sqlx::query("UPDATE invoices SET retention_until = CURRENT_DATE + 365 WHERE id = $1")
            .bind(inv_incomplete_id)
            .execute(&pool)
            .await
            .unwrap();
        assert!(
            sqlx::query("DELETE FROM invoices WHERE id = $1")
                .bind(inv_incomplete_id)
                .execute(&pool)
                .await
                .is_err()
        );

        // C. Expired date BUT retention_hold = true -> MUST STILL fail (§ 147(3) S. 5 AO)
        sqlx::query("UPDATE invoices SET retention_until = CURRENT_DATE - 1, retention_hold = true WHERE id = $1")
            .bind(inv_incomplete_id)
            .execute(&pool)
            .await
            .unwrap();
        assert!(
            sqlx::query("DELETE FROM invoices WHERE id = $1")
                .bind(inv_incomplete_id)
                .execute(&pool)
                .await
                .is_err()
        );

        // D. Unissued draft invoice without retention_hold -> allowed
        let res = sqlx::query("DELETE FROM invoices WHERE id = $1")
            .bind(inv_draft_id)
            .execute(&pool)
            .await
            .unwrap();
        assert_eq!(res.rows_affected(), 1);

        // FK ON DELETE SET NULL tests on service references
        sqlx::query("DELETE FROM services WHERE id = $1")
            .bind(svc_id)
            .execute(&pool)
            .await
            .unwrap();
        let line_svc: Option<Uuid> =
            sqlx::query_scalar("SELECT service_id FROM invoice_line_items WHERE invoice_id = $1")
                .bind(inv_complete_id)
                .fetch_one(&pool)
                .await
                .unwrap();
        let ticket_svc: Option<Uuid> =
            sqlx::query_scalar("SELECT related_service_id FROM tickets WHERE id = $1")
                .bind(ticket_id)
                .fetch_one(&pool)
                .await
                .unwrap();
        let job_svc: Option<Uuid> =
            sqlx::query_scalar("SELECT service_id FROM jobs WHERE id = $1")
                .bind(job_id)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert!(line_svc.is_none());
        assert!(ticket_svc.is_none());
        assert!(job_svc.is_none());

        // FK ON DELETE SET NULL test on customer deletion (invoice preserved + deserializable with customer_id = None)
        sqlx::query("DELETE FROM customers WHERE id = $1")
            .bind(cust_complete)
            .execute(&pool)
            .await
            .unwrap();
        let unlinked_inv: Invoice = sqlx::query_as("SELECT * FROM invoices WHERE id = $1")
            .bind(inv_complete_id)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert!(unlinked_inv.customer_id.is_none());
        assert!(unlinked_inv.billing_snapshot.as_ref().unwrap().0.snapshot_complete);

        // § 31(5) UStDV amending link via corrects_invoice_id
        let correction_id = Uuid::now_v7();
        sqlx::query(
            "INSERT INTO invoices (id, invoice_number, customer_id, status, subtotal, tax_rate, tax_amount, total, issue_date, due_date, corrects_invoice_id, notes) VALUES ($1, 'INV-CORR-2026-1', NULL, 'paid', 0.00, 0.00, 0.00, 0.00, $2, $2, $3, 'Section 31(5) UStDV correction')",
        )
        .bind(correction_id)
        .bind(issue_complete)
        .bind(inv_complete_id)
        .execute(&pool)
        .await
        .unwrap();
        let corr_inv: Invoice = sqlx::query_as("SELECT * FROM invoices WHERE id = $1")
            .bind(correction_id)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(corr_inv.corrects_invoice_id, Some(inv_complete_id));
    })
    .await;
}

#[tokio::test]
async fn test_0036_option_a_transactional_candidate() {
    with_scratch_db(|pool, db_url| async move {
        let dir = TempMigrationDir::new("opt_a");
        dir.copy_repo_migrations_up_to(35);
        dir.copy_candidate(
            "option_a_transactional",
            "0036_idx_invoices_stripe_pi_transactional.sql",
        );

        let migrator = Migrator::new(dir.path()).await.unwrap();
        let m36 = migrator.iter().find(|m| m.version == 36).unwrap();
        assert!(!m36.no_tx, "Option A must run inside a transaction");

        // Pre-apply 0001..0035 first, insert duplicates, and confirm Option A rolls back atomically
        apply_repo_migrations_up_to_35(&db_url).await;
        let cust_id = Uuid::parse_str("01a09b2e-0532-7403-86db-a10464205764").unwrap();
        let dup_a = Uuid::now_v7();
        let dup_b = Uuid::now_v7();
        let today = NaiveDate::from_ymd_opt(2026, 5, 1).unwrap();
        sqlx::query(
            "INSERT INTO invoices (id, invoice_number, customer_id, status, subtotal, tax_rate, tax_amount, total, issue_date, due_date, stripe_payment_intent_id) VALUES ($1, 'INV-OPTA-1', $3, 'paid', 10.00, 0.00, 0.00, 10.00, $4, $4, 'pi_dup_opta'), ($2, 'INV-OPTA-2', $3, 'paid', 10.00, 0.00, 0.00, 10.00, $4, $4, 'pi_dup_opta')",
        )
        .bind(dup_a)
        .bind(dup_b)
        .bind(cust_id)
        .bind(today)
        .execute(&pool)
        .await
        .unwrap();

        let err = run_migrator_isolated(&db_url, &migrator)
            .await
            .unwrap_err();
        assert!(matches!(err, MigrateError::ExecuteMigration(_, 36)));
        assert!(
            index_state(&pool, "idx_invoices_stripe_payment_intent_id")
                .await
                .is_none(),
            "Transactional Option A must roll back cleanly on duplicate failure"
        );
        let row_36_exists: bool =
            sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM _sqlx_migrations WHERE version = 36)")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert!(!row_36_exists);

        // Clear duplicate PI and re-run Option A -> succeeds and enforces partial uniqueness
        sqlx::query("UPDATE invoices SET stripe_payment_intent_id = 'pi_unique_opta_2' WHERE id = $1")
            .bind(dup_b)
            .execute(&pool)
            .await
            .unwrap();
        run_migrator_isolated(&db_url, &migrator).await.unwrap();

        let idx = index_state(&pool, "idx_invoices_stripe_payment_intent_id")
            .await
            .expect("index exists");
        assert_eq!(idx.1, "invoices");
        assert!(idx.2 && idx.3 && idx.4); // valid, ready, unique

        let dup_insert = sqlx::query(
            "INSERT INTO invoices (id, invoice_number, customer_id, status, subtotal, tax_rate, tax_amount, total, issue_date, due_date, stripe_payment_intent_id) VALUES ($1, 'INV-OPTA-3', $2, 'paid', 10.00, 0.00, 0.00, 10.00, $3, $3, 'pi_dup_opta')",
        )
        .bind(Uuid::now_v7())
        .bind(cust_id)
        .bind(today)
        .execute(&pool)
        .await;
        assert!(dup_insert.is_err(), "Unique index must reject duplicate PI");

        // Partial index must still allow multiple NULL stripe_payment_intent_id rows
        sqlx::query(
            "INSERT INTO invoices (id, invoice_number, customer_id, status, subtotal, tax_rate, tax_amount, total, issue_date, due_date, stripe_payment_intent_id) VALUES ($1, 'INV-OPTA-NULL-1', $3, 'draft', 10.00, 0.00, 0.00, 10.00, $4, $4, NULL), ($2, 'INV-OPTA-NULL-2', $3, 'draft', 10.00, 0.00, 0.00, 10.00, $4, $4, NULL)",
        )
        .bind(Uuid::now_v7())
        .bind(Uuid::now_v7())
        .bind(cust_id)
        .bind(today)
        .execute(&pool)
        .await
        .expect("partial index must allow multiple NULL stripe_payment_intent_id rows");
    })
    .await;
}

#[tokio::test]
async fn test_0036_option_b_clean_and_case1_invalid_index_recovery() {
    // Part 1: Clean Option B no_tx apply
    with_scratch_db(|pool, db_url| async move {
        let dir = TempMigrationDir::new("opt_b_clean");
        dir.copy_repo_migrations_up_to(35);
        let candidate_path = dir.copy_candidate(
            "option_b_concurrent",
            "0036_idx_invoices_stripe_pi_concurrent.sql",
        );

        let migrator = Migrator::new(dir.path()).await.unwrap();
        let m36 = migrator.iter().find(|m| m.version == 36).unwrap();
        assert!(m36.no_tx, "Option B must parse -- no-transaction");
        assert_eq!(hex::encode(&*m36.checksum), sha384_hex(&candidate_path));

        run_migrator_isolated(&db_url, &migrator).await.unwrap();

        let idx = index_state(&pool, "idx_invoices_stripe_payment_intent_id")
            .await
            .expect("Option B index exists");
        assert!(idx.2 && idx.3 && idx.4);
        assert_eq!(idx.5, vec!["stripe_payment_intent_id".to_string()]);
        let ok_row: (bool,) =
            sqlx::query_as("SELECT success FROM _sqlx_migrations WHERE version = 36")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert!(ok_row.0);
    })
    .await;

    // Part 2: Case 1 — Concurrent build fails on duplicates -> leaves indisvalid = false + absent row 36
    with_scratch_db(|pool, db_url| async move {
        apply_repo_migrations_up_to_35(&db_url).await;

        let dir = TempMigrationDir::new("opt_b_case1");
        dir.copy_repo_migrations_up_to(35);
        dir.copy_candidate(
            "option_b_concurrent",
            "0036_idx_invoices_stripe_pi_concurrent.sql",
        );
        let migrator = Migrator::new(dir.path()).await.unwrap();

        let cust_id = Uuid::parse_str("01a09b2e-0532-7403-86db-a10464205764").unwrap();
        let dup_a = Uuid::now_v7();
        let dup_b = Uuid::now_v7();
        let today = NaiveDate::from_ymd_opt(2026, 5, 1).unwrap();
        sqlx::query(
            "INSERT INTO invoices (id, invoice_number, customer_id, status, subtotal, tax_rate, tax_amount, total, issue_date, due_date, stripe_payment_intent_id) VALUES ($1, 'INV-C1-1', $3, 'paid', 10.00, 0.00, 0.00, 10.00, $4, $4, 'pi_dup_case1'), ($2, 'INV-C1-2', $3, 'paid', 10.00, 0.00, 0.00, 10.00, $4, $4, 'pi_dup_case1')",
        )
        .bind(dup_a)
        .bind(dup_b)
        .bind(cust_id)
        .bind(today)
        .execute(&pool)
        .await
        .unwrap();

        let err = run_migrator_isolated(&db_url, &migrator)
            .await
            .unwrap_err();
        assert!(matches!(err, MigrateError::ExecuteMigration(_, 36)));

        // Verify sqlx-postgres 0.8.6 did NOT insert success = false for no_tx failure
        let row_36_exists: bool =
            sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM _sqlx_migrations WHERE version = 36)")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert!(
            !row_36_exists,
            "sqlx 0.8.6 no_tx failure must leave version 36 absent from _sqlx_migrations"
        );
        let dirty_count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM _sqlx_migrations WHERE success = false")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(dirty_count, 0);

        // Verify broken invalid index was left behind
        let invalid_idx = index_state(&pool, "idx_invoices_stripe_payment_intent_id")
            .await
            .expect("invalid index residue exists");
        assert!(
            !invalid_idx.2,
            "Failed CONCURRENTLY build must leave indisvalid = false"
        );

        // Remediate duplicate value cleanly
        sqlx::query("UPDATE invoices SET stripe_payment_intent_id = 'pi_fixed_case1_b' WHERE id = $1")
            .bind(dup_b)
            .execute(&pool)
            .await
            .unwrap();

        // Re-running without dropping invalid index MUST fail closed (IF NOT EXISTS omitted!)
        let retry_before_drop = run_migrator_isolated(&db_url, &migrator).await;
        assert!(
            matches!(retry_before_drop, Err(MigrateError::ExecuteMigration(_, 36))),
            "Omitting IF NOT EXISTS must refuse retrying over invalid index"
        );

        // Case 1 recovery: DROP INDEX CONCURRENTLY + re-run Migrator
        sqlx::query("DROP INDEX CONCURRENTLY public.idx_invoices_stripe_payment_intent_id")
            .execute(&pool)
            .await
            .unwrap();
        assert!(index_state(&pool, "idx_invoices_stripe_payment_intent_id").await.is_none());

        run_migrator_isolated(&db_url, &migrator).await.unwrap();
        let valid_idx = index_state(&pool, "idx_invoices_stripe_payment_intent_id")
            .await
            .expect("recovered valid index exists");
        assert!(valid_idx.2 && valid_idx.3 && valid_idx.4);

        let dup_insert = sqlx::query(
            "INSERT INTO invoices (id, invoice_number, customer_id, status, subtotal, tax_rate, tax_amount, total, issue_date, due_date, stripe_payment_intent_id) VALUES ($1, 'INV-C1-3', $2, 'paid', 10.00, 0.00, 0.00, 10.00, $3, $3, 'pi_dup_case1')",
        )
        .bind(Uuid::now_v7())
        .bind(cust_id)
        .bind(today)
        .execute(&pool)
        .await;
        assert!(dup_insert.is_err(), "Recovered index must reject duplicate PI");
    })
    .await;
}

#[tokio::test]
async fn test_0036_option_b_case2_disconnected_after_ddl_reconciliation() {
    with_scratch_db(|pool, db_url| async move {
        apply_repo_migrations_up_to_35(&db_url).await;

        let dir = TempMigrationDir::new("opt_b_case2");
        dir.copy_repo_migrations_up_to(35);
        let candidate_path = dir.copy_candidate(
            "option_b_concurrent",
            "0036_idx_invoices_stripe_pi_concurrent.sql",
        );
        let migrator = Migrator::new(dir.path()).await.unwrap();
        let checksum_hex = sha384_hex(&candidate_path);

        // Simulate CREATE UNIQUE INDEX CONCURRENTLY completing before process crash/disconnect
        // prior to SQLx inserting version 36 into _sqlx_migrations
        sqlx::query(
            "CREATE UNIQUE INDEX CONCURRENTLY idx_invoices_stripe_payment_intent_id ON public.invoices (stripe_payment_intent_id) WHERE stripe_payment_intent_id IS NOT NULL",
        )
        .execute(&pool)
        .await
        .unwrap();

        let row_36_exists: bool =
            sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM _sqlx_migrations WHERE version = 36)")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert!(!row_36_exists);

        // Naive Migrator re-run fails 42P07; we must NOT drop the valid index
        assert!(matches!(
            run_migrator_isolated(&db_url, &migrator).await,
            Err(MigrateError::ExecuteMigration(_, 36))
        ));

        // Guard rejects invalid checksum length
        let bad_sql = guarded_case2_reconciliation_sql("deadbeef");
        assert!(sqlx::raw_sql(&bad_sql).execute(&pool).await.is_err());

        // Execute guarded Case 2 PL/pgSQL reconciliation cleanly
        let ok_sql = guarded_case2_reconciliation_sql(&checksum_hex);
        sqlx::raw_sql(&ok_sql).execute(&pool).await.unwrap();

        // Now Migrator runs cleanly with zero errors and working unique index untouched
        run_migrator_isolated(&db_url, &migrator).await.unwrap();
        let idx = index_state(&pool, "idx_invoices_stripe_payment_intent_id")
            .await
            .unwrap();
        assert!(idx.2 && idx.3 && idx.4);
    })
    .await;
}

#[tokio::test]
async fn test_0036_option_b_case3_misdefined_same_name_index_swap() {
    with_scratch_db(|pool, db_url| async move {
        apply_repo_migrations_up_to_35(&db_url).await;

        let dir = TempMigrationDir::new("opt_b_case3");
        dir.copy_repo_migrations_up_to(35);
        let candidate_path = dir.copy_candidate(
            "option_b_concurrent",
            "0036_idx_invoices_stripe_pi_concurrent.sql",
        );
        let migrator = Migrator::new(dir.path()).await.unwrap();
        let checksum_hex = sha384_hex(&candidate_path);

        // Pre-create misdefined non-unique index with matching name (no WHERE clause)
        sqlx::query(
            "CREATE INDEX idx_invoices_stripe_payment_intent_id ON public.invoices (stripe_payment_intent_id)",
        )
        .execute(&pool)
        .await
        .unwrap();

        let bad_idx = index_state(&pool, "idx_invoices_stripe_payment_intent_id")
            .await
            .unwrap();
        assert!(!bad_idx.4, "Misdefined index is non-unique");

        // Migrator fails fast instead of silently skipping
        assert!(matches!(
            run_migrator_isolated(&db_url, &migrator).await,
            Err(MigrateError::ExecuteMigration(_, 36))
        ));

        // Case 2 reconciliation MUST refuse this misdefined index
        let reconcile_sql = guarded_case2_reconciliation_sql(&checksum_hex);
        assert!(
            sqlx::raw_sql(&reconcile_sql).execute(&pool).await.is_err(),
            "Guarded reconciliation must reject non-unique / missing-predicate index"
        );

        // Case 3 online swap: build _new concurrently -> drop misdefined -> rename -> reconcile
        sqlx::query(
            "CREATE UNIQUE INDEX CONCURRENTLY idx_invoices_stripe_payment_intent_id_new ON public.invoices (stripe_payment_intent_id) WHERE stripe_payment_intent_id IS NOT NULL",
        )
        .execute(&pool)
        .await
        .unwrap();
        let new_idx = index_state(&pool, "idx_invoices_stripe_payment_intent_id_new")
            .await
            .unwrap();
        assert!(new_idx.2 && new_idx.3 && new_idx.4);

        sqlx::query("DROP INDEX CONCURRENTLY public.idx_invoices_stripe_payment_intent_id")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query(
            "ALTER INDEX public.idx_invoices_stripe_payment_intent_id_new RENAME TO idx_invoices_stripe_payment_intent_id",
        )
        .execute(&pool)
        .await
        .unwrap();

        // Guarded reconciliation succeeds now that catalog definition matches 100%
        sqlx::raw_sql(&reconcile_sql).execute(&pool).await.unwrap();
        run_migrator_isolated(&db_url, &migrator).await.unwrap();

        let repaired = index_state(&pool, "idx_invoices_stripe_payment_intent_id")
            .await
            .unwrap();
        assert!(repaired.2 && repaired.3 && repaired.4);
    })
    .await;
}

#[tokio::test]
async fn test_0036_option_b_case4_checksum_mismatch_and_forward_0037_repair() {
    with_scratch_db(|pool, db_url| async move {
        let dir = TempMigrationDir::new("opt_b_case4");
        dir.copy_repo_migrations_up_to(35);
        let candidate_path = dir.copy_candidate(
            "option_b_concurrent",
            "0036_idx_invoices_stripe_pi_concurrent.sql",
        );

        let migrator = Migrator::new(dir.path()).await.unwrap();
        run_migrator_isolated(&db_url, &migrator).await.unwrap();

        let original_checksum: Vec<u8> =
            sqlx::query_scalar("SELECT checksum FROM _sqlx_migrations WHERE version = 36")
                .fetch_one(&pool)
                .await
                .unwrap();

        // Part A: Simulated file-byte drift triggers MigrateError::VersionMismatch(36)
        let drifted_dir = TempMigrationDir::new("opt_b_case4_drift");
        drifted_dir.copy_repo_migrations_up_to(35);
        let original_sql = fs::read_to_string(&candidate_path).unwrap();
        drifted_dir.write_file(
            "0036_idx_invoices_stripe_pi_concurrent.sql",
            &format!("{original_sql}\n-- drifted whitespace comment\n"),
        );
        let drifted_migrator = Migrator::new(drifted_dir.path()).await.unwrap();
        let mismatch = run_migrator_isolated(&db_url, &drifted_migrator)
            .await
            .unwrap_err();
        assert!(
            matches!(mismatch, MigrateError::VersionMismatch(36)),
            "Expected VersionMismatch(36), got {mismatch:?}"
        );

        // Restoring original bytes resolves cleanly without editing _sqlx_migrations
        run_migrator_isolated(&db_url, &migrator).await.unwrap();

        // Part B: Recorded version=36 exists in _sqlx_migrations, but index in DB is broken/misdefined.
        // Forward-only corrective -- no-transaction migrations repair it without mutating 0036 or _sqlx_migrations.
        sqlx::query("DROP INDEX CONCURRENTLY public.idx_invoices_stripe_payment_intent_id")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query(
            "CREATE INDEX idx_invoices_stripe_payment_intent_id ON public.invoices (stripe_payment_intent_id)",
        )
        .execute(&pool)
        .await
        .unwrap();

        // Note each CONCURRENTLY command gets its own single-statement -- no-transaction migration
        // so Postgres executes it cleanly outside any implicit transaction block.
        dir.write_file(
            "0037_repair_build_stripe_pi_idx.sql",
            "-- no-transaction\nCREATE UNIQUE INDEX CONCURRENTLY idx_invoices_stripe_payment_intent_id_repair\n    ON public.invoices (stripe_payment_intent_id)\n    WHERE stripe_payment_intent_id IS NOT NULL;\n",
        );
        dir.write_file(
            "0038_repair_drop_old_stripe_pi_idx.sql",
            "-- no-transaction\nDROP INDEX CONCURRENTLY IF EXISTS public.idx_invoices_stripe_payment_intent_id;\n",
        );
        dir.write_file(
            "0039_repair_rename_stripe_pi_idx.sql",
            "ALTER INDEX public.idx_invoices_stripe_payment_intent_id_repair RENAME TO idx_invoices_stripe_payment_intent_id;\n",
        );

        let repair_migrator = Migrator::new(dir.path()).await.unwrap();
        run_migrator_isolated(&db_url, &repair_migrator)
            .await
            .unwrap();

        // Confirm version 36 checksum in _sqlx_migrations remained 100% untouched
        let checksum_after: Vec<u8> =
            sqlx::query_scalar("SELECT checksum FROM _sqlx_migrations WHERE version = 36")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(checksum_after, original_checksum);

        // Confirm valid unique index is active and enforces uniqueness
        let repaired = index_state(&pool, "idx_invoices_stripe_payment_intent_id")
            .await
            .unwrap();
        assert!(repaired.2 && repaired.3 && repaired.4);
        assert_eq!(repaired.5, vec!["stripe_payment_intent_id".to_string()]);

        let cust_id = Uuid::parse_str("01a09b2e-0532-7403-86db-a10464205764").unwrap();
        let today = NaiveDate::from_ymd_opt(2026, 5, 1).unwrap();
        sqlx::query(
            "INSERT INTO invoices (id, invoice_number, customer_id, status, subtotal, tax_rate, tax_amount, total, issue_date, due_date, stripe_payment_intent_id) VALUES ($1, 'INV-C4-1', $2, 'paid', 10.00, 0.00, 0.00, 10.00, $3, $3, 'pi_case4_unique')",
        )
        .bind(Uuid::now_v7())
        .bind(cust_id)
        .bind(today)
        .execute(&pool)
        .await
        .unwrap();
        let dup_insert = sqlx::query(
            "INSERT INTO invoices (id, invoice_number, customer_id, status, subtotal, tax_rate, tax_amount, total, issue_date, due_date, stripe_payment_intent_id) VALUES ($1, 'INV-C4-2', $2, 'paid', 10.00, 0.00, 0.00, 10.00, $3, $3, 'pi_case4_unique')",
        )
        .bind(Uuid::now_v7())
        .bind(cust_id)
        .bind(today)
        .execute(&pool)
        .await;
        assert!(dup_insert.is_err(), "Forward-repaired unique index must reject duplicate PI");
    })
    .await;
}
