use std::time::Duration;

use chrono::Utc;
use sqlx::PgPool;
use tokio::sync::watch;
use tracing::{error, info};
use uuid::Uuid;

#[derive(sqlx::FromRow)]
struct RenewalRow {
    id: Uuid,
    customer_id: Uuid,
    price: rust_decimal::Decimal,
    name: String,
    provider_resource_id: String,
    cancel_at_period_end: bool,
    product_category: String,
    provider: String,
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
                p.category as product_category, p.provider
            FROM services s
            JOIN products p ON s.product_id = p.id
            WHERE s.status = 'running' AND s.next_due <= now()
            FOR UPDATE SKIP LOCKED
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

            if row.product_category == "sftp" && price == rust_decimal::Decimal::new(0, 0) {
                // Trial expired, renew for 12.00 EUR
                price = rust_decimal::Decimal::new(1200, 2);
                sftp_upgrade = true;
                info!("service {} (sftp trial) upgrading to paid plan", row.id);
            }

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

            // Add 30 days to next_due (monthly renewal)
            let next_due = Utc::now() + chrono::Duration::days(30);

            sqlx::query(
                r#"
                UPDATE services
                SET next_due = $1, price = $2
                WHERE id = $3
                "#
            )
            .bind(next_due)
            .bind(price)
            .bind(row.id)
            .execute(&mut *tx)
            .await?;

            if sftp_upgrade {
                // Insert a job to upgrade the quota
                let job_id = Uuid::now_v7();
                // 150 GB in bytes
                let quota_size = 161_061_273_600_u64;
                let payload = serde_json::json!({
                    "quota_size": quota_size,
                    "provider_resource_id": row.provider_resource_id,
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
