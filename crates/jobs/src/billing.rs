use std::time::Duration;

use chrono::Utc;
use sqlx::PgPool;
use tokio::sync::watch;
use tracing::{error, info};
use uuid::Uuid;

#[derive(sqlx::FromRow)]
#[allow(dead_code)]
struct RenewalRow {
    id: Uuid,
    customer_id: Uuid,
    price: rust_decimal::Decimal,
    name: String,
    provider_resource_id: String,
    cancel_at_period_end: bool,
    product_category: String,
    provider: String,
    is_trial: bool,
    product_price: rust_decimal::Decimal,
    partner_id: Option<Uuid>,
    product_quota_gb: Option<i32>,
    partner_quota_gb_override: Option<i32>,
}

pub struct BillingRunner {
    pool: PgPool,
    poll_interval: Duration,
}

impl BillingRunner {
    pub fn new(pool: PgPool, poll_interval: Duration) -> Self {
        Self {
            pool,
            poll_interval,
        }
    }

    pub async fn run(self, mut shutdown: watch::Receiver<bool>) {
        info!("billing runner started, polling every {:?}", self.poll_interval);

        loop {
            tokio::select! {
                _ = tokio::time::sleep(self.poll_interval) => {
                    if let Err(e) = self.process_renewals().await {
                        error!("billing runner process_renewals error: {}", e);
                    }
                }
                _ = shutdown.changed() => {
                    info!("billing runner shutting down");
                    break;
                }
            }
        }
    }

    async fn process_renewals(&self) -> Result<(), sqlx::Error> {
        let rows: Vec<RenewalRow> = sqlx::query_as(
            r#"
            SELECT 
                s.id, s.customer_id, s.price, s.name, s.provider_resource_id, s.cancel_at_period_end,
                s.is_trial, s.partner_id,
                p.category as product_category, p.provider, p.price as product_price,
                p.quota_gb as product_quota_gb,
                part.quota_gb_override as partner_quota_gb_override
            FROM services s
            JOIN products p ON s.product_id = p.id
            LEFT JOIN partners part ON s.partner_id = part.id
            WHERE s.status = 'running' AND s.next_due <= now()
            FOR UPDATE OF s SKIP LOCKED
            "#
        )
        .fetch_all(&self.pool)
        .await?;

        for row in rows {
            info!("processing renewal for service {}", row.id);
            let mut tx = self.pool.begin().await?;

            if row.cancel_at_period_end {
                info!("service {} is marked for cancellation at period end; terminating", row.id);
                sqlx::query("UPDATE services SET status = 'terminated' WHERE id = $1")
                    .bind(row.id)
                    .execute(&mut *tx)
                    .await?;

                if !row.provider_resource_id.is_empty() {
                    let job_id = Uuid::now_v7();
                    let payload = serde_json::json!({
                        "resource_id": row.provider_resource_id,
                    });

                    sqlx::query(
                        r#"
                        INSERT INTO jobs (id, task, target_api, customer_id, service_id, request_payload, scheduled_at)
                        VALUES ($1, 'delete', $2, $3, $4, $5, now())
                        "#
                    )
                    .bind(job_id)
                    .bind(&row.provider)
                    .bind(row.customer_id)
                    .bind(row.id)
                    .bind(payload)
                    .execute(&mut *tx)
                    .await?;
                }

                tx.commit().await?;
                info!("service {} terminated due to scheduled cancellation", row.id);
                continue;
            }

            let mut price = row.price;
            let mut sftp_upgrade = false;

            if row.is_trial {
                if row.product_category == "sftp" {
                    price = rust_decimal::Decimal::new(1200, 2);
                    sftp_upgrade = true;
                    info!("service {} (sftp trial) upgrading to paid plan", row.id);
                } else if price <= rust_decimal::Decimal::ZERO && row.product_price > rust_decimal::Decimal::ZERO {
                    price = row.product_price;
                    info!("service {} trial ended, upgrading to regular price €{}", row.id, price);
                } else {
                    info!("service {} trial period completed", row.id);
                }
            } else if row.product_category == "sftp" && price == rust_decimal::Decimal::new(0, 0) {
                // Legacy fallback: SFTP trial expired, renew for 12.00 EUR
                price = rust_decimal::Decimal::new(1200, 2);
                sftp_upgrade = true;
                info!("service {} (sftp trial) upgrading to paid plan", row.id);
            }

            if price > rust_decimal::Decimal::ZERO {
                let quantity: i32 = 1;
                let subtotal = price * rust_decimal::Decimal::from(quantity);
                // 19% VAT
                let tax_rate = rust_decimal::Decimal::new(19, 2); 
                let tax_amount = subtotal * tax_rate;
                let total = subtotal + tax_amount;

                let invoice_id = Uuid::now_v7();
                let invoice_number = format!("INV-{}", invoice_id.to_string()[..8].to_uppercase());
                let issue_date = Utc::now().naive_utc().date();
                let due_date = issue_date + chrono::Days::new(14); // 14 days due

                sqlx::query(
                    r#"
                    INSERT INTO invoices (id, invoice_number, customer_id, status, subtotal, tax_rate, tax_amount, total, issue_date, due_date)
                    VALUES ($1, $2, $3, 'pending', $4, $5, $6, $7, $8, $9)
                    "#
                )
                .bind(invoice_id)
                .bind(&invoice_number)
                .bind(row.customer_id)
                .bind(subtotal)
                .bind(tax_rate)
                .bind(tax_amount)
                .bind(total)
                .bind(issue_date)
                .bind(due_date)
                .execute(&mut *tx)
                .await?;

                let line_item_id = Uuid::now_v7();
                let desc = format!("Renewal for {}", row.name);
                sqlx::query(
                    r#"
                    INSERT INTO invoice_line_items (id, invoice_id, description, quantity, unit_price, total, service_id, sort_order)
                    VALUES ($1, $2, $3, $4, $5, $6, $7, 1)
                    "#
                )
                .bind(line_item_id)
                .bind(invoice_id)
                .bind(&desc)
                .bind(quantity)
                .bind(price)
                .bind(subtotal)
                .bind(row.id)
                .execute(&mut *tx)
                .await?;
            }

            // Add 30 days to next_due (monthly renewal)
            let next_due = Utc::now() + chrono::Duration::days(30);

            let target_quota_gb = row
                .partner_quota_gb_override
                .or(row.product_quota_gb)
                .unwrap_or(150);

            sqlx::query(
                r#"
                UPDATE services
                SET next_due = $1, price = $2, is_trial = false, trial_ends_at = NULL, quota_gb = CASE WHEN $3::boolean THEN $4 ELSE quota_gb END
                WHERE id = $5
                "#
            )
            .bind(next_due)
            .bind(price)
            .bind(sftp_upgrade)
            .bind(target_quota_gb)
            .bind(row.id)
            .execute(&mut *tx)
            .await?;

            if sftp_upgrade {
                // Insert a job to upgrade the quota in SFTPGo
                let job_id = Uuid::now_v7();
                let quota_size = (target_quota_gb as u64) * 1024 * 1024 * 1024;
                let payload = serde_json::json!({
                    "resource_id": row.provider_resource_id,
                    "provider_resource_id": row.provider_resource_id,
                    "quota_size": quota_size,
                });

                sqlx::query(
                    r#"
                    INSERT INTO jobs (id, task, target_api, customer_id, service_id, request_payload, scheduled_at)
                    VALUES ($1, 'modify', $2, $3, $4, $5, now())
                    "#
                )
                .bind(job_id)
                .bind(&row.provider)
                .bind(row.customer_id)
                .bind(row.id)
                .bind(payload)
                .execute(&mut *tx)
                .await?;
            }

            tx.commit().await?;
            info!("successfully renewed service {}", row.id);
        }

        Ok(())
    }
}
