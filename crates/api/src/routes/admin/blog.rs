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
        .route("/tags", get(list_tags))
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
    tags: Vec<String>,
    author_id: Uuid,
    author_name: Option<String>,
    author_username: Option<String>,
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
            tags: p.tags,
            author_id: p.author_id,
            author_name: None,
            author_username: None,
            status: p.status,
            published_at: p.published_at,
            created_at: p.created_at,
            updated_at: p.updated_at,
        }
    }
}

async fn list_tags(
    auth: AuthAdmin,
    State(state): State<AppState>,
) -> Result<Json<Vec<String>>, ApiError> {
    check_permission(&auth, &state, "blog.read").await?;

    let tags: Vec<String> = sqlx::query_scalar(
        "SELECT DISTINCT unnest(tags) AS tag FROM blog_posts WHERE cardinality(tags) > 0 ORDER BY tag ASC"
    )
    .fetch_all(&state.db)
    .await
    .unwrap_or_default();

    Ok(Json(tags))
}

#[derive(sqlx::FromRow)]
struct AdminBlogRow {
    id: Uuid,
    title: String,
    slug: String,
    excerpt: String,
    image_url: Option<String>,
    content: String,
    tags: Vec<String>,
    author_id: Uuid,
    status: String,
    published_at: Option<chrono::DateTime<chrono::Utc>>,
    created_at: chrono::DateTime<chrono::Utc>,
    updated_at: chrono::DateTime<chrono::Utc>,
    author_username: Option<String>,
    author_first_name: Option<String>,
    author_last_name: Option<String>,
}

async fn list(
    auth: AuthAdmin,
    State(state): State<AppState>,
) -> Result<Json<Vec<BlogPostResp>>, ApiError> {
    check_permission(&auth, &state, "blog.read").await?;

    let posts = sqlx::query_as::<_, AdminBlogRow>(
        "SELECT b.id, b.title, b.slug, b.excerpt, b.image_url, b.content, b.tags, b.author_id, b.status, b.published_at, b.created_at, b.updated_at,
                s.username AS author_username, s.first_name AS author_first_name, s.last_name AS author_last_name
         FROM blog_posts b
         LEFT JOIN staff s ON b.author_id = s.id
         ORDER BY b.created_at DESC"
    )
    .fetch_all(&state.db)
    .await?;

    let resps = posts.into_iter().map(|p| {
        let display_name = match (&p.author_first_name, &p.author_last_name) {
            (Some(f), Some(l)) if !f.trim().is_empty() || !l.trim().is_empty() => {
                Some(format!("{} {}", f.trim(), l.trim()).trim().to_string())
            }
            (Some(f), None) if !f.trim().is_empty() => Some(f.trim().to_string()),
            _ => p.author_username.clone(),
        };

        BlogPostResp {
            id: p.id,
            title: p.title,
            slug: p.slug,
            excerpt: p.excerpt,
            image_url: p.image_url,
            content: p.content,
            tags: p.tags,
            author_id: p.author_id,
            author_name: display_name,
            author_username: p.author_username,
            status: p.status,
            published_at: p.published_at,
            created_at: p.created_at,
            updated_at: p.updated_at,
        }
    }).collect();

    Ok(Json(resps))
}

#[derive(Deserialize)]
struct CreatePostPayload {
    title: String,
    slug: String,
    excerpt: String,
    image_url: Option<String>,
    content: String,
    status: Option<String>,
    tags: Option<Vec<String>>,
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
    let tags = payload.tags.unwrap_or_default();

    let post = sqlx::query_as::<_, BlogPost>(
        r#"
        INSERT INTO blog_posts (title, slug, excerpt, image_url, content, author_id, status, published_at, tags)
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
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
    .bind(&tags)
    .fetch_one(&state.db)
    .await?;

    Ok(Json(post.into()))
}

async fn get_one(
    auth: AuthAdmin,
    Path(id_or_slug): Path<String>,
    State(state): State<AppState>,
) -> Result<Json<BlogPostResp>, ApiError> {
    check_permission(&auth, &state, "blog.read").await?;

    let post = if let Ok(uuid) = Uuid::parse_str(&id_or_slug) {
        sqlx::query_as::<_, BlogPost>("SELECT * FROM blog_posts WHERE id = $1")
            .bind(uuid)
            .fetch_optional(&state.db)
            .await?
    } else {
        sqlx::query_as::<_, BlogPost>("SELECT * FROM blog_posts WHERE slug = $1")
            .bind(&id_or_slug)
            .fetch_optional(&state.db)
            .await?
    };

    let post = post.ok_or(ApiError::NotFound("Post not found".into()))?;

    Ok(Json(post.into()))
}

async fn update(
    auth: AuthAdmin,
    Path(id_or_slug): Path<String>,
    State(state): State<AppState>,
    Json(payload): Json<CreatePostPayload>,
) -> Result<Json<BlogPostResp>, ApiError> {
    check_permission(&auth, &state, "blog.edit").await?;

    validate_required_string("title", &payload.title)?;
    validate_required_string("slug", &payload.slug)?;
    validate_required_string("excerpt", &payload.excerpt)?;
    validate_required_string("content", &payload.content)?;
    
    let status = payload.status.unwrap_or_else(|| "draft".to_string());
    
    let current_post = if let Ok(uuid) = Uuid::parse_str(&id_or_slug) {
        sqlx::query_as::<_, BlogPost>("SELECT * FROM blog_posts WHERE id = $1")
            .bind(uuid)
            .fetch_optional(&state.db)
            .await?
    } else {
        sqlx::query_as::<_, BlogPost>("SELECT * FROM blog_posts WHERE slug = $1")
            .bind(&id_or_slug)
            .fetch_optional(&state.db)
            .await?
    };
    let current_post = current_post.ok_or(ApiError::NotFound("Post not found".into()))?;
        
    let mut published_at = current_post.published_at;
    if status == "published" && published_at.is_none() {
        published_at = Some(chrono::Utc::now());
    } else if status == "draft" {
        published_at = None;
    }

    let tags = payload.tags.unwrap_or(current_post.tags);

    let post = sqlx::query_as::<_, BlogPost>(
        r#"
        UPDATE blog_posts
        SET title = $1, slug = $2, excerpt = $3, image_url = $4, content = $5, status = $6, published_at = $7, tags = $8, updated_at = NOW()
        WHERE id = $9
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
    .bind(&tags)
    .bind(current_post.id)
    .fetch_one(&state.db)
    .await?;

    Ok(Json(post.into()))
}

async fn delete(
    auth: AuthAdmin,
    Path(id_or_slug): Path<String>,
    State(state): State<AppState>,
) -> Result<Json<serde_json::Value>, ApiError> {
    check_permission(&auth, &state, "blog.delete").await?;

    let result = if let Ok(uuid) = Uuid::parse_str(&id_or_slug) {
        sqlx::query("DELETE FROM blog_posts WHERE id = $1")
            .bind(uuid)
            .execute(&state.db)
            .await?
    } else {
        sqlx::query("DELETE FROM blog_posts WHERE slug = $1")
            .bind(&id_or_slug)
            .execute(&state.db)
            .await?
    };

    if result.rows_affected() == 0 {
        return Err(ApiError::NotFound("Post not found".into()));
    }

    Ok(Json(serde_json::json!({"success": true})))
}
