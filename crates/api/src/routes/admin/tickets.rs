use axum::extract::{Path, Query, State};
use axum::routing::get;
use axum::{Json, Router};
use serde::Deserialize;
use uuid::Uuid;

use crate::auth::AuthAdmin;
use crate::error::ApiError;
use crate::state::AppState;
use crate::validation::{
    validate_required_string, validate_string_length, MAX_SHORT_STRING_LENGTH, MAX_STRING_LENGTH,
};
use vaultscope_db::models::{ActorType, Ticket, TicketCategory, TicketMessage, TicketPriority, TicketStatus};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/", get(list).post(create))
        .route("/{id}", get(get_one).put(update))
        .route("/{id}/messages", get(list_messages).post(create_message))
}

#[derive(Deserialize)]
struct ListParams {
    customer_id: Option<Uuid>,
    status: Option<String>,
    limit: Option<i64>,
    offset: Option<i64>,
}

async fn list(
    _auth: AuthAdmin,
    State(state): State<AppState>,
    Query(params): Query<ListParams>,
) -> Result<Json<Vec<Ticket>>, ApiError> {
    let limit = params.limit.unwrap_or(50).min(200);
    let offset = params.offset.unwrap_or(0);

    let tickets: Vec<Ticket> = if let Some(cid) = params.customer_id {
        sqlx::query_as(
            "SELECT * FROM tickets WHERE customer_id = $1 ORDER BY created_at DESC LIMIT $2 OFFSET $3"
        )
        .bind(cid)
        .bind(limit)
        .bind(offset)
        .fetch_all(&state.db)
        .await?
    } else {
        sqlx::query_as(
            "SELECT * FROM tickets ORDER BY created_at DESC LIMIT $1 OFFSET $2"
        )
        .bind(limit)
        .bind(offset)
        .fetch_all(&state.db)
        .await?
    };

    Ok(Json(tickets))
}

async fn get_one(
    _auth: AuthAdmin,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<Ticket>, ApiError> {
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
    _auth: AuthAdmin,
    State(state): State<AppState>,
    Json(payload): Json<CreateTicket>,
) -> Result<Json<Ticket>, ApiError> {
    // Validate inputs
    validate_required_string(&payload.subject, "Subject")?;
    validate_string_length(&payload.subject, "Subject", MAX_SHORT_STRING_LENGTH)?;

    let random_id = Uuid::new_v7(uuid::Timestamp::now(uuid::NoContext));
    let ticket_number = format!("TKT-{}", &random_id.to_string()[..8].to_uppercase());

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
}

async fn update(
    _auth: AuthAdmin,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(payload): Json<UpdateTicket>,
) -> Result<Json<Ticket>, ApiError> {
    let ticket: Ticket = sqlx::query_as(
        r#"UPDATE tickets
           SET subject = COALESCE($1, subject),
               status = COALESCE($2::ticket_status, status),
               priority = COALESCE($3::ticket_priority, priority),
               assignee_id = COALESCE($4, assignee_id),
               updated_at = now()
           WHERE id = $5
           RETURNING *"#
    )
    .bind(payload.subject)
    .bind(payload.status)
    .bind(payload.priority)
    .bind(payload.assignee_id)
    .bind(id)
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound("ticket not found".into()))?;

    Ok(Json(ticket))
}

async fn list_messages(
    _auth: AuthAdmin,
    State(state): State<AppState>,
    Path(ticket_id): Path<Uuid>,
) -> Result<Json<Vec<TicketMessage>>, ApiError> {
    let messages: Vec<TicketMessage> = sqlx::query_as(
        "SELECT * FROM ticket_messages WHERE ticket_id = $1 ORDER BY created_at ASC"
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
}

async fn create_message(
    _auth: AuthAdmin,
    State(state): State<AppState>,
    Path(ticket_id): Path<Uuid>,
    Json(payload): Json<CreateTicketMessage>,
) -> Result<Json<TicketMessage>, ApiError> {
    // Validate inputs
    validate_required_string(&payload.content, "Content")?;
    validate_string_length(&payload.content, "Content", MAX_STRING_LENGTH)?;

    let message: TicketMessage = sqlx::query_as(
        r#"INSERT INTO ticket_messages (ticket_id, author_id, author_type, content, internal)
           VALUES ($1, $2, $3::actor_type, $4, $5)
           RETURNING *"#
    )
    .bind(ticket_id)
    .bind(payload.author_id)
    .bind(payload.author_type)
    .bind(&payload.content)
    .bind(payload.internal.unwrap_or(false))
    .fetch_one(&state.db)
    .await?;

    sqlx::query("UPDATE tickets SET updated_at = now() WHERE id = $1")
        .bind(ticket_id)
        .execute(&state.db)
        .await?;

    Ok(Json(message))
}
