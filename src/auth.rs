use std::{
    collections::{HashMap, VecDeque},
    sync::Mutex,
    time::{Duration, Instant},
};

use axum::{
    body::{to_bytes, Body},
    extract::{Request, State},
    http::{header, HeaderMap, Method, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
};
use rand::{rngs::OsRng, RngCore};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

const COOKIE: &str = "admin_session";
const SESSION_TTL: Duration = Duration::from_secs(8 * 60 * 60);
const MAX_SESSIONS: usize = 64;
const LOGIN_WINDOW: Duration = Duration::from_secs(60);
const MAX_LOGIN_ATTEMPTS: usize = 20;

#[derive(Clone)]
pub struct AdminSession {
    pub csrf: String,
    expires: Instant,
}

pub struct AdminAuth {
    password_hash: Option<[u8; 32]>,
    secure_cookies: bool,
    sessions: Mutex<HashMap<String, AdminSession>>,
    attempts: Mutex<VecDeque<Instant>>,
}

impl AdminAuth {
    pub fn from_env() -> anyhow::Result<Self> {
        let password = match std::env::var("RUSTCHAN_ADMIN_PASSWORD") {
            Ok(value) => Some(value),
            Err(std::env::VarError::NotPresent) => None,
            Err(error) => return Err(error.into()),
        };
        let secure = match std::env::var("RUSTCHAN_SECURE_COOKIES").as_deref() {
            Ok("false") => false,
            Ok("true") | Err(std::env::VarError::NotPresent) => true,
            _ => anyhow::bail!("RUSTCHAN_SECURE_COOKIES must be true or false"),
        };
        if password.is_none() {
            tracing::warn!("Administration disabled: set RUSTCHAN_ADMIN_PASSWORD to enable it");
        }
        Self::new(password.as_deref(), secure)
    }

    pub fn new(password: Option<&str>, secure_cookies: bool) -> anyhow::Result<Self> {
        if let Some(value) = password {
            anyhow::ensure!(
                value.chars().count() >= 16 && !value.trim().is_empty(),
                "RUSTCHAN_ADMIN_PASSWORD must contain at least 16 characters"
            );
        }
        Ok(Self {
            password_hash: password.map(|value| Sha256::digest(value.as_bytes()).into()),
            secure_cookies,
            sessions: Mutex::new(HashMap::new()),
            attempts: Mutex::new(VecDeque::new()),
        })
    }

    pub fn login(&self, password: &str) -> Result<(String, String), StatusCode> {
        let expected = self.password_hash.ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
        let now = Instant::now();
        let mut attempts = self.attempts.lock().expect("login lock poisoned");
        while attempts
            .front()
            .is_some_and(|at| now.duration_since(*at) >= LOGIN_WINDOW)
        {
            attempts.pop_front();
        }
        if attempts.len() >= MAX_LOGIN_ATTEMPTS {
            return Err(StatusCode::TOO_MANY_REQUESTS);
        }
        attempts.push_back(now);
        drop(attempts);
        let actual: [u8; 32] = Sha256::digest(password.as_bytes()).into();
        if !bool::from(expected.ct_eq(&actual)) {
            return Err(StatusCode::UNAUTHORIZED);
        }
        let token = random_token();
        let session = AdminSession {
            csrf: random_token(),
            expires: now + SESSION_TTL,
        };
        let mut sessions = self.sessions.lock().expect("session lock poisoned");
        sessions.retain(|_, value| value.expires > now);
        if sessions.len() >= MAX_SESSIONS {
            if let Some(oldest) = sessions
                .iter()
                .min_by_key(|(_, value)| value.expires)
                .map(|(key, _)| key.clone())
            {
                sessions.remove(&oldest);
            }
        }
        sessions.insert(token.clone(), session);
        let cookie = self.cookie(&token, SESSION_TTL.as_secs());
        Ok((token, cookie))
    }

    pub fn session(&self, headers: &HeaderMap) -> Option<AdminSession> {
        let token = session_token(headers)?;
        let mut sessions = self.sessions.lock().ok()?;
        let now = Instant::now();
        sessions.retain(|_, session| session.expires > now);
        sessions.get(&token).cloned()
    }

    pub fn logout(&self, headers: &HeaderMap) -> String {
        if let Some(token) = session_token(headers) {
            self.sessions
                .lock()
                .expect("session lock poisoned")
                .remove(&token);
        }
        self.cookie("", 0)
    }

    fn cookie(&self, token: &str, age: u64) -> String {
        format!(
            "{COOKIE}={token}; Path=/; HttpOnly; SameSite=Strict; Max-Age={age}{}",
            if self.secure_cookies { "; Secure" } else { "" }
        )
    }
}

fn random_token() -> String {
    let mut bytes = [0_u8; 32];
    OsRng.fill_bytes(&mut bytes);
    hex::encode(bytes)
}

fn session_token(headers: &HeaderMap) -> Option<String> {
    let mut found = None;
    for header in headers.get_all(header::COOKIE) {
        for cookie in header.to_str().ok()?.split(';') {
            if let Some((name, value)) = cookie.trim().split_once('=') {
                if name == COOKIE {
                    if found.is_some()
                        || value.len() != 64
                        || !value.bytes().all(|b| b.is_ascii_hexdigit())
                    {
                        return None;
                    }
                    found = Some(value.to_owned());
                }
            }
        }
    }
    found
}

pub async fn require_admin(
    State(state): State<std::sync::Arc<crate::AppState>>,
    mut request: Request,
    next: Next,
) -> Response {
    let Some(session) = state.auth.session(request.headers()) else {
        return (
            StatusCode::UNAUTHORIZED,
            "Please sign in to administer RustChan.",
        )
            .into_response();
    };
    if request.method() != Method::GET && request.method() != Method::HEAD {
        let is_form = request
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| v.split(';').next() == Some("application/x-www-form-urlencoded"));
        if !is_form {
            return (StatusCode::FORBIDDEN, "A valid form token is required.").into_response();
        }
        let (parts, body) = request.into_parts();
        let Ok(bytes) = to_bytes(body, 16 * 1024).await else {
            return StatusCode::PAYLOAD_TOO_LARGE.into_response();
        };
        let fields: Vec<(String, String)> = match serde_urlencoded::from_bytes(&bytes) {
            Ok(fields) => fields,
            Err(_) => return StatusCode::BAD_REQUEST.into_response(),
        };
        let tokens: Vec<_> = fields
            .iter()
            .filter(|(name, _)| name == "csrf_token")
            .collect();
        if tokens.len() != 1 || !bool::from(session.csrf.as_bytes().ct_eq(tokens[0].1.as_bytes())) {
            return (
                StatusCode::FORBIDDEN,
                "Invalid form token. Reload the page and retry.",
            )
                .into_response();
        }
        request = Request::from_parts(parts, Body::from(bytes));
    }
    request.extensions_mut().insert(session);
    let mut response = next.run(request).await;
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, "no-store".parse().unwrap());
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_forged_ambiguous_expired_and_revoked_sessions() {
        let auth = AdminAuth::new(Some("a-long-test-password"), true).unwrap();
        let (token, cookie) = auth.login("a-long-test-password").unwrap();
        assert!(cookie.contains("HttpOnly; SameSite=Strict"));
        assert!(cookie.ends_with("; Secure"));
        let mut headers = HeaderMap::new();
        headers.insert(
            header::COOKIE,
            format!("admin_session={}", "a".repeat(64)).parse().unwrap(),
        );
        assert!(auth.session(&headers).is_none());
        headers.insert(
            header::COOKIE,
            format!("admin_session={token}").parse().unwrap(),
        );
        assert!(auth.session(&headers).is_some());
        headers.append(
            header::COOKIE,
            format!("admin_session={token}").parse().unwrap(),
        );
        assert!(auth.session(&headers).is_none());
        headers.remove(header::COOKIE);
        headers.insert(
            header::COOKIE,
            format!("admin_session={token}").parse().unwrap(),
        );
        auth.sessions
            .lock()
            .unwrap()
            .get_mut(&token)
            .unwrap()
            .expires = Instant::now() - Duration::from_secs(1);
        assert!(auth.session(&headers).is_none());
        let (new_token, _) = auth.login("a-long-test-password").unwrap();
        headers.insert(
            header::COOKIE,
            format!("admin_session={new_token}").parse().unwrap(),
        );
        assert!(auth.logout(&headers).contains("Max-Age=0"));
        assert!(auth.session(&headers).is_none());
    }

    #[test]
    fn disables_unconfigured_admin_and_limits_login_attempts() {
        assert!(AdminAuth::new(Some("admin123"), true).is_err());
        let disabled = AdminAuth::new(None, true).unwrap();
        assert_eq!(
            disabled.login("anything").unwrap_err(),
            StatusCode::SERVICE_UNAVAILABLE
        );
        let auth = AdminAuth::new(Some("a-long-test-password"), false).unwrap();
        for _ in 0..MAX_LOGIN_ATTEMPTS {
            assert_eq!(auth.login("wrong").unwrap_err(), StatusCode::UNAUTHORIZED);
        }
        assert_eq!(
            auth.login("a-long-test-password").unwrap_err(),
            StatusCode::TOO_MANY_REQUESTS
        );
        auth.attempts.lock().unwrap().clear();
        assert!(!auth
            .login("a-long-test-password")
            .unwrap()
            .1
            .contains("Secure"));
    }
}
