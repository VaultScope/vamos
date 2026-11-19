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
    notifier: Option<Arc<dyn JobNotifier>>,
    encryption_key: Option<[u8; 32]>,
}

#[async_trait::async_trait]
pub trait JobNotifier: Send + Sync {
    async fn on_service_provisioned(&self, service_id: uuid::Uuid, response: &serde_json::Value);
}

#[async_trait::async_trait]
pub trait ConnectorRegistry: Send + Sync {
    async fn get(&self, provider: &str) -> Option<Arc<dyn ProvisioningProvider>>;
}

/// Extracts a provisioned service password (`raw_response.password`,
/// `raw_response.root_password`, top-level `password`, `root_password`, or
/// `raw_response.sftpgo_user.password`) from a connector provision response.
pub fn extract_provision_password(response: &serde_json::Value) -> Option<String> {
    let raw = response.get("raw_response");
    raw.and_then(|r| r.get("password"))
        .and_then(|v| v.as_str())
        .or_else(|| {
            raw.and_then(|r| r.get("root_password"))
                .and_then(|v| v.as_str())
        })
        .or_else(|| response.get("password").and_then(|v| v.as_str()))
        .or_else(|| response.get("root_password").and_then(|v| v.as_str()))
        .or_else(|| {
            raw.and_then(|r| r.get("sftpgo_user"))
                .and_then(|u| u.get("password"))
                .and_then(|v| v.as_str())
        })
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

/// Returns a copy of a provision response with any plaintext `password` or
/// `root_password` fields removed from both top-level and `raw_response`.
pub fn strip_provision_passwords(response: &serde_json::Value) -> serde_json::Value {
    let mut sanitized = response.clone();
    if let Some(obj) = sanitized.as_object_mut() {
        obj.remove("password");
        obj.remove("root_password");
        if let Some(raw_obj) = obj.get_mut("raw_response").and_then(|v| v.as_object_mut()) {
            raw_obj.remove("password");
            raw_obj.remove("root_password");
            if let Some(sftp_obj) = raw_obj
                .get_mut("sftpgo_user")
                .and_then(|v| v.as_object_mut())
            {
                sftp_obj.remove("password");
            }
        }
    }
    sanitized
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
            notifier: None,
            encryption_key: None,
        }
    }

    pub fn with_notifier(mut self, notifier: Arc<dyn JobNotifier>) -> Self {
        self.notifier = Some(notifier);
        self
    }

    pub fn with_encryption_key(mut self, key: [u8; 32]) -> Self {
        self.encryption_key = Some(key);
        self
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

    async fn claim_next_job(&self) -> Result<Option<Job>, sqlx::Error> {
        sqlx::query_as(
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
        .await
    }

    async fn poll_once(&self) -> Result<bool, sqlx::Error> {
        let Some(job) = self.claim_next_job().await? else {
            return Ok(false);
        };

        info!(job_id = %job.id, task = %job.task, "executing job");

        let pool = self.pool.clone();
        let connectors = self.connectors.clone();
        let notifier = self.notifier.clone();
        let encryption_key = self.encryption_key;

        tokio::spawn(async move {
            execute_and_persist_job(pool, connectors, notifier, encryption_key, job).await;
        });

        Ok(true)
    }

    /// Claims and executes a single queued job inline (useful for deterministic integration tests).
    pub async fn run_one_inline(&self) -> Result<bool, sqlx::Error> {
        let Some(job) = self.claim_next_job().await? else {
            return Ok(false);
        };

        execute_and_persist_job(
            self.pool.clone(),
            self.connectors.clone(),
            self.notifier.clone(),
            self.encryption_key,
            job,
        )
        .await;

        Ok(true)
    }
}

async fn execute_and_persist_job(
    pool: PgPool,
    connectors: Arc<dyn ConnectorRegistry>,
    notifier: Option<Arc<dyn JobNotifier>>,
    encryption_key: Option<[u8; 32]>,
    job: Job,
) {
    let result = tasks::execute(&job, &connectors).await;

    match result {
        Ok(response) => {
            let sanitized_response = if job.task == "provision" {
                strip_provision_passwords(&response)
            } else {
                response.clone()
            };

            let encrypted_password: Option<String> = if job.task == "provision" {
                match extract_provision_password(&response) {
                    Some(pw) => match encryption_key {
                        Some(ref key) => match vaultscope_common::crypto::encrypt_secret_string(key, &pw) {
                            Ok(enc) => Some(enc),
                            Err(e) => {
                                error!("Failed to encrypt service password for job {}: {}", job.id, e);
                                None
                            }
                        },
                        None => {
                            warn!("JobRunner missing encryption_key; skipping encrypted_password persistence for job {}", job.id);
                            None
                        }
                    },
                    None => None,
                }
            } else {
                None
            };

            let mut tx = match pool.begin().await {
                Ok(t) => t,
                Err(e) => {
                    error!("Failed to begin tx for job {}: {}", job.id, e);
                    return;
                }
            };

            if let Err(e) = sqlx::query(
                "UPDATE jobs SET status = 'completed', completed_at = now(), response_payload = $1 WHERE id = $2",
            )
            .bind(&sanitized_response)
            .bind(job.id)
            .execute(&mut *tx)
            .await
            {
                error!("Failed to update completed job {}: {}", job.id, e);
            }

            if job.task == "provision" {
                if let Some(service_id) = job.service_id {
                    let resource_id = sanitized_response
                        .get("resource_id")
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    let ip = sanitized_response
                        .get("ip")
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    let hostname = sanitized_response
                        .get("hostname")
                        .and_then(|v| v.as_str())
                        .unwrap_or("");

                    if let Err(e) = sqlx::query(
                        "UPDATE services SET status = 'running', config = $1, provider_resource_id = $2, ip = $3, hostname = $4, encrypted_password = COALESCE($5, encrypted_password) WHERE id = $6",
                    )
                    .bind(&sanitized_response)
                    .bind(resource_id)
                    .bind(if ip.is_empty() { None } else { Some(ip) })
                    .bind(if hostname.is_empty() { None } else { Some(hostname) })
                    .bind(encrypted_password.as_deref())
                    .bind(service_id)
                    .execute(&mut *tx)
                    .await
                    {
                        error!("Failed to update service after provisioning {}: {}", service_id, e);
                    }
                }
            }

            if let Err(e) = tx.commit().await {
                error!("Failed to commit job {} tx: {}", job.id, e);
                return;
            }

            if job.task == "provision" {
                if let Some(service_id) = job.service_id {
                    if let Some(ref n) = notifier {
                        n.on_service_provisioned(service_id, &sanitized_response).await;
                    }
                }
            }
        }
        Err(e) => {
            let error_msg = e.to_string();
            warn!(job_id = %job.id, error = %error_msg, "job failed");

            if job.attempts >= job.max_attempts {
                if let Err(db_err) = sqlx::query(
                    "UPDATE jobs SET status = 'failed', completed_at = now(), error = $1 WHERE id = $2",
                )
                .bind(&error_msg)
                .bind(job.id)
                .execute(&pool)
                .await
                {
                    error!("Failed to update failed job {}: {}", job.id, db_err);
                }
            } else {
                let backoff = Duration::from_secs(2u64.pow(job.attempts as u32));
                let retry_at = Utc::now() + backoff;
                if let Err(db_err) = sqlx::query(
                    "UPDATE jobs SET status = 'queued', scheduled_at = $1, error = $2 WHERE id = $3",
                )
                .bind(retry_at)
                .bind(&error_msg)
                .bind(job.id)
                .execute(&pool)
                .await
                {
                    error!("Failed to update retrying job {}: {}", job.id, db_err);
                }
            }
        }
    }
}
