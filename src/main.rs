mod auth;
mod db;
mod handlers;
mod models;

use axum::{
    routing::{get, post},
    Router,
};
use sqlx::SqlitePool;
use std::sync::Arc;
use tera::{Tera, Value};
use tower_http::{services::ServeDir, trace::TraceLayer};

pub struct AppState {
    auth: auth::AdminAuth,
    pool: SqlitePool,
    tera: Tera,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    let mut tera = Tera::new("templates/**/*.html")?;

    tera.register_filter(
        "str_slice",
        |value: &Value, args: &std::collections::HashMap<String, Value>| {
            let s = match value.as_str() {
                Some(s) => s,
                None => return Ok(Value::String("".to_string())),
            };
            let end = args
                .get("end")
                .and_then(|v| v.as_i64())
                .unwrap_or(s.len() as i64) as usize;
            Ok(Value::String(s.chars().take(end).collect()))
        },
    );

    let pool = SqlitePool::connect("sqlite:rustchan.db?mode=rwc").await?;
    db::init_db(&pool).await?;

    tokio::fs::create_dir_all("uploads").await?;

    let auth = auth::AdminAuth::from_env()?;
    let state = Arc::new(AppState { auth, pool, tera });
    let app = router(state);

    let port = std::env::var("PORT").unwrap_or_else(|_| "3000".to_string());
    let addr = format!("0.0.0.0:{port}");
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    tracing::info!("Listening on http://{addr}");
    axum::serve(listener, app).await?;
    Ok(())
}

fn router(state: Arc<AppState>) -> Router {
    let admin = Router::new()
        .route("/admin/dashboard", get(handlers::admin_dashboard))
        .route(
            "/admin/boards",
            get(handlers::admin_boards).post(handlers::admin_create_board),
        )
        .route(
            "/admin/boards/:slug/edit",
            get(handlers::admin_edit_board_form).post(handlers::admin_update_board),
        )
        .route(
            "/admin/boards/:slug/delete",
            post(handlers::admin_delete_board),
        )
        .route("/admin/posts", get(handlers::admin_posts))
        .route(
            "/admin/posts/:post_id/delete",
            post(handlers::admin_delete_post),
        )
        .route(
            "/admin/threads/:thread_id/lock",
            post(handlers::admin_lock_thread),
        )
        .route(
            "/admin/threads/:thread_id/move",
            post(handlers::admin_move_thread),
        )
        .route("/logout", post(handlers::admin_logout))
        .route_layer(axum::middleware::from_fn_with_state(
            state.clone(),
            auth::require_admin,
        ));
    Router::new()
        .merge(admin)
        .route("/", get(handlers::index))
        .route(
            "/login",
            get(handlers::login_page).post(handlers::admin_login),
        )
        .route("/:board/", get(handlers::board))
        .route("/:board/post", post(handlers::create_thread))
        .route("/:board/thread/:thread_id", get(handlers::thread))
        .route(
            "/:board/thread/:thread_id/reply",
            post(handlers::create_reply),
        )
        .nest_service("/static", ServeDir::new("static"))
        .nest_service("/uploads", ServeDir::new("uploads"))
        .layer(TraceLayer::new_for_http())
        .layer(axum::middleware::from_fn(security_headers))
        .with_state(state)
}

#[cfg(test)]
mod tests;

async fn security_headers(
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    let upload = request.uri().path().starts_with("/uploads/");
    let mut response = next.run(request).await;
    response
        .headers_mut()
        .insert("x-content-type-options", "nosniff".parse().unwrap());
    response
        .headers_mut()
        .insert("x-frame-options", "DENY".parse().unwrap());
    if upload {
        // Sandbox existing uploads too, including files uploaded before this fix.
        response.headers_mut().insert(
            "content-security-policy",
            "default-src 'none'; sandbox".parse().unwrap(),
        );
    }
    response
}
