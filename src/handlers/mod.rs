use axum::{
    extract::{Multipart, Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::{Html, IntoResponse, Redirect},
};
use sha2::{Digest, Sha256};
use serde::Deserialize;
use std::sync::Arc;
use tera::Context;
use tokio::io::AsyncWriteExt;
use uuid::Uuid;

use crate::db;
use crate::AppState;

const ADMIN_PASSWORD: &str = "admin123";
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
        .replace_all(&html, "<a href=\"#p$1\" class=\"quote-link\">&gt;&gt;$1</a>")
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
    
    let threads = db::get_threads(&state.pool, &board_slug).await.unwrap_or_default();
    
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
                        let ext = fname.rsplit('.').next().unwrap_or("png");
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
                        let ext = fname.rsplit('.').next().unwrap_or("png");
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

fn check_admin_auth(headers: &HeaderMap) -> bool {
    if let Some(cookie_header) = headers.get("cookie") {
        if let Ok(cookie_str) = cookie_header.to_str() {
            return cookie_str.contains("admin_session=");
        }
    }
    false
}

pub async fn admin_login(
    State(_state): State<Arc<AppState>>,
    mut multipart: Multipart,
) -> impl IntoResponse {
    let mut password = String::new();
    
    while let Some(field) = multipart.next_field().await.unwrap_or(None) {
        let name = field.name().unwrap_or("").to_string();
        if name == "password" {
            password = field.text().await.unwrap_or_default();
        }
    }
    
    if password == ADMIN_PASSWORD {
        let token = Uuid::new_v4().to_string();
        let cookie = format!("admin_session={}; Path=/; HttpOnly; Max-Age=86400", token);
        (
            StatusCode::SEE_OTHER,
            [("Location", "/admin/dashboard"), ("Set-Cookie", cookie.as_str())],
        ).into_response()
    } else {
        (StatusCode::UNAUTHORIZED, "Invalid password").into_response()
    }
}

pub async fn admin_dashboard(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> impl IntoResponse {
    if !check_admin_auth(&headers) {
        return (StatusCode::UNAUTHORIZED, "Unauthorized").into_response();
    }
    
    let boards = db::get_boards(&state.pool).await.unwrap_or_default();
    let stats = db::get_admin_stats(&state.pool).await.unwrap_or_default();
    
    let mut context = Context::new();
    context.insert("boards", &boards);
    context.insert("stats", &stats);
    
    let html = state.tera.render("admin/dashboard.html", &context).unwrap();
    Html(html).into_response()
}

pub async fn admin_boards(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> impl IntoResponse {
    if !check_admin_auth(&headers) {
        return (StatusCode::UNAUTHORIZED, "Unauthorized").into_response();
    }
    
    let boards = db::get_boards(&state.pool).await.unwrap_or_default();
    
    let mut context = Context::new();
    context.insert("boards", &boards);
    
    let html = state.tera.render("admin/boards.html", &context).unwrap();
    Html(html).into_response()
}

pub async fn admin_create_board(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    mut multipart: Multipart,
) -> impl IntoResponse {
    if !check_admin_auth(&headers) {
        return (StatusCode::UNAUTHORIZED, "Unauthorized").into_response();
    }
    
    let mut slug = String::new();
    let mut name = String::new();
    let mut description = String::new();
    
    while let Some(field) = multipart.next_field().await.unwrap_or(None) {
        let field_name = field.name().unwrap_or("").to_string();
        match field_name.as_str() {
            "slug" => slug = field.text().await.unwrap_or_default(),
            "name" => name = field.text().await.unwrap_or_default(),
            "description" => description = field.text().await.unwrap_or_default(),
            _ => {}
        }
    }
    
    if slug.is_empty() || name.is_empty() {
        return (StatusCode::BAD_REQUEST, "Slug and name are required").into_response();
    }
    
    match db::create_board(&state.pool, &slug, &name, &description).await {
        Ok(_) => Redirect::to("/admin/boards").into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

pub async fn admin_delete_board(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(slug): Path<String>,
) -> impl IntoResponse {
    if !check_admin_auth(&headers) {
        return (StatusCode::UNAUTHORIZED, "Unauthorized").into_response();
    }
    
    match db::delete_board(&state.pool, &slug).await {
        Ok(_) => Redirect::to("/admin/boards").into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

pub async fn admin_edit_board_form(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(slug): Path<String>,
) -> impl IntoResponse {
    if !check_admin_auth(&headers) {
        return (StatusCode::UNAUTHORIZED, "Unauthorized").into_response();
    }
    
    let board = match db::get_board(&state.pool, &slug).await {
        Ok(Some(b)) => b,
        _ => return (StatusCode::NOT_FOUND, "Board not found").into_response(),
    };
    
    let mut context = Context::new();
    context.insert("board", &board);
    
    let html = state.tera.render("admin/edit_board.html", &context).unwrap();
    Html(html).into_response()
}

pub async fn admin_update_board(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(slug): Path<String>,
    mut multipart: Multipart,
) -> impl IntoResponse {
    if !check_admin_auth(&headers) {
        return (StatusCode::UNAUTHORIZED, "Unauthorized").into_response();
    }
    
    let mut name = String::new();
    let mut description = String::new();
    
    while let Some(field) = multipart.next_field().await.unwrap_or(None) {
        let field_name = field.name().unwrap_or("").to_string();
        match field_name.as_str() {
            "name" => name = field.text().await.unwrap_or_default(),
            "description" => description = field.text().await.unwrap_or_default(),
            _ => {}
        }
    }
    
    if name.is_empty() {
        return (StatusCode::BAD_REQUEST, "Name is required").into_response();
    }
    
    match db::update_board(&state.pool, &slug, &name, &description).await {
        Ok(_) => Redirect::to("/admin/boards").into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

pub async fn admin_posts(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(pagination): Query<Pagination>,
) -> impl IntoResponse {
    if !check_admin_auth(&headers) {
        return (StatusCode::UNAUTHORIZED, "Unauthorized").into_response();
    }
    
    let page = pagination.page.unwrap_or(1).max(1);
    let offset = (page - 1) * POSTS_PER_PAGE;
    
    let posts = db::get_all_posts(&state.pool, POSTS_PER_PAGE, offset).await.unwrap_or_default();
    let total_posts = db::get_post_count(&state.pool).await.unwrap_or(0);
    let total_pages = (total_posts as f64 / POSTS_PER_PAGE as f64).ceil() as i64;
    
    let mut context = Context::new();
    context.insert("posts", &posts);
    context.insert("current_page", &page);
    context.insert("total_pages", &total_pages);
    
    let html = state.tera.render("admin/posts.html", &context).unwrap();
    Html(html).into_response()
}

pub async fn admin_delete_post(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(post_id): Path<i64>,
) -> impl IntoResponse {
    if !check_admin_auth(&headers) {
        return (StatusCode::UNAUTHORIZED, "Unauthorized").into_response();
    }
    
    match db::delete_post(&state.pool, post_id).await {
        Ok(_) => Redirect::to("/admin/posts").into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

pub async fn admin_lock_thread(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(thread_id): Path<i64>,
) -> impl IntoResponse {
    if !check_admin_auth(&headers) {
        return (StatusCode::UNAUTHORIZED, "Unauthorized").into_response();
    }
    
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
    headers: HeaderMap,
    Path(thread_id): Path<i64>,
    Query(query): Query<MoveQuery>,
) -> impl IntoResponse {
    if !check_admin_auth(&headers) {
        return (StatusCode::UNAUTHORIZED, "Unauthorized").into_response();
    }
    
    match db::move_thread(&state.pool, thread_id, &query.target_board).await {
        Ok(_) => Redirect::to("/admin/posts").into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}
