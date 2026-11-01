use std::sync::Arc;

use axum::{
    body::{to_bytes, Body},
    http::{header, Request, StatusCode},
    Router,
};
use tower::ServiceExt;

const PASSWORD: &str = "integration-test-password";

async fn app() -> Router {
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    crate::db::init_db(&pool).await.unwrap();
    let mut tera = tera::Tera::new("templates/**/*.html").unwrap();
    tera.register_filter(
        "str_slice",
        |value: &tera::Value, args: &std::collections::HashMap<String, tera::Value>| {
            let end = args.get("end").and_then(|v| v.as_u64()).unwrap_or(80) as usize;
            Ok(tera::Value::String(
                value.as_str().unwrap_or("").chars().take(end).collect(),
            ))
        },
    );
    crate::router(Arc::new(crate::AppState {
        pool,
        tera,
        auth: crate::auth::AdminAuth::new(Some(PASSWORD), false).unwrap(),
    }))
}

async fn request(
    app: &Router,
    method: &str,
    path: &str,
    cookie: &str,
    body: &str,
) -> axum::response::Response {
    let mut request = Request::builder()
        .method(method)
        .uri(path)
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded");
    if !cookie.is_empty() {
        request = request.header(header::COOKIE, cookie);
    }
    app.clone()
        .oneshot(request.body(Body::from(body.to_owned())).unwrap())
        .await
        .unwrap()
}

async fn sign_in(app: &Router) -> (String, String) {
    let response = request(app, "POST", "/login", "", &format!("password={PASSWORD}")).await;
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    let cookie = response.headers()[header::SET_COOKIE]
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_owned();
    let response = request(app, "GET", "/admin/boards", &cookie, "").await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
    let body = String::from_utf8(
        to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap()
            .to_vec(),
    )
    .unwrap();
    let csrf = body
        .split("name=\"csrf_token\" value=\"")
        .nth(1)
        .unwrap()
        .split('"')
        .next()
        .unwrap()
        .to_owned();
    (cookie, csrf)
}

#[tokio::test]
async fn forged_cookies_cannot_read_or_mutate_admin_routes() {
    let app = app().await;
    for cookie in [
        "",
        "admin_session=anything",
        &format!("admin_session={}", "a".repeat(64)),
        "other_admin_session=anything",
    ] {
        for path in [
            "/admin/dashboard",
            "/admin/boards",
            "/admin/posts",
            "/admin/boards/g/edit",
        ] {
            assert_eq!(
                request(&app, "GET", path, cookie, "").await.status(),
                StatusCode::UNAUTHORIZED,
                "{path}"
            );
        }
        for path in [
            "/admin/boards",
            "/admin/boards/g/edit",
            "/admin/boards/g/delete",
            "/admin/posts/1/delete",
            "/admin/threads/1/lock",
            "/admin/threads/1/move",
            "/logout",
        ] {
            assert_eq!(
                request(&app, "POST", path, cookie, "csrf_token=forged")
                    .await
                    .status(),
                StatusCode::UNAUTHORIZED,
                "{path}"
            );
        }
    }
    assert_eq!(
        request(&app, "POST", "/login", "", "password=wrong")
            .await
            .status(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        request(&app, "GET", "/", "", "").await.status(),
        StatusCode::OK
    );
}

#[tokio::test]
async fn mutations_require_post_and_the_current_sessions_csrf_token() {
    let app = app().await;
    let (cookie, csrf) = sign_in(&app).await;
    let (_, other_csrf) = sign_in(&app).await;
    for path in [
        "/admin/boards/g/delete",
        "/admin/posts/1/delete",
        "/admin/threads/1/lock",
        "/admin/threads/1/move",
        "/logout",
    ] {
        assert_eq!(
            request(&app, "GET", path, &cookie, "").await.status(),
            StatusCode::METHOD_NOT_ALLOWED
        );
        assert_eq!(
            request(&app, "POST", path, &cookie, "").await.status(),
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            request(
                &app,
                "POST",
                path,
                &cookie,
                &format!("csrf_token={other_csrf}")
            )
            .await
            .status(),
            StatusCode::FORBIDDEN
        );
    }
    let form = format!("csrf_token={csrf}&slug=new-board&name=New+board&description=Test");
    assert_eq!(
        request(&app, "POST", "/admin/boards", &cookie, &form)
            .await
            .status(),
        StatusCode::SEE_OTHER
    );
    assert_eq!(
        request(&app, "GET", "/new-board/", "", "").await.status(),
        StatusCode::OK
    );
    assert_eq!(
        request(&app, "GET", "/admin/boards/new-board/edit", &cookie, "")
            .await
            .status(),
        StatusCode::OK
    );
    assert_eq!(
        request(
            &app,
            "POST",
            "/admin/boards/new-board/edit",
            &cookie,
            &format!("csrf_token={csrf}&name=Updated")
        )
        .await
        .status(),
        StatusCode::SEE_OTHER
    );
    assert_eq!(
        request(
            &app,
            "POST",
            "/admin/boards/new-board/delete",
            &cookie,
            &format!("csrf_token={csrf}")
        )
        .await
        .status(),
        StatusCode::SEE_OTHER
    );
    assert_eq!(
        request(&app, "GET", "/new-board/", "", "").await.status(),
        StatusCode::NOT_FOUND
    );
    let response = request(
        &app,
        "POST",
        "/logout",
        &cookie,
        &format!("csrf_token={csrf}"),
    )
    .await;
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    assert!(response.headers()[header::SET_COOKIE]
        .to_str()
        .unwrap()
        .contains("Max-Age=0"));
    assert_eq!(
        request(&app, "GET", "/admin/dashboard", &cookie, "")
            .await
            .status(),
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
async fn browser_login_rejects_cross_origin_forms() {
    let app = app().await;
    for origin in ["https://another-site.example", "null"] {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/login")
                    .header("host", "rustchan.example")
                    .header("origin", origin)
                    .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                    .body(Body::from(format!("password={PASSWORD}")))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }
}

#[tokio::test]
async fn uploads_are_sandboxed_even_when_the_file_is_missing() {
    let app = app().await;
    let response = request(&app, "GET", "/uploads/legacy.html", "", "").await;
    assert_eq!(response.headers()["x-content-type-options"], "nosniff");
    assert_eq!(
        response.headers()["content-security-policy"],
        "default-src 'none'; sandbox"
    );
}
