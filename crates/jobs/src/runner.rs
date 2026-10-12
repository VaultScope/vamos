use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use sqlx::PgPool;
use tokio::sync::watch;
use tracing::{error, info, warn};

use vaultscope_connectors::ProvisioningProvider;
use vaultscope_db::models::Job;

use crate::tasks;

pub struct JobRunner {
    pool: PgPool,
    connectors: Arc<dyn ConnectorRegistry>,
    poll_interval: Duration,
}

#[async_trait::async_trait]
pub trait ConnectorRegistry: Send + Sync {
    async fn get(&self, provider: &str) -> Option<Arc<dyn ProvisioningProvider>>;
}

impl JobRunner {
    pub fn new(
        pool: PgPool,
        connectors: Arc<dyn ConnectorRegistry>,
        poll_interval: Duration,
    ) -> Self {
        Self {
            pool,
            connectors,
            poll_interval,
        }
    }

    pub async fn run(self, mut shutdown: watch::Receiver<bool>) {
        info!("job runner started, polling every {:?}", self.poll_interval);

        loop {
            tokio::select! {
                _ = tokio::time::sleep(self.poll_interval) => {
                    // Process as many jobs as possible before sleeping again
                    loop {
                        match self.poll_once().await {
                            Ok(true) => continue, // A job was picked up, try to pick up another one immediately
                            Ok(false) => break,   // No jobs left, break and sleep
                            Err(e) => {
                                error!("job runner poll error: {}", e);
                                break;
                            }
                        }
                    }
                }
                _ = shutdown.changed() => {
                    info!("job runner shutting down");
                    break;
                }
            }
        }
    }

    async fn poll_once(&self) -> Result<bool, sqlx::Error> {
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
            return Ok(false);
        };

        info!(job_id = %job.id, task = %job.task, "executing job");

        let pool = self.pool.clone();
        let connectors = self.connectors.clone();

        tokio::spawn(async move {
            let result = tasks::execute(&job, &connectors).await;

            match result {
                Ok(response) => {
                    let mut tx = pool.begin().await.unwrap_or_else(|e| panic!("Failed to begin tx: {}", e));
                    
                    if let Err(e) = sqlx::query(
                        "UPDATE jobs SET status = 'completed', completed_at = now(), response_payload = $1 WHERE id = $2"
                    )
                    .bind(&response)
                    .bind(job.id)
                    .execute(&mut *tx)
                    .await {
                        error!("Failed to update completed job {}: {}", job.id, e);
                    }

                    if job.task == "provision" {
                        if let Some(service_id) = job.service_id {
                            let resource_id = response.get("resource_id").and_then(|v| v.as_str()).unwrap_or("");
                            let ip = response.get("ip").and_then(|v| v.as_str()).unwrap_or("");
                            let hostname = response.get("hostname").and_then(|v| v.as_str()).unwrap_or("");
                            
                            if let Err(e) = sqlx::query(
                                "UPDATE services SET status = 'running', config = $1, provider_resource_id = $2, ip = $3, hostname = $4 WHERE id = $5"
                            )
                            .bind(&response)
                            .bind(resource_id)
                            .bind(if ip.is_empty() { None } else { Some(ip) })
                            .bind(if hostname.is_empty() { None } else { Some(hostname) })
                            .bind(service_id)
                            .execute(&mut *tx)
                            .await {
                                error!("Failed to update service after provisioning {}: {}", service_id, e);
                            }
                        }
                    }
                    
                    let _ = tx.commit().await;
                }
                Err(e) => {
                    let error_msg = e.to_string();
                    warn!(job_id = %job.id, error = %error_msg, "job failed");

                    if job.attempts >= job.max_attempts {
                        if let Err(db_err) = sqlx::query(
                            "UPDATE jobs SET status = 'failed', completed_at = now(), error = $1 WHERE id = $2"
                        )
                        .bind(&error_msg)
                        .bind(job.id)
                        .execute(&pool)
                        .await {
                            error!("Failed to update failed job {}: {}", job.id, db_err);
                        }
                    } else {
                        let backoff = Duration::from_secs(2u64.pow(job.attempts as u32));
                        let retry_at = Utc::now() + backoff;
                        if let Err(db_err) = sqlx::query(
                            "UPDATE jobs SET status = 'queued', scheduled_at = $1, error = $2 WHERE id = $3"
                        )
                        .bind(retry_at)
                        .bind(&error_msg)
                        .bind(job.id)
                        .execute(&pool)
                        .await {
                            error!("Failed to update retrying job {}: {}", job.id, db_err);
                        }
                    }
                }
            }
        });

        Ok(true)
    }
}
