use axum::extract::{Path, State};
use axum::response::Html;
use axum::routing::get;
use axum::Router;

use crate::state::AppState;
use vaultscope_db::models::Partner;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/ps/{slug}", get(get_partner_meta))
        .route("/resources/blog/{slug}", get(get_blog_meta))
        .route("/blog/{slug}", get(get_blog_meta))
}

fn escape_html(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

async fn get_partner_meta(
    State(state): State<AppState>,
    Path(slug): Path<String>,
) -> Html<String> {
    let clean_slug = slug.trim().to_lowercase();
    let partner: Option<Partner> = sqlx::query_as(
        "SELECT * FROM partners WHERE LOWER(slug) = LOWER($1) AND active = true",
    )
    .bind(&clean_slug)
    .fetch_optional(&state.db)
    .await
    .unwrap_or(None);

    let (title, description, canonical_url, image) = match partner {
        Some(p) => {
            let trial_days = p.trial_days_override.unwrap_or(60);
            let trial_quota = p.trial_quota_gb_override.unwrap_or(25);
            let title = format!("{} × VaultScope — Exclusive Partner Offer", p.name);
            let description = p
                .description
                .clone()
                .or(p.headline.clone())
                .unwrap_or_else(|| {
                    format!(
                        "Claim your free {}-day {}GB SFTP storage trial sponsored by {}. Enterprise performance with EU data sovereignty.",
                        trial_days, trial_quota, p.name
                    )
                });
            let canonical_url = format!("https://vaultscope.de/ps/{}", p.slug);
            let image = "https://vaultscope.de/og-image.png".to_string();
            (title, description, canonical_url, image)
        }
        None => (
            "Exclusive Partner Offer — VaultScope".to_string(),
            "Claim exclusive high-performance cloud infrastructure and SFTP storage offers with VaultScope."
                .to_string(),
            format!("https://vaultscope.de/ps/{}", clean_slug),
            "https://vaultscope.de/og-image.png".to_string(),
        ),
    };

    Html(render_card(
        &title,
        &description,
        &canonical_url,
        &image,
        "website",
        None,
    ))
}

#[derive(sqlx::FromRow)]
struct BlogMetaRow {
    title: String,
    slug: String,
    excerpt: String,
    image_url: Option<String>,
    published_at: Option<chrono::DateTime<chrono::Utc>>,
}

async fn get_blog_meta(
    State(state): State<AppState>,
    Path(slug): Path<String>,
) -> Html<String> {
    let clean_slug = slug.trim();
    let post: Option<BlogMetaRow> = sqlx::query_as(
        "SELECT title, slug, excerpt, image_url, published_at FROM blog_posts WHERE slug = $1 AND status = 'published'",
    )
    .bind(clean_slug)
    .fetch_optional(&state.db)
    .await
    .unwrap_or(None);

    let (title, description, canonical_url, image, published_at) = match post {
        Some(p) => {
            let title = format!("{} — VaultScope Blog", p.title);
            let description = if !p.excerpt.trim().is_empty() {
                p.excerpt
            } else {
                "Read the latest technical insights, infrastructure updates, and engineering deep dives from VaultScope."
                    .to_string()
            };
            let canonical_url = format!("https://vaultscope.de/resources/blog/{}", p.slug);
            let image = p
                .image_url
                .filter(|img| !img.trim().is_empty())
                .unwrap_or_else(|| "https://vaultscope.de/og-image.png".to_string());
            (title, description, canonical_url, image, p.published_at)
        }
        None => (
            "VaultScope Engineering Blog".to_string(),
            "Technical insights, engineering articles, and cloud infrastructure guides from the VaultScope team."
                .to_string(),
            format!("https://vaultscope.de/resources/blog/{}", clean_slug),
            "https://vaultscope.de/og-image.png".to_string(),
            None,
        ),
    };

    Html(render_card(
        &title,
        &description,
        &canonical_url,
        &image,
        "article",
        published_at.map(|t| t.to_rfc3339()).as_deref(),
    ))
}

fn render_card(
    title: &str,
    description: &str,
    canonical_url: &str,
    image: &str,
    og_type: &str,
    published_time: Option<&str>,
) -> String {
    let esc_title = escape_html(title);
    let esc_desc = escape_html(description);
    let esc_url = escape_html(canonical_url);
    let esc_image = escape_html(image);
    let article_meta = match published_time {
        Some(t) => {
            format!("<meta property=\"article:published_time\" content=\"{}\" />\n", escape_html(t))
        }
        None => String::new(),
    };

    format!(
        r##"<!doctype html>
<html lang="en">
<head>
  <meta charset="UTF-8" />
  <title>{esc_title}</title>
  <meta name="description" content="{esc_desc}" />
  <link rel="canonical" href="{esc_url}" />

  <!-- Theme color for Discord left border -->
  <meta name="theme-color" content="#2563eb" />
  <meta name="msapplication-TileColor" content="#2563eb" />

  <!-- Open Graph -->
  <meta property="og:site_name" content="VaultScope" />
  <meta property="og:type" content="{og_type}" />
  <meta property="og:url" content="{esc_url}" />
  <meta property="og:title" content="{esc_title}" />
  <meta property="og:description" content="{esc_desc}" />
  <meta property="og:image" content="{esc_image}" />
  <meta property="og:image:width" content="1200" />
  <meta property="og:image:height" content="630" />
  {article_meta}
  <!-- Twitter / X -->
  <meta name="twitter:card" content="summary_large_image" />
  <meta name="twitter:site" content="@vaultscope" />
  <meta name="twitter:title" content="{esc_title}" />
  <meta name="twitter:description" content="{esc_desc}" />
  <meta name="twitter:image" content="{esc_image}" />

  <meta name="robots" content="index, follow" />
</head>
<body>
  <h1>{esc_title}</h1>
  <p>{esc_desc}</p>
  <script>window.location.replace("{esc_url}");</script>
</body>
</html>"##
    )
}
