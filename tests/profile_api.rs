#[allow(dead_code)]
mod support;

use axum::{
    Router,
    body::Body,
    http::{
        Method, Request, StatusCode,
        header::{CACHE_CONTROL, CONTENT_TYPE, COOKIE, HOST, ORIGIN, PRAGMA},
    },
};
use http_body_util::BodyExt;
use raindrop::{
    app::{AppState, build_router},
    auth::build_session_cookie,
    db::{DatabaseConfig, connect, migrate},
    setup::SetupService,
};
use secrecy::{ExposeSecret, SecretString};
use serde_json::{Value, json};
use support::database::{USER_A_ID, USER_B_ID, insert_user};
use tempfile::TempDir;
use tower::ServiceExt;

struct ProfileFixture {
    _data: TempDir,
    app: Router,
    user_a_cookie: String,
    user_a_csrf: String,
    user_b_cookie: String,
    user_b_csrf: String,
}

#[derive(Clone, Copy)]
enum UserKind {
    A,
    B,
}

impl ProfileFixture {
    async fn new() -> Self {
        let data = tempfile::tempdir().expect("temporary directory should be created");
        let url = format!(
            "sqlite://{}?mode=rwc",
            data.path().join("profile-api.db").display()
        );
        let database = connect(&DatabaseConfig::new(SecretString::from(url)))
            .await
            .expect("profile API database should connect");
        migrate(&database)
            .await
            .expect("profile API database should migrate");
        insert_user(&database, USER_A_ID, "profile-a").await;
        insert_user(&database, USER_B_ID, "profile-b").await;

        let setup = SetupService::ready(data.path(), None, database);
        let session_a = setup
            .sessions()
            .create(USER_A_ID)
            .await
            .expect("user A session should create");
        let session_b = setup
            .sessions()
            .create(USER_B_ID)
            .await
            .expect("user B session should create");
        let app = build_router(AppState::new(setup));
        Self {
            _data: data,
            app,
            user_a_cookie: session_cookie(&session_a),
            user_a_csrf: session_a.csrf_token.expose_secret().to_owned(),
            user_b_cookie: session_cookie(&session_b),
            user_b_csrf: session_b.csrf_token.expose_secret().to_owned(),
        }
    }

    async fn request(
        &self,
        method: Method,
        body: Option<&str>,
        user: Option<UserKind>,
        include_csrf: bool,
        content_type: bool,
    ) -> CapturedResponse {
        let mut request = Request::builder().method(method).uri("/api/v2/profile");
        if let Some(user) = user {
            let (cookie, csrf) = match user {
                UserKind::A => (&self.user_a_cookie, &self.user_a_csrf),
                UserKind::B => (&self.user_b_cookie, &self.user_b_csrf),
            };
            request = request.header(COOKIE, cookie);
            if include_csrf {
                request = request
                    .header("x-csrf-token", csrf)
                    .header(ORIGIN, "http://profile.test")
                    .header(HOST, "profile.test");
            }
        }
        if content_type {
            request = request.header(CONTENT_TYPE, "application/json");
        }
        let response = self
            .app
            .clone()
            .oneshot(
                request
                    .body(body.map_or_else(Body::empty, |value| Body::from(value.to_owned())))
                    .expect("profile request should build"),
            )
            .await
            .expect("profile request should complete");
        CapturedResponse::from_response(response).await
    }

    async fn json_request(
        &self,
        method: Method,
        body: Option<Value>,
        user: Option<UserKind>,
        include_csrf: bool,
    ) -> CapturedResponse {
        let body = body.map(|value| value.to_string());
        self.request(method, body.as_deref(), user, include_csrf, body.is_some())
            .await
    }
}

struct CapturedResponse {
    status: StatusCode,
    headers: axum::http::HeaderMap,
    body: Vec<u8>,
}

impl CapturedResponse {
    async fn from_response(response: axum::response::Response) -> Self {
        let (parts, body) = response.into_parts();
        Self {
            status: parts.status,
            headers: parts.headers,
            body: body
                .collect()
                .await
                .expect("profile response body should collect")
                .to_bytes()
                .to_vec(),
        }
    }

    fn json(&self) -> Value {
        serde_json::from_slice(&self.body).expect("profile response should be JSON")
    }
}

#[tokio::test]
async fn profile_requires_authentication_csrf_and_strict_json() {
    let fixture = ProfileFixture::new().await;
    let unauthenticated = fixture.json_request(Method::GET, None, None, false).await;
    assert_error(
        &unauthenticated,
        StatusCode::UNAUTHORIZED,
        "AUTHENTICATION_REQUIRED",
    );

    let missing_csrf = fixture
        .json_request(
            Method::PATCH,
            Some(json!({ "displayName": "Reader" })),
            Some(UserKind::A),
            false,
        )
        .await;
    assert_error(&missing_csrf, StatusCode::FORBIDDEN, "FORBIDDEN");

    for body in [
        "{}",
        r#"{"unexpected":true}"#,
        r#"{"displayName":"ok","unexpected":true}"#,
        r#"{"displayName":"bad\u0000name"}"#,
        r#"{"email":"not-an-email"}"#,
        "{",
    ] {
        let invalid = fixture
            .request(Method::PATCH, Some(body), Some(UserKind::A), true, true)
            .await;
        assert_error(
            &invalid,
            StatusCode::UNPROCESSABLE_ENTITY,
            "VALIDATION_ERROR",
        );
    }

    let missing_content_type = fixture
        .request(
            Method::PATCH,
            Some(r#"{"displayName":"Reader"}"#),
            Some(UserKind::A),
            true,
            false,
        )
        .await;
    assert_error(
        &missing_content_type,
        StatusCode::UNPROCESSABLE_ENTITY,
        "VALIDATION_ERROR",
    );
}

#[tokio::test]
async fn profile_updates_normalize_clear_and_remain_user_scoped() {
    let fixture = ProfileFixture::new().await;
    let updated = fixture
        .json_request(
            Method::PATCH,
            Some(json!({
                "displayName": "  Rain Reader  ",
                "email": "  READER@EXAMPLE.COM  "
            })),
            Some(UserKind::A),
            true,
        )
        .await;
    assert_eq!(updated.status, StatusCode::OK);
    assert_eq!(updated.json()["userId"], USER_A_ID);
    assert_eq!(updated.json()["username"], "profile-a");
    assert_eq!(updated.json()["displayName"], "Rain Reader");
    assert_eq!(updated.json()["email"], "reader@example.com");
    assert_sensitive_cache_headers(&updated);

    let other = fixture
        .json_request(Method::GET, None, Some(UserKind::B), false)
        .await;
    assert_eq!(other.status, StatusCode::OK);
    assert_eq!(other.json()["userId"], USER_B_ID);
    assert!(other.json()["displayName"].is_null());
    assert!(other.json()["email"].is_null());

    let cleared = fixture
        .json_request(
            Method::PATCH,
            Some(json!({ "displayName": null, "email": null })),
            Some(UserKind::A),
            true,
        )
        .await;
    assert_eq!(cleared.status, StatusCode::OK);
    assert!(cleared.json()["displayName"].is_null());
    assert!(cleared.json()["email"].is_null());
}

#[tokio::test]
async fn duplicate_email_returns_a_stable_field_conflict_without_echoing_input() {
    let fixture = ProfileFixture::new().await;
    let owner = fixture
        .json_request(
            Method::PATCH,
            Some(json!({ "email": "owner@example.com" })),
            Some(UserKind::B),
            true,
        )
        .await;
    assert_eq!(owner.status, StatusCode::OK);

    let conflict = fixture
        .json_request(
            Method::PATCH,
            Some(json!({ "email": "OWNER@EXAMPLE.COM" })),
            Some(UserKind::A),
            true,
        )
        .await;
    assert_error(&conflict, StatusCode::CONFLICT, "PROFILE_EMAIL_TAKEN");
    assert!(conflict.json()["error"]["fields"]["email"].is_string());
    assert!(!String::from_utf8_lossy(&conflict.body).contains("owner@example.com"));
}

fn session_cookie(session: &raindrop::auth::CreatedSession) -> String {
    build_session_cookie(session, false)
        .to_string()
        .split(';')
        .next()
        .expect("session cookie should contain a pair")
        .to_owned()
}

fn assert_error(response: &CapturedResponse, status: StatusCode, code: &str) {
    assert_eq!(response.status, status);
    assert_eq!(response.json()["error"]["code"], code);
    assert_sensitive_cache_headers(response);
}

fn assert_sensitive_cache_headers(response: &CapturedResponse) {
    assert_eq!(
        response
            .headers
            .get(CACHE_CONTROL)
            .and_then(|value| value.to_str().ok()),
        Some("no-store")
    );
    assert_eq!(
        response
            .headers
            .get(PRAGMA)
            .and_then(|value| value.to_str().ok()),
        Some("no-cache")
    );
}
