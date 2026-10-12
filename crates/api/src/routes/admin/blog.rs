use axum::extract::{Path, State};
use axum::routing::get;
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use vaultscope_db::models::BlogPost;

use crate::auth::AuthAdmin;
use crate::error::ApiError;
use crate::middleware::rbac::check_permission;
use crate::state::AppState;
use crate::validation::{validate_required_string, validate_string_length};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/", get(list).post(create))
        .route("/{id}", get(get_one).put(update).delete(delete))
}

#[derive(Serialize)]
struct BlogPostResp {
    id: Uuid,
    title: String,
    slug: String,
    excerpt: String,
    image_url: Option<String>,
    content: String,
    author_id: Uuid,
    status: String,
    published_at: Option<chrono::DateTime<chrono::Utc>>,
    created_at: chrono::DateTime<chrono::Utc>,
    updated_at: chrono::DateTime<chrono::Utc>,
}

impl From<BlogPost> for BlogPostResp {
    fn from(p: BlogPost) -> Self {
        Self {
            id: p.id,
            title: p.title,
            slug: p.slug,
            excerpt: p.excerpt,
            image_url: p.image_url,
            content: p.content,
            author_id: p.author_id,
            status: p.status,
            published_at: p.published_at,
            created_at: p.created_at,
            updated_at: p.updated_at,
        }
    }
}

async fn list(
    auth: AuthAdmin,
    State(state): State<AppState>,
) -> Result<Json<Vec<BlogPostResp>>, ApiError> {
    check_permission(&auth, &state, "blog.read").await?;

    let posts = sqlx::query_as::<_, BlogPost>("SELECT * FROM blog_posts ORDER BY created_at DESC")
        .fetch_all(&state.db)
        .await?;

    Ok(Json(posts.into_iter().map(|p| p.into()).collect()))
}

#[derive(Deserialize)]
struct CreatePostPayload {
    title: String,
    slug: String,
    excerpt: String,
    image_url: Option<String>,
    content: String,
    status: Option<String>,
}

async fn create(
    auth: AuthAdmin,
    State(state): State<AppState>,
    Json(payload): Json<CreatePostPayload>,
) -> Result<Json<BlogPostResp>, ApiError> {
    check_permission(&auth, &state, "blog.write").await?;

    validate_required_string("title", &payload.title)?;
    validate_required_string("slug", &payload.slug)?;
    validate_required_string("excerpt", &payload.excerpt)?;
    validate_required_string("content", &payload.content)?;
    validate_string_length(&payload.slug, "slug", 255)?;
    
    let status = payload.status.unwrap_or_else(|| "draft".to_string());
    let published_at = if status == "published" { Some(chrono::Utc::now()) } else { None };

    let post = sqlx::query_as::<_, BlogPost>(
        r#"
        INSERT INTO blog_posts (title, slug, excerpt, image_url, content, author_id, status, published_at)
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
        RETURNING *
        "#,
    )
    .bind(&payload.title)
    .bind(&payload.slug)
    .bind(&payload.excerpt)
    .bind(&payload.image_url)
    .bind(&payload.content)
    .bind(auth.0.sub)
    .bind(status)
    .bind(published_at)
    .fetch_one(&state.db)
    .await?;

    Ok(Json(post.into()))
}

async fn get_one(
    auth: AuthAdmin,
    Path(id): Path<Uuid>,
    State(state): State<AppState>,
) -> Result<Json<BlogPostResp>, ApiError> {
    check_permission(&auth, &state, "blog.read").await?;

    let post = sqlx::query_as::<_, BlogPost>("SELECT * FROM blog_posts WHERE id = $1")
        .bind(id)
        .fetch_optional(&state.db)
        .await?
        .ok_or(ApiError::NotFound("Post not found".into()))?;

    Ok(Json(post.into()))
}

async fn update(
    auth: AuthAdmin,
    Path(id): Path<Uuid>,
    State(state): State<AppState>,
    Json(payload): Json<CreatePostPayload>,
) -> Result<Json<BlogPostResp>, ApiError> {
    check_permission(&auth, &state, "blog.edit").await?;

    validate_required_string("title", &payload.title)?;
    validate_required_string("slug", &payload.slug)?;
    validate_required_string("excerpt", &payload.excerpt)?;
    validate_required_string("content", &payload.content)?;
    
    let status = payload.status.unwrap_or_else(|| "draft".to_string());
    
    let current_post = sqlx::query_as::<_, BlogPost>("SELECT * FROM blog_posts WHERE id = $1")
        .bind(id)
        .fetch_optional(&state.db)
        .await?
        .ok_or(ApiError::NotFound("Post not found".into()))?;
        
    let mut published_at = current_post.published_at;
    if status == "published" && published_at.is_none() {
        published_at = Some(chrono::Utc::now());
    } else if status == "draft" {
        published_at = None;
    }

    let post = sqlx::query_as::<_, BlogPost>(
        r#"
        UPDATE blog_posts
        SET title = $1, slug = $2, excerpt = $3, image_url = $4, content = $5, status = $6, published_at = $7, updated_at = NOW()
        WHERE id = $8
        RETURNING *
        "#,
    )
    .bind(&payload.title)
    .bind(&payload.slug)
    .bind(&payload.excerpt)
    .bind(&payload.image_url)
    .bind(&payload.content)
    .bind(status)
    .bind(published_at)
    .bind(id)
    .fetch_one(&state.db)
    .await?;

    Ok(Json(post.into()))
}

async fn delete(
    auth: AuthAdmin,
    Path(id): Path<Uuid>,
    State(state): State<AppState>,
) -> Result<Json<serde_json::Value>, ApiError> {
    check_permission(&auth, &state, "blog.delete").await?;

    let result = sqlx::query("DELETE FROM blog_posts WHERE id = $1")
        .bind(id)
        .execute(&state.db)
        .await?;

    if result.rows_affected() == 0 {
        return Err(ApiError::NotFound("Post not found".into()));
    }

    Ok(Json(serde_json::json!({"success": true})))
}
