use chrono::Utc;
use uuid::Uuid;
use vaultscope_db::models::ActorType;

/// Asynchronously inserts an audit entry into `activity_log`.
pub async fn record(
    pool: &sqlx::PgPool,
    actor_id: Option<Uuid>,
    actor_type: ActorType,
    actor_name: &str,
    category: &str,
    action: &str,
    target: &str,
    detail: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"INSERT INTO activity_log (id, actor_id, actor_type, actor_name, category, action, target, detail, created_at)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)"#,
    )
    .bind(Uuid::now_v7())
    .bind(actor_id)
    .bind(actor_type)
    .bind(actor_name)
    .bind(category)
    .bind(action)
    .bind(target)
    .bind(detail)
    .bind(Utc::now())
    .execute(pool)
    .await?;

    Ok(())
}

/// Spawns a background task to insert an audit entry into `activity_log` without blocking the request handler.
pub fn spawn_record(
    pool: sqlx::PgPool,
    actor_id: Option<Uuid>,
    actor_type: ActorType,
    actor_name: String,
    category: String,
    action: String,
    target: String,
    detail: String,
) {
    tokio::spawn(async move {
        if let Err(e) = record(
            &pool,
            actor_id,
            actor_type,
            &actor_name,
            &category,
            &action,
            &target,
            &detail,
        )
        .await
        {
            tracing::warn!("Failed to write activity_log: {}", e);
        }
    });
}
