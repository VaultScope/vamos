use std::convert::Infallible;
use axum::extract::{DefaultBodyLimit, Multipart, Path, Query, State};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::routing::get;
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use tokio_stream::StreamExt;
use tokio_stream::wrappers::BroadcastStream;
use uuid::Uuid;

use crate::auth::AuthAdmin;
use crate::error::ApiError;
use crate::state::AppState;
use crate::validation::{
    validate_required_string, validate_string_length, MAX_SHORT_STRING_LENGTH, MAX_STRING_LENGTH,
};
use vaultscope_db::models::{
    ActorType, Ticket, TicketCategory, TicketMessage, TicketPriority, TicketStatus,
};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/", get(list).post(create))
        .route("/{id}", get(get_one).put(update))
        .route("/{id}/messages", get(list_messages).post(create_message))
        .route(
            "/{id}/attachments",
            axum::routing::post(upload_attachment).layer(DefaultBodyLimit::max(12 * 1024 * 1024)),
        )
        .route("/{id}/events", get(ticket_events))
}

#[derive(Deserialize)]
struct ListParams {
    customer_id: Option<Uuid>,
    status: Option<String>,
    limit: Option<i64>,
    offset: Option<i64>,
}

async fn list(
    auth: AuthAdmin,
    State(state): State<AppState>,
    Query(params): Query<ListParams>,
) -> Result<Json<Vec<Ticket>>, ApiError> {
    crate::require_permission!(auth, state, "tickets.view");
    let limit = params.limit.unwrap_or(50).min(200);
    let offset = params.offset.unwrap_or(0);

    let tickets: Vec<Ticket> = match (params.customer_id, &params.status) {
        (Some(cid), Some(status)) => sqlx::query_as(
            "SELECT * FROM tickets WHERE customer_id = $1 AND status = $2::ticket_status ORDER BY created_at DESC LIMIT $3 OFFSET $4"
        )
        .bind(cid)
        .bind(status)
        .bind(limit)
        .bind(offset)
        .fetch_all(&state.db)
        .await?,
        (Some(cid), None) => sqlx::query_as(
            "SELECT * FROM tickets WHERE customer_id = $1 ORDER BY created_at DESC LIMIT $2 OFFSET $3"
        )
        .bind(cid)
        .bind(limit)
        .bind(offset)
        .fetch_all(&state.db)
        .await?,
        (None, Some(status)) => sqlx::query_as(
            "SELECT * FROM tickets WHERE status = $1::ticket_status ORDER BY created_at DESC LIMIT $2 OFFSET $3"
        )
        .bind(status)
        .bind(limit)
        .bind(offset)
        .fetch_all(&state.db)
        .await?,
        (None, None) => sqlx::query_as(
            "SELECT * FROM tickets ORDER BY created_at DESC LIMIT $1 OFFSET $2"
        )
        .bind(limit)
        .bind(offset)
        .fetch_all(&state.db)
        .await?,
    };

    Ok(Json(tickets))
}

async fn get_one(
    auth: AuthAdmin,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<Ticket>, ApiError> {
    crate::require_permission!(auth, state, "tickets.view");
    let ticket: Ticket = sqlx::query_as("SELECT * FROM tickets WHERE id = $1")
        .bind(id)
        .fetch_optional(&state.db)
        .await?
        .ok_or(ApiError::NotFound("ticket not found".into()))?;
    Ok(Json(ticket))
}

#[derive(Deserialize)]
struct CreateTicket {
    customer_id: Uuid,
    category: TicketCategory,
    subject: String,
    priority: TicketPriority,
    assignee_id: Option<Uuid>,
    mailbox: String,
    related_service_id: Option<Uuid>,
    ip: Option<String>,
}

async fn create(
    auth: AuthAdmin,
    State(state): State<AppState>,
    Json(payload): Json<CreateTicket>,
) -> Result<Json<Ticket>, ApiError> {
    crate::require_permission!(auth, state, "tickets.reply");
    // Validate inputs
    validate_required_string(&payload.subject, "Subject")?;
    validate_string_length(&payload.subject, "Subject", MAX_SHORT_STRING_LENGTH)?;

    let random_id = Uuid::new_v7(uuid::Timestamp::now(uuid::NoContext));
    let ticket_number = format!("TKT-{}", random_id.to_string()[..8].to_uppercase());

    let ticket: Ticket = sqlx::query_as(
        r#"INSERT INTO tickets (ticket_number, customer_id, category, subject, priority, assignee_id, mailbox, related_service_id, ip)
           VALUES ($1, $2, $3::ticket_category, $4, $5::ticket_priority, $6, $7, $8, $9)
           RETURNING *"#
    )
    .bind(&ticket_number)
    .bind(payload.customer_id)
    .bind(payload.category)
    .bind(&payload.subject)
    .bind(payload.priority)
    .bind(payload.assignee_id)
    .bind(&payload.mailbox)
    .bind(payload.related_service_id)
    .bind(payload.ip.unwrap_or_default())
    .fetch_one(&state.db)
    .await?;

    Ok(Json(ticket))
}

#[derive(Deserialize)]
struct UpdateTicket {
    subject: Option<String>,
    status: Option<TicketStatus>,
    priority: Option<TicketPriority>,
    assignee_id: Option<Uuid>,
    related_service_id: Option<Uuid>,
}

async fn update(
    auth: AuthAdmin,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(payload): Json<UpdateTicket>,
) -> Result<Json<Ticket>, ApiError> {
    crate::require_permission!(auth, state, "tickets.reply");
    let ticket: Ticket = sqlx::query_as(
        r#"UPDATE tickets
           SET subject = COALESCE($1, subject),
               status = COALESCE($2::ticket_status, status),
               priority = COALESCE($3::ticket_priority, priority),
               assignee_id = COALESCE($4, assignee_id),
               related_service_id = COALESCE($5, related_service_id),
               updated_at = now()
           WHERE id = $6
           RETURNING *"#,
    )
    .bind(payload.subject)
    .bind(payload.status)
    .bind(payload.priority)
    .bind(payload.assignee_id)
    .bind(payload.related_service_id)
    .bind(id)
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound("ticket not found".into()))?;

    Ok(Json(ticket))
}

async fn list_messages(
    auth: AuthAdmin,
    State(state): State<AppState>,
    Path(ticket_id): Path<Uuid>,
) -> Result<Json<Vec<TicketMessage>>, ApiError> {
    crate::require_permission!(auth, state, "tickets.view");
    let messages: Vec<TicketMessage> = sqlx::query_as(
        "SELECT * FROM ticket_messages WHERE ticket_id = $1 ORDER BY created_at ASC",
    )
    .bind(ticket_id)
    .fetch_all(&state.db)
    .await?;

    Ok(Json(messages))
}

#[derive(Deserialize)]
struct CreateTicketMessage {
    author_id: Uuid,
    author_type: ActorType,
    content: String,
    internal: Option<bool>,
    attachments: Option<serde_json::Value>,
}

#[derive(Serialize)]
pub struct AttachmentResponse {
    pub id: Uuid,
    pub filename: String,
    pub url: String,
    pub size: usize,
    pub content_type: String,
}

const MAX_ATTACHMENT_SIZE: usize = 10 * 1024 * 1024; // 10MB limit

async fn upload_attachment(
    auth: AuthAdmin,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    mut multipart: Multipart,
) -> Result<Json<AttachmentResponse>, ApiError> {
    crate::require_permission!(auth, state, "tickets.reply");

    let ticket_exists: Option<Uuid> =
        sqlx::query_scalar("SELECT id FROM tickets WHERE id = $1")
            .bind(id)
            .fetch_optional(&state.db)
            .await?;

    if ticket_exists.is_none() {
        return Err(ApiError::NotFound("ticket not found".into()));
    }

    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| ApiError::BadRequest(e.to_string()))?
    {
        let raw_filename = field.file_name().unwrap_or("image.png").to_string();
        let ext = raw_filename
            .rsplit('.')
            .next()
            .unwrap_or("")
            .to_lowercase();

        // 1. Only images accepted
        if !["png", "jpg", "jpeg", "webp", "gif"].contains(&ext.as_str()) {
            return Err(ApiError::BadRequest(
                "Only image files (PNG, JPEG, WebP, GIF) are accepted.".into(),
            ));
        }

        let content_type = field.content_type().unwrap_or("image/png").to_string();
        if !content_type.starts_with("image/") {
            return Err(ApiError::BadRequest("Only image files are accepted.".into()));
        }

        let data = field
            .bytes()
            .await
            .map_err(|e| ApiError::BadRequest(e.to_string()))?;

        // 2. Limit attachment size to 10MB
        if data.len() > MAX_ATTACHMENT_SIZE {
            return Err(ApiError::BadRequest(
                "Image file exceeds the 10MB size limit.".into(),
            ));
        }

        if data.is_empty() {
            return Err(ApiError::BadRequest("File is empty.".into()));
        }

        // 3. Verify magic bytes for real image integrity
        let is_valid_image = match ext.as_str() {
            "png" => data.starts_with(&[0x89, 0x50, 0x4E, 0x47]),
            "jpg" | "jpeg" => data.starts_with(&[0xFF, 0xD8, 0xFF]),
            "gif" => data.starts_with(b"GIF87a") || data.starts_with(b"GIF89a"),
            "webp" => data.starts_with(b"RIFF") && data.len() > 12 && &data[8..12] == b"WEBP",
            _ => false,
        };

        if !is_valid_image {
            return Err(ApiError::BadRequest(
                "File content does not match a valid image format.".into(),
            ));
        }

        let dir = std::path::Path::new("uploads/tickets");
        tokio::fs::create_dir_all(dir)
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?;

        let attach_id = Uuid::now_v7();
        let clean_filename = format!(
            "{}_{}.{}",
            attach_id,
            raw_filename
                .rsplit('.')
                .skip(1)
                .collect::<Vec<_>>()
                .join(".")
                .replace(|c: char| !c.is_alphanumeric() && c != '-' && c != '_', "_"),
            ext
        );
        let path = dir.join(&clean_filename);

        tokio::fs::write(&path, &data)
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?;

        let url = format!("/uploads/tickets/{}", clean_filename);

        return Ok(Json(AttachmentResponse {
            id: attach_id,
            filename: raw_filename,
            url,
            size: data.len(),
            content_type,
        }));
    }

    Err(ApiError::BadRequest("No file uploaded.".into()))
}

async fn create_message(
    auth: AuthAdmin,
    State(state): State<AppState>,
    Path(ticket_id): Path<Uuid>,
    Json(payload): Json<CreateTicketMessage>,
) -> Result<Json<TicketMessage>, ApiError> {
    crate::require_permission!(auth, state, "tickets.reply");
    // Validate inputs
    validate_required_string(&payload.content, "Content")?;
    validate_string_length(&payload.content, "Content", MAX_STRING_LENGTH)?;

    let attachments = payload.attachments.unwrap_or_else(|| serde_json::json!([]));

    let message: TicketMessage = sqlx::query_as(
        r#"INSERT INTO ticket_messages (ticket_id, author_id, author_type, content, internal, attachments)
           VALUES ($1, $2, $3::actor_type, $4, $5, $6)
           RETURNING *"#,
    )
    .bind(ticket_id)
    .bind(payload.author_id)
    .bind(payload.author_type)
    .bind(&payload.content)
    .bind(payload.internal.unwrap_or(false))
    .bind(attachments)
    .fetch_one(&state.db)
    .await?;

    sqlx::query("UPDATE tickets SET updated_at = now() WHERE id = $1")
        .bind(ticket_id)
        .execute(&state.db)
        .await?;

    // Broadcast message in real-time
    let _ = state.ticket_bus.send(crate::state::TicketEvent {
        ticket_id,
        message: message.clone(),
    });

    // If message is not internal, notify customer via email
    if !payload.internal.unwrap_or(false) {
        crate::mailer::spawn_staff_reply_notification(state.clone(), ticket_id, payload.content);
    }

    Ok(Json(message))
}

async fn ticket_events(
    auth: AuthAdmin,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Sse<impl tokio_stream::Stream<Item = Result<Event, Infallible>>>, ApiError> {
    crate::require_permission!(auth, state, "tickets.view");

    let ticket_exists: Option<Uuid> =
        sqlx::query_scalar("SELECT id FROM tickets WHERE id = $1")
            .bind(id)
            .fetch_optional(&state.db)
            .await?;

    if ticket_exists.is_none() {
        return Err(ApiError::NotFound("ticket not found".into()));
    }

    let rx = state.ticket_bus.subscribe();
    let stream = BroadcastStream::new(rx).filter_map(move |item| match item {
        Ok(event) => {
            // Admin can see all messages for this ticket (including internal notes)
            if event.ticket_id == id {
                match serde_json::to_string(&event.message) {
                    Ok(data) => Some(Ok(Event::default().event("message").data(data))),
                    Err(_) => None,
                }
            } else {
                None
            }
        }
        Err(_) => None,
    });

    Ok(Sse::new(stream).keep_alive(KeepAlive::default()))
}
