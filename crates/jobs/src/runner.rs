use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use sqlx::PgPool;
use tokio::sync::watch;
use tracing::{error, info, warn};
use uuid::Uuid;

use vaultscope_connectors::ProvisioningProvider;
use vaultscope_db::models::Job;

use crate::tasks;

pub struct JobRunner {
    pool: PgPool,
    connectors: Arc<dyn ConnectorRegistry>,
    poll_interval: Duration,
}

pub trait ConnectorRegistry: Send + Sync {
    fn get(&self, connector_id: &Uuid) -> Option<Arc<dyn ProvisioningProvider>>;
}

impl JobRunner {
    pub fn new(pool: PgPool, connectors: Arc<dyn ConnectorRegistry>, poll_interval: Duration) -> Self {
        Self { pool, connectors, poll_interval }
    }

    pub async fn run(self, mut shutdown: watch::Receiver<bool>) {
        info!("job runner started, polling every {:?}", self.poll_interval);

        loop {
            tokio::select! {
                _ = tokio::time::sleep(self.poll_interval) => {
                    if let Err(e) = self.poll_once().await {
                        error!("job runner poll error: {}", e);
                    }
                }
                _ = shutdown.changed() => {
                    info!("job runner shutting down");
                    break;
                }
            }
        }
    }

    async fn poll_once(&self) -> Result<(), sqlx::Error> {
        let job: Option<Job> = sqlx::query_as(
            r#"
            UPDATE jobs
            SET status = 'in_progress', started_at = now(), attempts = attempts + 1
            WHERE id = (
                SELECT id FROM jobs
                WHERE status = 'queued' AND scheduled_at <= now()
                ORDER BY scheduled_at ASC
                FOR UPDATE SKIP LOCKED
                LIMIT 1
            )
            RETURNING *
            "#,
        )
        .fetch_optional(&self.pool)
        .await?;

        let Some(job) = job else {
            return Ok(());
        };

        info!(job_id = %job.id, task = %job.task, "executing job");

        let result = tasks::execute(&job, &self.connectors).await;

        match result {
            Ok(response) => {
                sqlx::query(
                    "UPDATE jobs SET status = 'completed', completed_at = now(), response_payload = $1 WHERE id = $2"
                )
                .bind(&response)
                .bind(job.id)
                .execute(&self.pool)
                .await?;
            }
            Err(e) => {
                let error_msg = e.to_string();
                warn!(job_id = %job.id, error = %error_msg, "job failed");

                if job.attempts >= job.max_attempts {
                    sqlx::query(
                        "UPDATE jobs SET status = 'failed', completed_at = now(), error = $1 WHERE id = $2"
                    )
                    .bind(&error_msg)
                    .bind(job.id)
                    .execute(&self.pool)
                    .await?;
                } else {
                    let backoff = Duration::from_secs(2u64.pow(job.attempts as u32));
                    let retry_at = Utc::now() + backoff;
                    sqlx::query(
                        "UPDATE jobs SET status = 'queued', scheduled_at = $1, error = $2 WHERE id = $3"
                    )
                    .bind(retry_at)
                    .bind(&error_msg)
                    .bind(job.id)
                    .execute(&self.pool)
                    .await?;
                }
            }
        }

        Ok(())
    }
}
