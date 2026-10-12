use chrono::Utc;
use uuid::Uuid;
use vaultscope_db::models::EmailDeliveryStatus;

use crate::state::AppState;

/// Asynchronously dispatches an email via SMTP and writes an audit entry into `email_log`.
pub async fn send_email(
    state: &AppState,
    to_email: &str,
    subject: &str,
    body: &str,
    template_id: Option<Uuid>,
) -> bool {
    let smtp_host = state.config.smtp_host.clone().unwrap_or_else(|| "mail.vaultscope.de".to_string());
    let smtp_user = state.config.smtp_username.clone().unwrap_or_else(|| "support@vaultscope.de".to_string());
    let smtp_pass = state.config.smtp_password.clone().unwrap_or_default();
    let smtp_from = state.config.smtp_from.clone().unwrap_or_else(|| "support@vaultscope.de".to_string());

    let (from_addr, to_addr) = match (smtp_from.parse(), to_email.parse()) {
        (Ok(f), Ok(t)) => (f, t),
        _ => {
            tracing::warn!("Invalid email address format for dispatch: {} or {}", smtp_from, to_email);
            let _ = record_email_log(state, to_email, subject, template_id, EmailDeliveryStatus::Failed, Some("Invalid email address format")).await;
            return false;
        }
    };

    let email = match lettre::Message::builder()
        .from(from_addr)
        .to(to_addr)
        .subject(subject)
        .body(body.to_string())
    {
        Ok(m) => m,
        Err(e) => {
            tracing::warn!("Failed to build MIME email: {}", e);
            let _ = record_email_log(state, to_email, subject, template_id, EmailDeliveryStatus::Failed, Some(&e.to_string())).await;
            return false;
        }
    };

    let creds = lettre::transport::smtp::authentication::Credentials::new(smtp_user, smtp_pass);
    let mailer = match lettre::SmtpTransport::relay(&smtp_host) {
        Ok(builder) => builder.credentials(creds).build(),
        Err(e) => {
            tracing::warn!("Failed to resolve SMTP relay {}: {}", smtp_host, e);
            let _ = record_email_log(state, to_email, subject, template_id, EmailDeliveryStatus::Failed, Some(&e.to_string())).await;
            return false;
        }
    };

    let res = lettre::Transport::send(&mailer, &email);
    match res {
        Ok(_) => {
            tracing::info!("Dispatched email to {} with subject: {}", to_email, subject);
            let _ = record_email_log(state, to_email, subject, template_id, EmailDeliveryStatus::Delivered, None).await;
            true
        }
        Err(e) => {
            tracing::warn!("SMTP transport error sending to {}: {}", to_email, e);
            let _ = record_email_log(state, to_email, subject, template_id, EmailDeliveryStatus::Failed, Some(&e.to_string())).await;
            false
        }
    }
}

async fn record_email_log(
    state: &AppState,
    recipient: &str,
    subject: &str,
    template_id: Option<Uuid>,
    status: EmailDeliveryStatus,
    error: Option<&str>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"INSERT INTO email_log (id, recipient, subject, template_id, status, error, sent_at)
           VALUES ($1, $2, $3, $4, $5, $6, $7)"#,
    )
    .bind(Uuid::now_v7())
    .bind(recipient)
    .bind(subject)
    .bind(template_id)
    .bind(status)
    .bind(error)
    .bind(Utc::now())
    .execute(&state.db)
    .await?;

    Ok(())
}

/// Spawns a background task to notify the customer of a staff reply to their ticket.
pub fn spawn_staff_reply_notification(
    state: AppState,
    ticket_id: Uuid,
    message_content: String,
) {
    tokio::spawn(async move {
        // Fetch ticket and customer
        let ticket_opt: Option<(String, Uuid)> = match sqlx::query_as(
            "SELECT subject, customer_id FROM tickets WHERE id = $1"
        )
        .bind(ticket_id)
        .fetch_optional(&state.db)
        .await
        {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!("Failed to query ticket for notification {}: {}", ticket_id, e);
                return;
            }
        };

        if let Some((subject, customer_id)) = ticket_opt {
            let customer_opt: Option<(String, String)> = sqlx::query_as(
                "SELECT name, email FROM customers WHERE id = $1"
            )
            .bind(customer_id)
            .fetch_optional(&state.db)
            .await
            .unwrap_or(None);

            if let Some((name, email)) = customer_opt {
                let email_subject = format!("[Ticket #{}] Support Reply: {}", &ticket_id.to_string()[..8], subject);
                let body = format!(
                    "Hello {},\n\nOur support team has posted a reply to your ticket \"{}\":\n\n----------------------------------------\n{}\n----------------------------------------\n\nYou can reply directly by logging into your VaultScope Customer Portal:\nhttps://vaultscope.de/dashboard/support/{}\n\nBest regards,\nVaultScope Support Team",
                    name, subject, message_content, ticket_id
                );

                send_email(&state, &email, &email_subject, &body, None).await;
            }
        }
    });
}

/// Spawns a background task to notify support staff of a customer reply.
pub fn spawn_customer_reply_notification(
    state: AppState,
    ticket_id: Uuid,
    message_content: String,
) {
    tokio::spawn(async move {
        let ticket_opt: Option<(String, Option<Uuid>, Uuid)> = match sqlx::query_as(
            "SELECT subject, assigned_to, customer_id FROM tickets WHERE id = $1"
        )
        .bind(ticket_id)
        .fetch_optional(&state.db)
        .await
        {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!("Failed to query ticket {}: {}", ticket_id, e);
                return;
            }
        };

        if let Some((subject, assigned_to, customer_id)) = ticket_opt {
            let customer_name: String = sqlx::query_scalar(
                "SELECT name FROM customers WHERE id = $1"
            )
            .bind(customer_id)
            .fetch_optional(&state.db)
            .await
            .unwrap_or(None)
            .unwrap_or_else(|| "Customer".to_string());

            let mut staff_email = "support@vaultscope.de".to_string();
            let mut staff_name = "Support Team".to_string();

            if let Some(staff_id) = assigned_to {
                let staff_res: Result<Option<(String, String)>, sqlx::Error> = sqlx::query_as(
                    "SELECT username, email FROM staff WHERE id = $1"
                )
                .bind(staff_id)
                .fetch_optional(&state.db)
                .await;

                if let Ok(Some((s_name, s_email))) = staff_res {
                    staff_name = s_name;
                    staff_email = s_email;
                }
            }

            let email_subject = format!("[Action Required] New Customer Reply on Ticket #{}: {}", &ticket_id.to_string()[..8], subject);
            let body = format!(
                "Hello {},\n\n{} has added a new reply to Ticket \"{}\" (ID: {}):\n\n----------------------------------------\n{}\n----------------------------------------\n\nOpen CAMOS to respond:\nhttps://admin.vaultscope.de/tickets/{}\n\nVaultScope Internal Notification",
                staff_name, customer_name, subject, ticket_id, message_content, ticket_id
            );

            send_email(&state, &staff_email, &email_subject, &body, None).await;
        }
    });
}
