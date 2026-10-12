use axum::extract::{Path, State};
use axum::routing::get;
use axum::{Json, Router};
use serde::Serialize;
use uuid::Uuid;
use vaultscope_db::models::BlogPost;

use crate::error::ApiError;
use crate::state::AppState;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/", get(list_published))
        .route("/{slug}", get(get_by_slug))
}

#[derive(Serialize)]
struct PublicBlogPost {
    id: Uuid,
    title: String,
    slug: String,
    excerpt: String,
    image_url: Option<String>,
    content: String,
    published_at: chrono::DateTime<chrono::Utc>,
}

async fn list_published(
    State(state): State<AppState>,
) -> Result<Json<Vec<PublicBlogPost>>, ApiError> {
    let posts = sqlx::query_as::<_, BlogPost>(
        "SELECT * FROM blog_posts WHERE status = 'published' ORDER BY published_at DESC"
    )
    .fetch_all(&state.db)
    .await?;

    let public_posts = posts.into_iter().map(|p| PublicBlogPost {
        id: p.id,
        title: p.title,
        slug: p.slug,
        excerpt: p.excerpt,
        image_url: p.image_url,
        content: p.content,
        published_at: p.published_at.unwrap_or(p.created_at),
    }).collect();

    Ok(Json(public_posts))
}

async fn get_by_slug(
    Path(slug): Path<String>,
    State(state): State<AppState>,
) -> Result<Json<PublicBlogPost>, ApiError> {
    let post = sqlx::query_as::<_, BlogPost>(
        "SELECT * FROM blog_posts WHERE slug = $1 AND status = 'published'"
    )
    .bind(slug)
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound("Post not found".into()))?;

    Ok(Json(PublicBlogPost {
        id: post.id,
        title: post.title,
        slug: post.slug,
        excerpt: post.excerpt,
        image_url: post.image_url,
        content: post.content,
        published_at: post.published_at.unwrap_or(post.created_at),
    }))
}
