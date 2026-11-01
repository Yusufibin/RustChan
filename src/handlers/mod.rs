use axum::{
    extract::{Multipart, Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::{Html, IntoResponse, Redirect},
    Extension, Form,
};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::sync::Arc;
use tera::Context;
use tokio::io::AsyncWriteExt;

use crate::auth::AdminSession;
use crate::db;
use crate::AppState;

const POSTS_PER_PAGE: i64 = 50;

fn generate_author_id(ip: &str, board_slug: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(format!("{}-{}-salt", ip, board_slug));
    let result = hasher.finalize();
    hex::encode(&result[..8])
}

fn format_content(content: &str) -> String {
    let mut html = html_escape::encode_text(content).to_string();

    html = html.replace("\r\n", "\n");

    let lines: Vec<&str> = html.split('\n').collect();
    let formatted_lines: Vec<String> = lines
        .iter()
        .map(|line| {
            if line.starts_with('>') {
                format!("<span class=\"greentext\">{}</span>", line)
            } else {
                line.to_string()
            }
        })
        .collect();
    html = formatted_lines.join("<br>");

    let quote_regex = regex::Regex::new(r"&gt;&gt;(\d+)").unwrap();
    html = quote_regex
        .replace_all(
            &html,
            "<a href=\"#p$1\" class=\"quote-link\">&gt;&gt;$1</a>",
        )
        .to_string();

    html
}

mod html_escape {
    pub fn encode_text(s: &str) -> String {
        s.replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
            .replace('"', "&quot;")
            .replace('\'', "&#39;")
    }
}

pub async fn index(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let boards = db::get_boards(&state.pool).await.unwrap_or_default();

    let mut context = Context::new();
    context.insert("boards", &boards);

    let html = state.tera.render("index.html", &context).unwrap();
    Html(html)
}

pub async fn login_page(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let html = state.tera.render("login.html", &Context::new()).unwrap();
    Html(html)
}

pub async fn board(
    State(state): State<Arc<AppState>>,
    Path(board_slug): Path<String>,
) -> impl IntoResponse {
    let boards = db::get_boards(&state.pool).await.unwrap_or_default();

    let board = match db::get_board(&state.pool, &board_slug).await {
        Ok(Some(b)) => b,
        _ => return (StatusCode::NOT_FOUND, "Board not found").into_response(),
    };

    let threads = db::get_threads(&state.pool, &board_slug)
        .await
        .unwrap_or_default();

    let mut context = Context::new();
    context.insert("boards", &boards);
    context.insert("board", &board);
    context.insert("threads", &threads);

    let html = state.tera.render("board.html", &context).unwrap();
    Html(html).into_response()
}

pub async fn thread(
    State(state): State<Arc<AppState>>,
    Path((board_slug, thread_id)): Path<(String, i64)>,
) -> impl IntoResponse {
    let boards = db::get_boards(&state.pool).await.unwrap_or_default();

    let board = match db::get_board(&state.pool, &board_slug).await {
        Ok(Some(b)) => b,
        _ => return (StatusCode::NOT_FOUND, "Board not found").into_response(),
    };

    let thread = match db::get_thread(&state.pool, thread_id).await {
        Ok(Some(t)) => t,
        _ => return (StatusCode::NOT_FOUND, "Thread not found").into_response(),
    };

    let mut context = Context::new();
    context.insert("boards", &boards);
    context.insert("board", &board);
    context.insert("thread", &thread);

    let html = state.tera.render("thread.html", &context).unwrap();
    Html(html).into_response()
}

pub async fn create_thread(
    State(state): State<Arc<AppState>>,
    Path(board_slug): Path<String>,
    mut multipart: Multipart,
) -> impl IntoResponse {
    let mut content = String::new();
    let mut image_path: Option<String> = None;
    let mut image_name: Option<String> = None;

    while let Some(field) = multipart.next_field().await.unwrap_or(None) {
        let name = field.name().unwrap_or("").to_string();

        match name.as_str() {
            "content" => {
                content = field.text().await.unwrap_or_default();
            }
            "image" => {
                let filename = field.file_name().map(|s| s.to_string());
                let data = field.bytes().await.unwrap_or_default();

                if !data.is_empty() {
                    if let Some(fname) = filename {
                        let Some(ext) = image_extension(&data) else {
                            return (
                                StatusCode::BAD_REQUEST,
                                "Only PNG, JPEG, GIF and WebP images are accepted.",
                            )
                                .into_response();
                        };
                        let new_name = format!("{}.{}", uuid::Uuid::new_v4(), ext);
                        let path = format!("uploads/{}", new_name);

                        if let Ok(mut file) = tokio::fs::File::create(&path).await {
                            let _ = file.write_all(&data).await;
                            image_path = Some(new_name);
                            image_name = Some(fname);
                        }
                    }
                }
            }
            _ => {}
        }
    }

    if content.trim().is_empty() && image_path.is_none() {
        return (StatusCode::BAD_REQUEST, "Post must have content or image").into_response();
    }

    let author_id = generate_author_id("127.0.0.1", &board_slug);

    match db::create_post(
        &state.pool,
        &board_slug,
        None,
        &author_id,
        &format_content(&content),
        image_path.as_deref(),
        image_name.as_deref(),
    )
    .await
    {
        Ok(_) => Redirect::to(&format!("/{}/", board_slug)).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

pub async fn create_reply(
    State(state): State<Arc<AppState>>,
    Path((board_slug, thread_id)): Path<(String, i64)>,
    mut multipart: Multipart,
) -> impl IntoResponse {
    let mut content = String::new();
    let mut image_path: Option<String> = None;
    let mut image_name: Option<String> = None;

    while let Some(field) = multipart.next_field().await.unwrap_or(None) {
        let name = field.name().unwrap_or("").to_string();

        match name.as_str() {
            "content" => {
                content = field.text().await.unwrap_or_default();
            }
            "image" => {
                let filename = field.file_name().map(|s| s.to_string());
                let data = field.bytes().await.unwrap_or_default();

                if !data.is_empty() {
                    if let Some(fname) = filename {
                        let Some(ext) = image_extension(&data) else {
                            return (
                                StatusCode::BAD_REQUEST,
                                "Only PNG, JPEG, GIF and WebP images are accepted.",
                            )
                                .into_response();
                        };
                        let new_name = format!("{}.{}", uuid::Uuid::new_v4(), ext);
                        let path = format!("uploads/{}", new_name);

                        if let Ok(mut file) = tokio::fs::File::create(&path).await {
                            let _ = file.write_all(&data).await;
                            image_path = Some(new_name);
                            image_name = Some(fname);
                        }
                    }
                }
            }
            _ => {}
        }
    }

    if content.trim().is_empty() && image_path.is_none() {
        return (StatusCode::BAD_REQUEST, "Post must have content or image").into_response();
    }

    let author_id = generate_author_id("127.0.0.1", &board_slug);

    match db::create_post(
        &state.pool,
        &board_slug,
        Some(thread_id),
        &author_id,
        &format_content(&content),
        image_path.as_deref(),
        image_name.as_deref(),
    )
    .await
    {
        Ok(_) => Redirect::to(&format!("/{}/thread/{}", board_slug, thread_id)).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

#[derive(Debug, Deserialize)]
pub struct Pagination {
    page: Option<i64>,
}

impl Default for Pagination {
    fn default() -> Self {
        Self { page: Some(1) }
    }
}

#[derive(Deserialize)]
pub struct LoginForm {
    password: String,
}

pub async fn admin_login(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Form(form): Form<LoginForm>,
) -> impl IntoResponse {
    // Browsers send Origin for form posts. Reject cross-origin login attempts;
    // command-line clients without an Origin header remain supported.
    if headers
        .get("sec-fetch-site")
        .is_some_and(|v| v == "cross-site")
    {
        return StatusCode::FORBIDDEN.into_response();
    }
    if let Some(origin) = headers.get("origin") {
        let origin = origin
            .to_str()
            .ok()
            .and_then(|v| v.parse::<axum::http::Uri>().ok());
        let host = headers.get("host").and_then(|v| v.to_str().ok());
        if !origin.as_ref().is_some_and(|uri| {
            matches!(uri.scheme_str(), Some("http" | "https"))
                && uri.authority().is_some_and(|authority| {
                    host.is_some_and(|host| authority.as_str().eq_ignore_ascii_case(host))
                })
        }) {
            return StatusCode::FORBIDDEN.into_response();
        }
    }
    match state.auth.login(&form.password) {
        Ok((_, cookie)) => {
            state.auth.logout(&headers);
            (
                StatusCode::SEE_OTHER,
                [
                    ("Location", "/admin/dashboard"),
                    ("Set-Cookie", cookie.as_str()),
                    ("Cache-Control", "no-store"),
                ],
            )
                .into_response()
        }
        Err(status) => (
            status,
            "Sign-in failed or temporarily unavailable. Please retry later.",
        )
            .into_response(),
    }
}

pub async fn admin_logout(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> impl IntoResponse {
    let cookie = state.auth.logout(&headers);
    (
        StatusCode::SEE_OTHER,
        [("Location", "/login"), ("Set-Cookie", cookie.as_str())],
    )
        .into_response()
}

pub async fn admin_dashboard(
    State(state): State<Arc<AppState>>,
    Extension(session): Extension<AdminSession>,
) -> impl IntoResponse {
    let boards = db::get_boards(&state.pool).await.unwrap_or_default();
    let stats = db::get_admin_stats(&state.pool).await.unwrap_or_default();

    let mut context = Context::new();
    context.insert("csrf_token", &session.csrf);
    context.insert("boards", &boards);
    context.insert("stats", &stats);

    let html = state.tera.render("admin/dashboard.html", &context).unwrap();
    Html(html).into_response()
}

pub async fn admin_boards(
    State(state): State<Arc<AppState>>,
    Extension(session): Extension<AdminSession>,
) -> impl IntoResponse {
    let boards = db::get_boards(&state.pool).await.unwrap_or_default();

    let mut context = Context::new();
    context.insert("csrf_token", &session.csrf);
    context.insert("boards", &boards);

    let html = state.tera.render("admin/boards.html", &context).unwrap();
    Html(html).into_response()
}

#[derive(Deserialize)]
pub struct BoardForm {
    #[serde(default)]
    slug: String,
    name: String,
    #[serde(default)]
    description: String,
}

pub async fn admin_create_board(
    State(state): State<Arc<AppState>>,
    Form(form): Form<BoardForm>,
) -> impl IntoResponse {
    if form.slug.is_empty()
        || !form
            .slug
            .bytes()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-')
        || form.name.trim().is_empty()
    {
        return (
            StatusCode::BAD_REQUEST,
            "A lowercase slug and a name are required.",
        )
            .into_response();
    }
    match db::create_board(&state.pool, &form.slug, &form.name, &form.description).await {
        Ok(_) => Redirect::to("/admin/boards").into_response(),
        Err(_) => (StatusCode::BAD_REQUEST, "Unable to create this board.").into_response(),
    }
}

pub async fn admin_delete_board(
    State(state): State<Arc<AppState>>,
    Path(slug): Path<String>,
) -> impl IntoResponse {
    match db::delete_board(&state.pool, &slug).await {
        Ok(_) => Redirect::to("/admin/boards").into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

pub async fn admin_edit_board_form(
    State(state): State<Arc<AppState>>,
    Extension(session): Extension<AdminSession>,
    Path(slug): Path<String>,
) -> impl IntoResponse {
    let board = match db::get_board(&state.pool, &slug).await {
        Ok(Some(b)) => b,
        _ => return (StatusCode::NOT_FOUND, "Board not found").into_response(),
    };

    let mut context = Context::new();
    context.insert("csrf_token", &session.csrf);
    context.insert("board", &board);

    let html = state
        .tera
        .render("admin/edit_board.html", &context)
        .unwrap();
    Html(html).into_response()
}

pub async fn admin_update_board(
    State(state): State<Arc<AppState>>,
    Path(slug): Path<String>,
    Form(form): Form<BoardForm>,
) -> impl IntoResponse {
    if form.name.trim().is_empty() {
        return (StatusCode::BAD_REQUEST, "Name is required").into_response();
    }
    match db::update_board(&state.pool, &slug, &form.name, &form.description).await {
        Ok(_) => Redirect::to("/admin/boards").into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

pub async fn admin_posts(
    State(state): State<Arc<AppState>>,
    Extension(session): Extension<AdminSession>,
    Query(pagination): Query<Pagination>,
) -> impl IntoResponse {
    let page = pagination.page.unwrap_or(1).max(1);
    let offset = (page - 1) * POSTS_PER_PAGE;

    let posts = db::get_all_posts(&state.pool, POSTS_PER_PAGE, offset)
        .await
        .unwrap_or_default();
    let total_posts = db::get_post_count(&state.pool).await.unwrap_or(0);
    let total_pages = (total_posts as f64 / POSTS_PER_PAGE as f64).ceil() as i64;

    let mut context = Context::new();
    context.insert("csrf_token", &session.csrf);
    context.insert("posts", &posts);
    context.insert("current_page", &page);
    context.insert("total_pages", &total_pages);

    let html = state.tera.render("admin/posts.html", &context).unwrap();
    Html(html).into_response()
}

pub async fn admin_delete_post(
    State(state): State<Arc<AppState>>,
    Path(post_id): Path<i64>,
) -> impl IntoResponse {
    match db::delete_post(&state.pool, post_id).await {
        Ok(_) => Redirect::to("/admin/posts").into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

pub async fn admin_lock_thread(
    State(state): State<Arc<AppState>>,
    Path(thread_id): Path<i64>,
) -> impl IntoResponse {
    match db::lock_thread(&state.pool, thread_id).await {
        Ok(_) => Redirect::to("/admin/posts").into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

#[derive(Debug, Deserialize)]
pub struct MoveQuery {
    target_board: String,
}

pub async fn admin_move_thread(
    State(state): State<Arc<AppState>>,
    Path(thread_id): Path<i64>,
    Form(query): Form<MoveQuery>,
) -> impl IntoResponse {
    match db::move_thread(&state.pool, thread_id, &query.target_board).await {
        Ok(_) => Redirect::to("/admin/posts").into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

// Use a server-selected extension, never a user-controlled active document type.
fn image_extension(data: &[u8]) -> Option<&'static str> {
    if data.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("png")
    } else if data.starts_with(b"\xff\xd8\xff") {
        Some("jpg")
    } else if data.starts_with(b"GIF87a") || data.starts_with(b"GIF89a") {
        Some("gif")
    } else if data.starts_with(b"RIFF") && data.get(8..12) == Some(b"WEBP") {
        Some("webp")
    } else {
        None
    }
}

#[cfg(test)]
mod upload_tests {
    use super::image_extension;
    #[test]
    fn rejects_active_documents_and_uses_image_signatures() {
        assert_eq!(image_extension(b"<html><script>alert(1)</script>"), None);
        assert_eq!(image_extension(b"<svg onload='alert(1)'>"), None);
        assert_eq!(image_extension(b"\x89PNG\r\n\x1a\nrest"), Some("png"));
        assert_eq!(image_extension(b"\xff\xd8\xffrest"), Some("jpg"));
        assert_eq!(image_extension(b"GIF89a..."), Some("gif"));
        assert_eq!(image_extension(b"RIFF....WEBP"), Some("webp"));
    }
}
