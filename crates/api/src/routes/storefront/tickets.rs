use std::convert::Infallible;
use axum::extract::{DefaultBodyLimit, Multipart, Path, State};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::Json;
use serde::{Deserialize, Serialize};
use tokio_stream::StreamExt;
use tokio_stream::wrappers::BroadcastStream;
use utoipa::ToSchema;
use utoipa_axum::{router::OpenApiRouter, routes};
use uuid::Uuid;

use crate::auth::AuthCustomer;
use crate::error::ApiError;
use crate::openapi::{BinaryFileUpload, ErrorResponse};
use crate::state::AppState;
use crate::validation::{
    validate_required_string, validate_string_length, MAX_SHORT_STRING_LENGTH, MAX_STRING_LENGTH,
};
use vaultscope_db::models::{
    ActorType, Ticket, TicketCategory, TicketMessage, TicketPriority, TicketStatus,
};

pub fn routes() -> OpenApiRouter<AppState> {
    let attachment_router = OpenApiRouter::new()
        .routes(routes!(upload_attachment))
        .layer(DefaultBodyLimit::max(12 * 1024 * 1024));

    OpenApiRouter::new()
        .routes(routes!(list_mine, create_ticket))
        .routes(routes!(get_one))
        .routes(routes!(add_message))
        .routes(routes!(ticket_events))
        .merge(attachment_router)
}

#[utoipa::path(
    get,
    path = "/",
    tag = "Storefront Support",
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "List customer tickets", body = Vec<Ticket>),
        (status = 401, description = "Unauthorized", body = ErrorResponse)
    )
)]
pub(crate) async fn list_mine(
    auth: AuthCustomer,
    State(state): State<AppState>,
) -> Result<Json<Vec<Ticket>>, ApiError> {
    let tickets: Vec<Ticket> =
        sqlx::query_as("SELECT * FROM tickets WHERE customer_id = $1 ORDER BY updated_at DESC")
            .bind(auth.0.sub)
            .fetch_all(&state.db)
            .await?;
    Ok(Json(tickets))
}

#[derive(Serialize, ToSchema)]
pub struct TicketWithMessages {
    pub ticket: Ticket,
    pub messages: Vec<TicketMessage>,
    pub related_service: Option<vaultscope_db::models::Service>,
}

#[utoipa::path(
    get,
    path = "/{id}",
    tag = "Storefront Support",
    security(("bearer_auth" = [])),
    params(("id" = Uuid, Path, description = "Ticket ID")),
    responses(
        (status = 200, description = "Ticket details with public messages and related service", body = TicketWithMessages),
        (status = 404, description = "Ticket not found", body = ErrorResponse)
    )
)]
pub(crate) async fn get_one(
    auth: AuthCustomer,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<TicketWithMessages>, ApiError> {
    let ticket: Ticket = sqlx::query_as("SELECT * FROM tickets WHERE id = $1 AND customer_id = $2")
        .bind(id)
        .bind(auth.0.sub)
        .fetch_optional(&state.db)
        .await?
        .ok_or(ApiError::NotFound("ticket not found".into()))?;

    let messages: Vec<TicketMessage> = sqlx::query_as(
        "SELECT * FROM ticket_messages WHERE ticket_id = $1 AND internal = false ORDER BY created_at ASC",
    )
    .bind(id)
    .fetch_all(&state.db)
    .await?;

    let related_service: Option<vaultscope_db::models::Service> = match ticket.related_service_id {
        Some(sid) => {
            sqlx::query_as("SELECT * FROM services WHERE id = $1 AND customer_id = $2")
                .bind(sid)
                .bind(auth.0.sub)
                .fetch_optional(&state.db)
                .await?
        }
        None => None,
    };

    Ok(Json(TicketWithMessages { ticket, messages, related_service }))
}

#[derive(Deserialize, ToSchema)]
#[schema(as = StorefrontCreateTicketPayload)]
pub struct CreateTicketPayload {
    pub subject: String,
    pub content: String,
    pub category: TicketCategory,
    pub priority: TicketPriority,
    pub related_service_id: Option<Uuid>,
    pub attachments: Option<serde_json::Value>,
}

#[utoipa::path(
    post,
    path = "/",
    tag = "Storefront Support",
    security(("bearer_auth" = []), ("csrf_token" = [])),
    request_body = CreateTicketPayload,
    responses(
        (status = 200, description = "Created support ticket", body = Ticket),
        (status = 400, description = "Validation error", body = ErrorResponse)
    )
)]
pub(crate) async fn create_ticket(
    auth: AuthCustomer,
    State(state): State<AppState>,
    Json(payload): Json<CreateTicketPayload>,
) -> Result<Json<Ticket>, ApiError> {
    validate_required_string(&payload.subject, "Subject")?;
    validate_string_length(&payload.subject, "Subject", MAX_SHORT_STRING_LENGTH)?;
    validate_required_string(&payload.content, "Content")?;
    validate_string_length(&payload.content, "Content", MAX_STRING_LENGTH)?;

    // Security Check: Customer can only bind tickets to services they own
    if let Some(service_id) = payload.related_service_id {
        let owns_service: Option<Uuid> = sqlx::query_scalar(
            "SELECT id FROM services WHERE id = $1 AND customer_id = $2"
        )
        .bind(service_id)
        .bind(auth.0.sub)
        .fetch_optional(&state.db)
        .await?;

        if owns_service.is_none() {
            return Err(ApiError::BadRequest("The selected service does not belong to your account".into()));
        }
    }

    // Fetch the default mailbox to associate with this ticket
    let default_mailbox: Option<String> =
        sqlx::query_scalar("SELECT user_name FROM mailboxes WHERE is_default = true LIMIT 1")
            .fetch_optional(&state.db)
            .await
            .unwrap_or(None);
    let mailbox = default_mailbox.unwrap_or_default();

    let initial_attachments =
        crate::media::validate_ticket_attachments(payload.attachments.as_ref())?;

    let ticket_id = Uuid::now_v7();
    let id_str = ticket_id.to_string();
    let ticket_number = format!("TKT-{}", id_str[24..36].to_uppercase());

    let mut tx = state.db.begin().await?;

    let ticket: Ticket = sqlx::query_as(
        "INSERT INTO tickets (id, ticket_number, customer_id, category, subject, status, priority, mailbox, related_service_id, ip)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
         RETURNING *"
    )
    .bind(ticket_id)
    .bind(ticket_number)
    .bind(auth.0.sub)
    .bind(payload.category)
    .bind(payload.subject)
    .bind(TicketStatus::Open)
    .bind(payload.priority)
    .bind(&mailbox)
    .bind(payload.related_service_id)
    .bind("127.0.0.1")
    .fetch_one(&mut *tx)
    .await?;

    sqlx::query(
        "INSERT INTO ticket_messages (id, ticket_id, author_id, author_type, content, internal, attachments)
         VALUES ($1, $2, $3, $4, $5, $6, $7)",
    )
    .bind(Uuid::now_v7())
    .bind(ticket_id)
    .bind(auth.0.sub)
    .bind(ActorType::Customer)
    .bind(payload.content)
    .bind(false)
    .bind(initial_attachments)
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;

    Ok(Json(ticket))
}

#[derive(Deserialize, ToSchema)]
pub struct AddMessagePayload {
    pub content: String,
    pub attachments: Option<serde_json::Value>,
}

#[derive(Serialize, ToSchema)]
#[schema(as = StorefrontAttachmentResponse)]
pub struct AttachmentResponse {
    pub id: Uuid,
    pub filename: String,
    pub url: String,
    pub size: usize,
    pub content_type: String,
}

#[utoipa::path(
    post,
    path = "/{id}/attachments",
    tag = "Storefront Support",
    security(("bearer_auth" = []), ("csrf_token" = [])),
    params(("id" = Uuid, Path, description = "Ticket ID")),
    request_body(content = BinaryFileUpload, content_type = "multipart/form-data"),
    responses(
        (status = 200, description = "Uploaded image attachment", body = AttachmentResponse),
        (status = 400, description = "Invalid image or size exceeded", body = ErrorResponse),
        (status = 404, description = "Ticket not found", body = ErrorResponse)
    )
)]
pub(crate) async fn upload_attachment(
    auth: AuthCustomer,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    mut multipart: Multipart,
) -> Result<Json<AttachmentResponse>, ApiError> {
    // Verify customer owns this ticket
    let ticket_exists: Option<Uuid> =
        sqlx::query_scalar("SELECT id FROM tickets WHERE id = $1 AND customer_id = $2")
            .bind(id)
            .bind(auth.0.sub)
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
        let content_type = field.content_type().map(|s| s.to_string());

        let data = field
            .bytes()
            .await
            .map_err(|e| ApiError::BadRequest(e.to_string()))?;

        let processed = crate::media::validate_and_save_ticket_attachment(
            &raw_filename,
            content_type.as_deref(),
            &data,
        )
        .await?;

        return Ok(Json(AttachmentResponse {
            id: processed.id,
            filename: raw_filename,
            url: processed.url,
            size: processed.size,
            content_type: processed.content_type,
        }));
    }

    Err(ApiError::BadRequest("No file uploaded.".into()))
}

#[utoipa::path(
    post,
    path = "/{id}/messages",
    tag = "Storefront Support",
    security(("bearer_auth" = []), ("csrf_token" = [])),
    params(("id" = Uuid, Path, description = "Ticket ID")),
    request_body = AddMessagePayload,
    responses(
        (status = 200, description = "Posted reply message to ticket", body = TicketMessage),
        (status = 404, description = "Ticket not found", body = ErrorResponse)
    )
)]
pub(crate) async fn add_message(
    auth: AuthCustomer,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(payload): Json<AddMessagePayload>,
) -> Result<Json<TicketMessage>, ApiError> {
    validate_required_string(&payload.content, "Content")?;
    validate_string_length(&payload.content, "Content", MAX_STRING_LENGTH)?;
    let attachments = crate::media::validate_ticket_attachments(payload.attachments.as_ref())?;

    // verify ownership
    let ticket_exists: Option<Uuid> =
        sqlx::query_scalar("SELECT id FROM tickets WHERE id = $1 AND customer_id = $2")
            .bind(id)
            .bind(auth.0.sub)
            .fetch_optional(&state.db)
            .await?;

    if ticket_exists.is_none() {
        return Err(ApiError::NotFound("ticket not found".into()));
    }

    let mut tx = state.db.begin().await?;

    let message: TicketMessage = sqlx::query_as(
        "INSERT INTO ticket_messages (id, ticket_id, author_id, author_type, content, internal, attachments)
         VALUES ($1, $2, $3, $4, $5, $6, $7)
         RETURNING *",
    )
    .bind(Uuid::now_v7())
    .bind(id)
    .bind(auth.0.sub)
    .bind(ActorType::Customer)
    .bind(&payload.content)
    .bind(false)
    .bind(attachments)
    .fetch_one(&mut *tx)
    .await?;

    // Update ticket status and updated_at
    sqlx::query("UPDATE tickets SET status = $1, updated_at = NOW() WHERE id = $2")
        .bind(TicketStatus::Open)
        .bind(id)
        .execute(&mut *tx)
        .await?;

    tx.commit().await?;

    // Broadcast new message in real-time
    let _ = state.ticket_bus.send(crate::state::TicketEvent {
        ticket_id: id,
        message: message.clone(),
    });

    // Notify support staff via email
    crate::mailer::spawn_customer_reply_notification(state.clone(), id, payload.content);

    Ok(Json(message))
}

#[utoipa::path(
    get,
    path = "/{id}/events",
    tag = "Storefront Support",
    security(("bearer_auth" = [])),
    params(("id" = Uuid, Path, description = "Ticket ID")),
    responses(
        (status = 200, description = "Server-Sent Events stream of public ticket replies", content_type = "text/event-stream", body = String),
        (status = 404, description = "Ticket not found", body = ErrorResponse)
    )
)]
pub(crate) async fn ticket_events(
    auth: AuthCustomer,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Sse<impl tokio_stream::Stream<Item = Result<Event, Infallible>>>, ApiError> {
    // Verify customer owns this ticket
    let ticket_exists: Option<Uuid> =
        sqlx::query_scalar("SELECT id FROM tickets WHERE id = $1 AND customer_id = $2")
            .bind(id)
            .bind(auth.0.sub)
            .fetch_optional(&state.db)
            .await?;

    if ticket_exists.is_none() {
        return Err(ApiError::NotFound("ticket not found".into()));
    }

    let rx = state.ticket_bus.subscribe();
    let stream = BroadcastStream::new(rx).filter_map(move |item| match item {
        Ok(event) => {
            // Must match this ticket and customer must not see internal staff notes
            if event.ticket_id == id && !event.message.internal {
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
