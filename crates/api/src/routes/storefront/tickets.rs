use axum::extract::{Path, State};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::auth::AuthCustomer;
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
        .route("/", get(list_mine).post(create_ticket))
        .route("/{id}", get(get_one))
        .route("/{id}/messages", post(add_message))
}

async fn list_mine(
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

#[derive(Serialize)]
pub struct TicketWithMessages {
    pub ticket: Ticket,
    pub messages: Vec<TicketMessage>,
}

async fn get_one(
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

    Ok(Json(TicketWithMessages { ticket, messages }))
}

#[derive(Deserialize)]
pub struct CreateTicketPayload {
    pub subject: String,
    pub content: String,
    pub category: TicketCategory,
    pub priority: TicketPriority,
    pub related_service_id: Option<Uuid>,
}

async fn create_ticket(
    auth: AuthCustomer,
    State(state): State<AppState>,
    Json(payload): Json<CreateTicketPayload>,
) -> Result<Json<Ticket>, ApiError> {
    validate_required_string(&payload.subject, "Subject")?;
    validate_string_length(&payload.subject, "Subject", MAX_SHORT_STRING_LENGTH)?;
    validate_required_string(&payload.content, "Content")?;
    validate_string_length(&payload.content, "Content", MAX_STRING_LENGTH)?;

    // Fetch the default mailbox to associate with this ticket
    let default_mailbox: Option<String> =
        sqlx::query_scalar("SELECT user_name FROM mailboxes WHERE is_default = true LIMIT 1")
            .fetch_optional(&state.db)
            .await
            .unwrap_or(None);
    let mailbox = default_mailbox.unwrap_or_default();

    let ticket_id = Uuid::now_v7();
    // generate simple ticket number
    let ticket_number = format!("TKT-{}", ticket_id.to_string()[0..6].to_uppercase());

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
        "INSERT INTO ticket_messages (id, ticket_id, author_id, author_type, content, internal)
         VALUES ($1, $2, $3, $4, $5, $6)",
    )
    .bind(Uuid::now_v7())
    .bind(ticket_id)
    .bind(auth.0.sub)
    .bind(ActorType::Customer)
    .bind(payload.content)
    .bind(false)
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;

    Ok(Json(ticket))
}

#[derive(Deserialize)]
pub struct AddMessagePayload {
    pub content: String,
}

async fn add_message(
    auth: AuthCustomer,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(payload): Json<AddMessagePayload>,
) -> Result<Json<TicketMessage>, ApiError> {
    validate_required_string(&payload.content, "Content")?;
    validate_string_length(&payload.content, "Content", MAX_STRING_LENGTH)?;

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
        "INSERT INTO ticket_messages (id, ticket_id, author_id, author_type, content, internal)
         VALUES ($1, $2, $3, $4, $5, $6)
         RETURNING *",
    )
    .bind(Uuid::now_v7())
    .bind(id)
    .bind(auth.0.sub)
    .bind(ActorType::Customer)
    .bind(payload.content)
    .bind(false)
    .fetch_one(&mut *tx)
    .await?;

    // Update ticket status and updated_at
    sqlx::query("UPDATE tickets SET status = $1, updated_at = NOW() WHERE id = $2")
        .bind(TicketStatus::Open)
        .bind(id)
        .execute(&mut *tx)
        .await?;

    tx.commit().await?;

    Ok(Json(message))
}
