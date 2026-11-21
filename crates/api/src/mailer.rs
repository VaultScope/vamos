use chrono::Utc;
use uuid::Uuid;
use vaultscope_db::models::EmailDeliveryStatus;

use crate::state::AppState;

/// Strips CR, LF, and NUL bytes from email header values to prevent SMTP header injection.
pub fn sanitize_email_header(value: &str) -> String {
    value.replace(['\r', '\n', '\0'], "")
}

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
    let safe_subject = sanitize_email_header(subject);

    if to_email.contains(['\r', '\n', '\0']) || smtp_from.contains(['\r', '\n', '\0']) {
        tracing::warn!("Rejected email address containing header control characters");
        let _ = record_email_log(state, &sanitize_email_header(to_email), &safe_subject, template_id, EmailDeliveryStatus::Failed, Some("Invalid email address format")).await;
        return false;
    }

    let (from_addr, to_addr) = match (smtp_from.parse(), to_email.parse()) {
        (Ok(f), Ok(t)) => (f, t),
        _ => {
            tracing::warn!("Invalid email address format for dispatch: {} or {}", smtp_from, to_email);
            let _ = record_email_log(state, to_email, &safe_subject, template_id, EmailDeliveryStatus::Failed, Some("Invalid email address format")).await;
            return false;
        }
    };

    let email = match lettre::Message::builder()
        .from(from_addr)
        .to(to_addr)
        .subject(&safe_subject)
        .body(body.to_string())
    {
        Ok(m) => m,
        Err(e) => {
            tracing::warn!("Failed to build MIME email: {}", e);
            let _ = record_email_log(state, to_email, &safe_subject, template_id, EmailDeliveryStatus::Failed, Some(&e.to_string())).await;
            return false;
        }
    };

    let creds = lettre::transport::smtp::authentication::Credentials::new(smtp_user, smtp_pass);
    let mailer = match lettre::SmtpTransport::relay(&smtp_host) {
        Ok(builder) => builder.credentials(creds).build(),
        Err(e) => {
            tracing::warn!("Failed to resolve SMTP relay {}: {}", smtp_host, e);
            let _ = record_email_log(state, to_email, &safe_subject, template_id, EmailDeliveryStatus::Failed, Some(&e.to_string())).await;
            return false;
        }
    };

    let res = tokio::task::spawn_blocking(move || lettre::Transport::send(&mailer, &email)).await;
    match res {
        Ok(Ok(_)) => {
            tracing::info!("Dispatched email to {} with subject: {}", to_email, safe_subject);
            let _ = record_email_log(state, to_email, &safe_subject, template_id, EmailDeliveryStatus::Delivered, None).await;
            true
        }
        Ok(Err(e)) => {
            tracing::warn!("SMTP transport error sending to {}: {}", to_email, e);
            let _ = record_email_log(state, to_email, &safe_subject, template_id, EmailDeliveryStatus::Failed, Some(&e.to_string())).await;
            false
        }
        Err(e) => {
            tracing::warn!("SMTP dispatch task join error sending to {}: {}", to_email, e);
            let _ = record_email_log(state, to_email, &safe_subject, template_id, EmailDeliveryStatus::Failed, Some("Email dispatch task failed")).await;
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
            "SELECT subject, assignee_id, customer_id FROM tickets WHERE id = $1"
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

pub fn is_dedicated_product(provider: &str, category: &str) -> bool {
    let prov = provider.trim().to_ascii_lowercase();
    let cat = category.trim().to_ascii_lowercase();
    prov == "reliablesite"
        || prov == "vaultscope_dedicated"
        || cat == "dedicated"
        || cat == "ds"
        || cat == "bare-metal"
}

pub fn build_service_welcome_email(
    customer_name: &str,
    service_name: &str,
    service_id: Uuid,
    hostname: &str,
    quota: i32,
    is_trial: bool,
    price: rust_decimal::Decimal,
    provider: &str,
    category: &str,
    response_payload: &serde_json::Value,
) -> (String, String) {
    let is_dedicated = is_dedicated_product(provider, category);
    let is_sftp = response_payload.get("sftpgo_user").is_some()
        || response_payload.get("raw_response").and_then(|r| r.get("sftpgo_user")).is_some()
        || service_name.to_lowercase().contains("sftp")
        || service_name.to_lowercase().contains("storage");

    if is_sftp {
        let email_subject = format!("[VaultScope] Your service is ready: {}", service_name);
        let username = response_payload
            .get("resource_id")
            .and_then(|v| v.as_str())
            .or_else(|| {
                response_payload
                    .get("raw_response")
                    .and_then(|r| r.get("sftpgo_user"))
                    .and_then(|u| u.get("username"))
                    .and_then(|v| v.as_str())
            })
            .unwrap_or("vs_user");
        let password = "[Shown in Encrypted Customer Dashboard]";
        let billing_line = if is_trial {
            format!("Subscription: Free Trial ({} GB - auto-upgrades to full plan on renewal)", quota)
        } else {
            format!("Subscription: Regular Plan ({} GB) - €{}/mo", quota, price)
        };
        let details_block = format!(
            "SERVICE DETAILS & ACCESS CREDENTIALS\n----------------------------------------\nProduct: {}\n{}\nStorage Quota: {} GB\n\nProtocol: SFTP (SSH File Transfer Protocol) / FTPS / WebDAV\nHost: {}\nPort: 2022 (SFTP)\nUsername: {}\nPassword: {}\n----------------------------------------\n\nManage your service in Dashboard: https://vaultscope.de/dashboard/storage",
            service_name, billing_line, quota, hostname, username, password
        );
        let body = format!(
            "Hello {},\n\nThank you for choosing VaultScope!\n\nYour service \"{}\" has been successfully provisioned and is now active.\n\n{}\n\nYou can access your service, manage configuration, or view billing anytime in your VaultScope Portal:\nhttps://vaultscope.de/dashboard\n\nIf you have any questions or require support, our team is available via your dashboard:\nhttps://vaultscope.de/dashboard/support\n\nBest regards,\nThe VaultScope Team\nhttps://vaultscope.de",
            customer_name, service_name, details_block
        );
        return (email_subject, body);
    }

    let ip_opt = response_payload
        .get("ip")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty());

    let dashboard_path = if is_dedicated { "ds" } else { "vps" };
    let billing_line = if is_trial {
        "Subscription: Free Trial".to_string()
    } else {
        format!("Subscription: Regular Plan - €{}", price)
    };

    if is_dedicated && ip_opt.is_none() {
        let email_subject = format!(
            "[VaultScope] Dedicated Server Provisioning Started: {}",
            service_name
        );
        let body = format!(
            "Hello {},\n\nThank you for your payment! Your dedicated server order \"{}\" has been submitted to hardware provisioning.\n\nPROVISIONING TIMELINE\n----------------------------------------\nProduct: {}\n{}\nHostname: {}\nProvisioning Time: Up to 7 business days (regularly after payment has succeeded)\n----------------------------------------\n\nDedicated server provisioning regularly takes up to 7 business days after payment succeeds. Once hardware setup, OS installation, and network allocation are complete, your IP address, credentials, and IPMI/KVM console will appear in your dashboard:\nhttps://vaultscope.de/dashboard/ds/{}\n\nIf you have any questions or require support, our team is available via your dashboard:\nhttps://vaultscope.de/dashboard/support\n\nBest regards,\nThe VaultScope Team\nhttps://vaultscope.de",
            customer_name, service_name, service_name, billing_line, hostname, service_id
        );
        return (email_subject, body);
    }

    let ip = ip_opt.unwrap_or("Assigned in Dashboard");
    let root_pass = "[Shown in Encrypted Customer Dashboard]";
    let email_subject = format!("[VaultScope] Your service is ready: {}", service_name);
    let details_block = format!(
        "SERVICE DETAILS & ACCESS CREDENTIALS\n----------------------------------------\nProduct: {}\n{}\n\nIP Address: {}\nHostname: {}\nSSH Port: 22\nUsername: root\nRoot Password: {}\n----------------------------------------\n\nManage your server in Dashboard: https://vaultscope.de/dashboard/{}/{}",
        service_name, billing_line, ip, hostname, root_pass, dashboard_path, service_id
    );
    let body = format!(
        "Hello {},\n\nThank you for choosing VaultScope!\n\nYour service \"{}\" has been successfully provisioned and is now active.\n\n{}\n\nYou can access your service, manage configuration, or view billing anytime in your VaultScope Portal:\nhttps://vaultscope.de/dashboard\n\nIf you have any questions or require support, our team is available via your dashboard:\nhttps://vaultscope.de/dashboard/support\n\nBest regards,\nThe VaultScope Team\nhttps://vaultscope.de",
        customer_name, service_name, details_block
    );
    (email_subject, body)
}

/// Spawns a background task to notify the customer right after payment & order placement.
pub fn spawn_order_confirmation_notification(
    state: AppState,
    customer_id: Uuid,
    service_name: String,
    provider: String,
    category: String,
    service_id: Uuid,
    paid_total: Option<rust_decimal::Decimal>,
) {
    tokio::spawn(async move {
        let customer_opt: Option<(String, String)> =
            sqlx::query_as("SELECT name, email FROM customers WHERE id = $1")
                .bind(customer_id)
                .fetch_optional(&state.db)
                .await
                .unwrap_or(None);

        let Some((name, email)) = customer_opt else {
            return;
        };

        let is_dedicated = is_dedicated_product(&provider, &category);
        let paid_line = paid_total
            .map(|t| format!("Amount Paid: €{:.2}\n", t))
            .unwrap_or_default();

        let (subject, body) = if is_dedicated {
            (
                format!("[VaultScope] Payment Received — Dedicated Server Order Confirmed: {}", service_name),
                format!(
                    "Hello {},\n\nThank you for your order! Your payment has succeeded and your dedicated server \"{}\" is now queued for hardware provisioning.\n\nORDER SUMMARY\n----------------------------------------\nServer: {}\n{}Provisioning Time: Regularly up to 7 business days after payment succeeds\n----------------------------------------\n\nPlease note that dedicated server provisioning regularly takes up to 7 business days after payment succeeds. You will receive updates by email once hardware setup and network configuration are ready, and you can track your server status anytime in your dashboard:\n{}/dashboard/ds/{}\n\nBest regards,\nThe VaultScope Team",
                    name,
                    service_name,
                    service_name,
                    paid_line,
                    state.config.storefront_url.trim_end_matches('/'),
                    service_id
                ),
            )
        } else {
            (
                format!("[VaultScope] Order Confirmed: {}", service_name),
                format!(
                    "Hello {},\n\nThank you for your order! Your payment has succeeded and provisioning has started for \"{}\".\n\n{}You can view status and credentials in your dashboard:\n{}/dashboard\n\nBest regards,\nThe VaultScope Team",
                    name,
                    service_name,
                    paid_line,
                    state.config.storefront_url.trim_end_matches('/')
                ),
            )
        };

        send_billing_email_with_cc(&state, customer_id, &email, &subject, &body).await;
    });
}

/// Spawns a background task to notify the customer that their ordered service is provisioned and ready.
pub fn spawn_service_welcome_notification(
    state: AppState,
    service_id: Uuid,
    response_payload: serde_json::Value,
) {
    tokio::spawn(async move {
        let service_res = sqlx::query_as::<_, (Uuid, String, Option<String>, Option<i32>, bool, rust_decimal::Decimal, String, String)>(
            "SELECT s.customer_id, s.name, s.hostname, s.quota_gb, s.is_trial, s.price, p.provider, p.category FROM services s JOIN products p ON p.id = s.product_id WHERE s.id = $1"
        )
        .bind(service_id)
        .fetch_optional(&state.db)
        .await;

        let (customer_id, service_name, hostname_opt, quota_gb_opt, is_trial, price, provider, category) = match service_res {
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
        let (email_subject, body) = build_service_welcome_email(
            &customer_name,
            &service_name,
            service_id,
            &hostname,
            quota,
            is_trial,
            price,
            &provider,
            &category,
            &response_payload,
        );

        send_email(&state, &customer_email, &email_subject, &body, None).await;
    });
}

/// Dispatches a billing email to the customer's primary email and CCs verified secondary billing emails.
pub async fn send_billing_email_with_cc(
    state: &AppState,
    customer_id: Uuid,
    primary_email: &str,
    subject: &str,
    body: &str,
) {
    send_email(state, primary_email, subject, body, None).await;

    let cc_emails: Vec<String> = sqlx::query_scalar(
        "SELECT email FROM customer_billing_emails WHERE customer_id = $1 AND verified = true",
    )
    .bind(customer_id)
    .fetch_all(&state.db)
    .await
    .unwrap_or_default();

    for cc_email in cc_emails {
        if !cc_email.eq_ignore_ascii_case(primary_email) {
            send_email(state, &cc_email, subject, body, None).await;
        }
    }
}

/// Sends a reminder 7 days before a service renewal / upcoming payment is required.
pub fn spawn_upcoming_payment_7d_notification(
    state: AppState,
    _service_id: Uuid,
    customer_id: Uuid,
    service_name: String,
    net_price: rust_decimal::Decimal,
    due_date: String,
) {
    tokio::spawn(async move {
        let customer_opt: Option<(String, String)> =
            sqlx::query_as("SELECT name, email FROM customers WHERE id = $1")
                .bind(customer_id)
                .fetch_optional(&state.db)
                .await
                .unwrap_or(None);

        let Some((name, email)) = customer_opt else {
            return;
        };

        let gross_amount = net_price * rust_decimal::Decimal::new(119, 2);
        let billing_url = format!(
            "{}/dashboard/profile/billing",
            state.config.storefront_url.trim_end_matches('/')
        );
        let subject = format!("[VaultScope] Upcoming renewal in 7 days: {}", service_name);
        let body = format!(
            "Hello {},\n\nThis is a reminder that your next payment / renewal for \"{}\" is scheduled in 7 days (on {}).\n\nEstimated Amount (incl. 19% VAT): €{:.2} (€{:.2} net)\n\nA renewal invoice will be issued on your renewal date. You can review your subscriptions, invoices, or cancel at period end anytime in your Billing Dashboard:\n{}\n\nBest regards,\nThe VaultScope Billing Team",
            name, service_name, due_date, gross_amount, net_price, billing_url
        );

        send_billing_email_with_cc(&state, customer_id, &email, &subject, &body).await;
    });
}

/// Sends a notification when a new invoice is generated.
pub fn spawn_new_invoice_notification(state: AppState, invoice_id: Uuid) {
    tokio::spawn(async move {
        let row: Option<(String, Option<Uuid>, rust_decimal::Decimal, chrono::NaiveDate)> =
            sqlx::query_as(
                "SELECT invoice_number, customer_id, total, due_date FROM invoices WHERE id = $1",
            )
            .bind(invoice_id)
            .fetch_optional(&state.db)
            .await
            .unwrap_or(None);

        let Some((invoice_number, Some(customer_id), total, due_date)) = row else {
            return;
        };

        let customer_opt: Option<(String, String)> =
            sqlx::query_as("SELECT name, email FROM customers WHERE id = $1")
                .bind(customer_id)
                .fetch_optional(&state.db)
                .await
                .unwrap_or(None);

        let Some((name, email)) = customer_opt else {
            return;
        };

        let billing_url = format!(
            "{}/dashboard/profile/billing",
            state.config.storefront_url.trim_end_matches('/')
        );
        let subject = format!("[VaultScope] New Invoice {} (€{:.2})", invoice_number, total);
        let body = format!(
            "Hello {},\n\nA new invoice has been issued for your VaultScope account.\n\nInvoice Number: {}\nAmount Due: €{:.2}\nDue Date: {}\n\nPlease pay this invoice online via Stripe (Card, SEPA, Apple Pay, Google Pay, PayPal, Klarna) in your Billing Dashboard:\n{}\n\nBest regards,\nThe VaultScope Billing Team",
            name, invoice_number, total, due_date, billing_url
        );

        send_billing_email_with_cc(&state, customer_id, &email, &subject, &body).await;
    });
}

/// Sends a payment warning email for a pending invoice approaching its due date or an overdue invoice.
pub fn spawn_payment_warning_notification(state: AppState, invoice_id: Uuid, is_overdue: bool) {
    tokio::spawn(async move {
        let row: Option<(String, Option<Uuid>, rust_decimal::Decimal, chrono::NaiveDate)> =
            sqlx::query_as(
                "SELECT invoice_number, customer_id, total, due_date FROM invoices WHERE id = $1",
            )
            .bind(invoice_id)
            .fetch_optional(&state.db)
            .await
            .unwrap_or(None);

        let Some((invoice_number, Some(customer_id), total, due_date)) = row else {
            return;
        };

        let customer_opt: Option<(String, String)> =
            sqlx::query_as("SELECT name, email FROM customers WHERE id = $1")
                .bind(customer_id)
                .fetch_optional(&state.db)
                .await
                .unwrap_or(None);

        let Some((name, email)) = customer_opt else {
            return;
        };

        let billing_url = format!(
            "{}/dashboard/profile/billing",
            state.config.storefront_url.trim_end_matches('/')
        );
        let subject = if is_overdue {
            format!(
                "[VaultScope] Payment Overdue Warning: Invoice {} (€{:.2})",
                invoice_number, total
            )
        } else {
            format!(
                "[VaultScope] Payment Reminder: Invoice {} due on {}",
                invoice_number, due_date
            )
        };
        let body = if is_overdue {
            format!(
                "Hello {},\n\nYour VaultScope invoice {} for €{:.2} is now overdue (due date was {}).\n\nPlease settle this invoice as soon as possible to avoid service interruption:\n{}\n\nBest regards,\nThe VaultScope Billing Team",
                name, invoice_number, total, due_date, billing_url
            )
        } else {
            format!(
                "Hello {},\n\nThis is a friendly reminder that your VaultScope invoice {} for €{:.2} is due on {}.\n\nYou can pay this invoice directly via Stripe in your Billing Dashboard:\n{}\n\nBest regards,\nThe VaultScope Billing Team",
                name, invoice_number, total, due_date, billing_url
            )
        };

        send_billing_email_with_cc(&state, customer_id, &email, &subject, &body).await;
    });
}

/// Sends a confirmation email after an invoice is paid.
pub fn spawn_invoice_paid_notification(state: AppState, invoice_id: Uuid) {
    tokio::spawn(async move {
        let row: Option<(String, Option<Uuid>, rust_decimal::Decimal)> = sqlx::query_as(
            "SELECT invoice_number, customer_id, total FROM invoices WHERE id = $1",
        )
        .bind(invoice_id)
        .fetch_optional(&state.db)
        .await
        .unwrap_or(None);

        let Some((invoice_number, Some(customer_id), total)) = row else {
            return;
        };

        let customer_opt: Option<(String, String)> =
            sqlx::query_as("SELECT name, email FROM customers WHERE id = $1")
                .bind(customer_id)
                .fetch_optional(&state.db)
                .await
                .unwrap_or(None);

        let Some((name, email)) = customer_opt else {
            return;
        };

        let has_dedicated: bool = sqlx::query_scalar(
            r#"SELECT EXISTS(
                SELECT 1
                FROM invoice_line_items ili
                JOIN services s ON s.id = ili.service_id
                JOIN products p ON p.id = s.product_id
                WHERE ili.invoice_id = $1
                  AND (p.provider IN ('reliablesite', 'vaultscope_dedicated')
                       OR LOWER(p.category) IN ('dedicated', 'ds', 'bare-metal'))
            )"#,
        )
        .bind(invoice_id)
        .fetch_one(&state.db)
        .await
        .unwrap_or(false);

        let dedicated_note = if has_dedicated {
            "\nPlease note: Dedicated server provisioning regularly takes up to 7 business days after payment succeeds. We will send an email as soon as your server setup and credentials are ready.\n"
        } else {
            ""
        };

        let billing_url = format!(
            "{}/dashboard/profile/billing",
            state.config.storefront_url.trim_end_matches('/')
        );
        let subject = format!(
            "[VaultScope] Payment Confirmation for Invoice {}",
            invoice_number
        );
        let body = format!(
            "Hello {},\n\nThank you! We have received your payment of €{:.2} for invoice {}.\n{}You can download the PDF receipt anytime from your Billing Dashboard:\n{}\n\nBest regards,\nThe VaultScope Billing Team",
            name, total, invoice_number, dedicated_note, billing_url
        );

        send_billing_email_with_cc(&state, customer_id, &email, &subject, &body).await;
    });
}

/// Sends an email notification when an Admin updates a service's order/provisioning state.
pub fn spawn_order_state_updated_notification(
    state: AppState,
    service_id: Uuid,
    order_state: String,
    order_state_note: Option<String>,
) {
    tokio::spawn(async move {
        let row: Option<(Uuid, String, String, String)> = sqlx::query_as(
            "SELECT s.customer_id, s.name, p.provider, p.category FROM services s JOIN products p ON p.id = s.product_id WHERE s.id = $1",
        )
        .bind(service_id)
        .fetch_optional(&state.db)
        .await
        .unwrap_or(None);

        let Some((customer_id, service_name, provider, category)) = row else {
            return;
        };

        let customer_opt: Option<(String, String)> =
            sqlx::query_as("SELECT name, email FROM customers WHERE id = $1")
                .bind(customer_id)
                .fetch_optional(&state.db)
                .await
                .unwrap_or(None);

        let Some((name, email)) = customer_opt else {
            return;
        };

        let is_dedicated = is_dedicated_product(&provider, &category);
        let state_label = match order_state.as_str() {
            "payment_received" => "Payment Received / Order Queued",
            "provisioning" => "Provisioning in Progress",
            "hardware_setup" => "Hardware Setup & OS Installation",
            "active" => "Active / Ready",
            "action_required" => "Action Required",
            "cancelled" => "Cancelled",
            other => other,
        };
        let note_line = order_state_note
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|n| format!("Note: {}\n", n))
            .unwrap_or_default();
        let timeline_line = if is_dedicated && order_state != "active" && order_state != "cancelled" {
            "\nPlease note: Dedicated server provisioning regularly takes up to 7 business days after payment succeeds.\n"
        } else {
            ""
        };
        let base_url = state.config.storefront_url.trim_end_matches('/');
        let target_url = if is_dedicated {
            format!("{}/dashboard/ds/{}", base_url, service_id)
        } else {
            format!("{}/dashboard/orders", base_url)
        };
        let subject = format!("[VaultScope] Order Status Update ({}): {}", state_label, service_name);
        let body = format!(
            "Hello {},\n\nThere is an update on your order \"{}\":\n\nNew Order State: {}\n{}{}\nView your order and service status anytime in your dashboard:\n{}\n\nBest regards,\nThe VaultScope Team",
            name, service_name, state_label, note_line, timeline_line, target_url
        );

        send_billing_email_with_cc(&state, customer_id, &email, &subject, &body).await;
    });
}
