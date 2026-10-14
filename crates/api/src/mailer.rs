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

/// Spawns a background task to notify the customer that their ordered service is provisioned and ready.
pub fn spawn_service_welcome_notification(
    state: AppState,
    service_id: Uuid,
    response_payload: serde_json::Value,
) {
    tokio::spawn(async move {
        // Query service and customer info
        let service_res = sqlx::query_as::<_, (Uuid, String, Option<String>, Option<i32>, bool, rust_decimal::Decimal)>(
            "SELECT customer_id, name, hostname, quota_gb, is_trial, price FROM services WHERE id = $1"
        )
        .bind(service_id)
        .fetch_optional(&state.db)
        .await;

        let (customer_id, service_name, hostname_opt, quota_gb_opt, is_trial, price) = match service_res {
            Ok(Some(row)) => row,
            Ok(None) => {
                tracing::warn!("Service {} not found for welcome notification", service_id);
                return;
            }
            Err(e) => {
                tracing::warn!("Failed to fetch service for welcome notification {}: {}", service_id, e);
                return;
            }
        };

        let customer_opt: Option<(String, String)> = sqlx::query_as(
            "SELECT name, email FROM customers WHERE id = $1"
        )
        .bind(customer_id)
        .fetch_optional(&state.db)
        .await
        .unwrap_or(None);

        let (customer_name, customer_email) = match customer_opt {
            Some(row) => row,
            None => {
                tracing::warn!("Customer {} not found for service welcome notification", customer_id);
                return;
            }
        };

        let hostname = hostname_opt.unwrap_or_else(|| "sftp.vaultscope.de".to_string());
        let quota = quota_gb_opt.unwrap_or(25);

        let is_sftp = response_payload.get("sftpgo_user").is_some()
            || response_payload.get("raw_response").and_then(|r| r.get("sftpgo_user")).is_some()
            || service_name.to_lowercase().contains("sftp")
            || service_name.to_lowercase().contains("storage");

        let email_subject = format!("[VaultScope] Your service is ready: {}", service_name);

        let details_block = if is_sftp {
            let username = response_payload.get("resource_id")
                .and_then(|v| v.as_str())
                .or_else(|| response_payload.get("raw_response").and_then(|r| r.get("sftpgo_user")).and_then(|u| u.get("username")).and_then(|v| v.as_str()))
                .unwrap_or("vs_user");

            let password = response_payload.get("raw_response")
                .and_then(|r| r.get("password"))
                .and_then(|v| v.as_str())
                .or_else(|| response_payload.get("password").and_then(|v| v.as_str()))
                .unwrap_or("[Shown in Dashboard]");

            let billing_line = if is_trial {
                format!("Subscription: Free Trial ({} GB - auto-upgrades to full plan on renewal)", quota)
            } else {
                format!("Subscription: Regular Plan ({} GB) - €{}/mo", quota, price)
            };

            format!(
                "SERVICE DETAILS & ACCESS CREDENTIALS\n----------------------------------------\nProduct: {}\n{}\nStorage Quota: {} GB\n\nProtocol: SFTP (SSH File Transfer Protocol) / FTPS / WebDAV\nHost: {}\nPort: 2022 (SFTP)\nUsername: {}\nPassword: {}\n----------------------------------------\n\nManage your service in Dashboard: https://vaultscope.de/dashboard/storage",
                service_name, billing_line, quota, hostname, username, password
            )
        } else {
            let ip = response_payload.get("ip")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .unwrap_or("Assigned in Dashboard");

            let root_pass = response_payload.get("raw_response")
                .and_then(|r| r.get("root_password"))
                .and_then(|v| v.as_str())
                .unwrap_or("[Available in Dashboard / Configured SSH Key]");

            let billing_line = if is_trial {
                "Subscription: Free Trial".to_string()
            } else {
                format!("Subscription: Regular Plan - €{}/mo", price)
            };

            format!(
                "SERVICE DETAILS & ACCESS CREDENTIALS\n----------------------------------------\nProduct: {}\n{}\n\nIP Address: {}\nHostname: {}\nSSH Port: 22\nUsername: root\nRoot Password: {}\n----------------------------------------\n\nManage your server in Dashboard: https://vaultscope.de/dashboard/vps/{}",
                service_name, billing_line, ip, hostname, root_pass, service_id
            )
        };

        let body = format!(
            "Hello {},\n\nThank you for choosing VaultScope!\n\nYour service \"{}\" has been successfully provisioned and is now active.\n\n{}\n\nYou can access your service, manage configuration, or view billing anytime in your VaultScope Portal:\nhttps://vaultscope.de/dashboard\n\nIf you have any questions or require support, our team is available 24/7 in your dashboard:\nhttps://vaultscope.de/dashboard/support\n\nBest regards,\nThe VaultScope Team\nhttps://vaultscope.de",
            customer_name, service_name, details_block
        );

        send_email(&state, &customer_email, &email_subject, &body, None).await;
    });
}
