use axum::extract::{Path, Query, State};
use axum::routing::get;
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::ApiError;
use crate::state::AppState;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/", get(list_published))
        .route("/tags", get(list_tags))
        .route("/{slug}", get(get_by_slug))
}

#[derive(Serialize)]
pub struct BlogAuthorInfo {
    pub name: String,
    pub username: String,
    pub avatar: Option<String>,
}

#[derive(Serialize)]
pub struct PublicBlogPost {
    pub id: Uuid,
    pub title: String,
    pub slug: String,
    pub excerpt: String,
    pub image_url: Option<String>,
    pub content: String,
    pub tags: Vec<String>,
    pub published_at: chrono::DateTime<chrono::Utc>,
    pub author_name: Option<String>,
    pub author: Option<BlogAuthorInfo>,
}

#[derive(sqlx::FromRow)]
struct BlogPostRow {
    id: Uuid,
    title: String,
    slug: String,
    excerpt: String,
    image_url: Option<String>,
    content: String,
    tags: Vec<String>,
    published_at: Option<chrono::DateTime<chrono::Utc>>,
    created_at: chrono::DateTime<chrono::Utc>,
    author_username: Option<String>,
    author_first_name: Option<String>,
    author_last_name: Option<String>,
    author_avatar: Option<String>,
}

impl BlogPostRow {
    fn into_public(self) -> PublicBlogPost {
        let display_name = match (&self.author_first_name, &self.author_last_name) {
            (Some(f), Some(l)) if !f.trim().is_empty() || !l.trim().is_empty() => {
                format!("{} {}", f.trim(), l.trim()).trim().to_string()
            }
            (Some(f), None) if !f.trim().is_empty() => f.trim().to_string(),
            _ => self.author_username.clone().unwrap_or_else(|| "VaultScope Team".to_string()),
        };

        let author = Some(BlogAuthorInfo {
            name: display_name.clone(),
            username: self.author_username.clone().unwrap_or_else(|| "team".to_string()),
            avatar: self.author_avatar.filter(|a| !a.trim().is_empty()),
        });

        PublicBlogPost {
            id: self.id,
            title: self.title,
            slug: self.slug,
            excerpt: self.excerpt,
            image_url: self.image_url,
            content: self.content,
            tags: self.tags,
            published_at: self.published_at.unwrap_or(self.created_at),
            author_name: Some(display_name),
            author,
        }
    }
}

#[derive(Deserialize)]
pub struct ListBlogParams {
    tag: Option<String>,
}

async fn list_tags(
    State(state): State<AppState>,
) -> Result<Json<Vec<String>>, ApiError> {
    let tags: Vec<String> = sqlx::query_scalar(
        "SELECT DISTINCT unnest(tags) AS tag FROM blog_posts WHERE status = 'published' AND cardinality(tags) > 0 ORDER BY tag ASC"
    )
    .fetch_all(&state.db)
    .await
    .unwrap_or_default();

    Ok(Json(tags))
}

async fn list_published(
    State(state): State<AppState>,
    Query(params): Query<ListBlogParams>,
) -> Result<Json<Vec<PublicBlogPost>>, ApiError> {
    let posts = if let Some(tag) = params.tag.filter(|t| !t.trim().is_empty()) {
        sqlx::query_as::<_, BlogPostRow>(
            "SELECT b.id, b.title, b.slug, b.excerpt, b.image_url, b.content, b.tags, b.published_at, b.created_at,
                    s.username AS author_username, s.first_name AS author_first_name, s.last_name AS author_last_name, s.avatar_url AS author_avatar
             FROM blog_posts b
             LEFT JOIN staff s ON b.author_id = s.id
             WHERE b.status = 'published' AND $1 = ANY(b.tags)
             ORDER BY b.published_at DESC"
        )
        .bind(tag.trim().to_lowercase())
        .fetch_all(&state.db)
        .await?
    } else {
        sqlx::query_as::<_, BlogPostRow>(
            "SELECT b.id, b.title, b.slug, b.excerpt, b.image_url, b.content, b.tags, b.published_at, b.created_at,
                    s.username AS author_username, s.first_name AS author_first_name, s.last_name AS author_last_name, s.avatar_url AS author_avatar
             FROM blog_posts b
             LEFT JOIN staff s ON b.author_id = s.id
             WHERE b.status = 'published'
             ORDER BY b.published_at DESC"
        )
        .fetch_all(&state.db)
        .await?
    };

    let public_posts = posts.into_iter().map(|p| p.into_public()).collect();

    Ok(Json(public_posts))
}

async fn get_by_slug(
    Path(slug): Path<String>,
    State(state): State<AppState>,
) -> Result<Json<PublicBlogPost>, ApiError> {
    let post = sqlx::query_as::<_, BlogPostRow>(
        "SELECT b.id, b.title, b.slug, b.excerpt, b.image_url, b.content, b.tags, b.published_at, b.created_at,
                s.username AS author_username, s.first_name AS author_first_name, s.last_name AS author_last_name, s.avatar_url AS author_avatar
         FROM blog_posts b
         LEFT JOIN staff s ON b.author_id = s.id
         WHERE b.slug = $1 AND b.status = 'published'"
    )
    .bind(slug)
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound("Post not found".into()))?;

    Ok(Json(post.into_public()))
}
