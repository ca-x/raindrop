#[allow(dead_code)]
mod support;

use axum::{
    Router,
    body::Body,
    http::{
        Method, Request, StatusCode,
        header::{
            ACCEPT_LANGUAGE, CACHE_CONTROL, CONTENT_TYPE, COOKIE, HOST, ORIGIN, PRAGMA, RETRY_AFTER,
        },
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

struct PreferenceFixture {
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

#[derive(Clone, Copy, Default)]
struct RequestOptions<'a> {
    user: Option<UserKind>,
    include_csrf: bool,
    content_type: bool,
    accept_language: Option<&'a str>,
}

impl PreferenceFixture {
    async fn new() -> Self {
        let data = tempfile::tempdir().expect("temporary directory should be created");
        let url = format!(
            "sqlite://{}?mode=rwc",
            data.path().join("preference-api.db").display()
        );
        let database = connect(&DatabaseConfig::new(SecretString::from(url)))
            .await
            .expect("preference API database should connect");
        migrate(&database)
            .await
            .expect("preference API database should migrate");
        insert_user(&database, USER_A_ID, "preference-api-a").await;
        insert_user(&database, USER_B_ID, "preference-api-b").await;

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
        let user_a_cookie = session_cookie(&session_a);
        let user_b_cookie = session_cookie(&session_b);
        let user_a_csrf = session_a.csrf_token.expose_secret().to_owned();
        let user_b_csrf = session_b.csrf_token.expose_secret().to_owned();
        let app = build_router(AppState::new(setup));

        Self {
            _data: data,
            app,
            user_a_cookie,
            user_a_csrf,
            user_b_cookie,
            user_b_csrf,
        }
    }

    async fn request(
        &self,
        method: Method,
        uri: &str,
        body: Option<&str>,
        options: RequestOptions<'_>,
    ) -> CapturedResponse {
        let mut request = Request::builder().method(method).uri(uri);
        if let Some(user) = options.user {
            let (cookie, csrf) = match user {
                UserKind::A => (&self.user_a_cookie, &self.user_a_csrf),
                UserKind::B => (&self.user_b_cookie, &self.user_b_csrf),
            };
            request = request.header(COOKIE, cookie);
            if options.include_csrf {
                request = request
                    .header("x-csrf-token", csrf)
                    .header(ORIGIN, "http://preferences.test")
                    .header(HOST, "preferences.test");
            }
        }
        if options.content_type {
            request = request.header(CONTENT_TYPE, "application/json");
        }
        if let Some(accept_language) = options.accept_language {
            request = request.header(ACCEPT_LANGUAGE, accept_language);
        }
        let response = self
            .app
            .clone()
            .oneshot(
                request
                    .body(body.map_or_else(Body::empty, |body| Body::from(body.to_owned())))
                    .expect("preference request should build"),
            )
            .await
            .expect("preference request should complete");
        CapturedResponse::from_response(response).await
    }

    async fn json_request(
        &self,
        method: Method,
        uri: &str,
        body: Option<Value>,
        user: Option<UserKind>,
        include_csrf: bool,
        accept_language: Option<&str>,
    ) -> CapturedResponse {
        let body = body.map(|value| value.to_string());
        self.request(
            method,
            uri,
            body.as_deref(),
            RequestOptions {
                user,
                include_csrf,
                content_type: body.is_some(),
                accept_language,
            },
        )
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
                .expect("preference response body should collect")
                .to_bytes()
                .to_vec(),
        }
    }

    fn json(&self) -> Value {
        serde_json::from_slice(&self.body).expect("preference response should be JSON")
    }
}

#[tokio::test]
async fn get_requires_authentication_and_uses_only_the_leading_language_range() {
    let fixture = PreferenceFixture::new().await;
    let unauthenticated = fixture
        .json_request(
            Method::GET,
            "/api/v1/preferences",
            None,
            None,
            false,
            Some("zh-CN"),
        )
        .await;
    assert_error(
        &unauthenticated,
        StatusCode::UNAUTHORIZED,
        "AUTHENTICATION_REQUIRED",
    );

    let zh = fixture
        .json_request(
            Method::GET,
            "/api/v1/preferences",
            None,
            Some(UserKind::A),
            false,
            Some("zh-Hans, en;q=0.8"),
        )
        .await;
    assert_eq!(zh.status, StatusCode::OK);
    assert_eq!(
        zh.json(),
        json!({
            "locale": "zh-CN",
            "themeMode": "SYSTEM",
            "layoutDensity": "BALANCED",
            "readingFontScale": 100
        })
    );
    assert_sensitive_cache_headers(&zh);

    let en = fixture
        .json_request(
            Method::GET,
            "/api/v1/preferences",
            None,
            Some(UserKind::B),
            false,
            Some("en-US, zh;q=1.0"),
        )
        .await;
    assert_eq!(en.status, StatusCode::OK);
    assert_eq!(en.json()["locale"], "en");
    assert_sensitive_cache_headers(&en);
}

#[tokio::test]
async fn patch_enforces_authentication_csrf_json_and_strict_validation_in_order() {
    let fixture = PreferenceFixture::new().await;
    let unknown = r#"{"unexpected":true}"#;
    let unauthenticated = fixture
        .request(
            Method::PATCH,
            "/api/v1/preferences",
            Some(unknown),
            RequestOptions {
                content_type: true,
                ..Default::default()
            },
        )
        .await;
    assert_error(
        &unauthenticated,
        StatusCode::UNAUTHORIZED,
        "AUTHENTICATION_REQUIRED",
    );

    let missing_csrf = fixture
        .request(
            Method::PATCH,
            "/api/v1/preferences",
            Some(unknown),
            RequestOptions {
                user: Some(UserKind::A),
                content_type: true,
                ..Default::default()
            },
        )
        .await;
    assert_error(&missing_csrf, StatusCode::FORBIDDEN, "FORBIDDEN");

    for body in [
        unknown,
        "{}",
        r#"{"locale":null}"#,
        r#"{"locale":null,"themeMode":"DARK"}"#,
        r#"{"locale":"fr"}"#,
        r#"{"themeMode":"AUTO"}"#,
        r#"{"layoutDensity":"DENSE"}"#,
        r#"{"readingFontScale":84}"#,
        r#"{"readingFontScale":131}"#,
        "{",
    ] {
        let invalid = fixture
            .request(
                Method::PATCH,
                "/api/v1/preferences",
                Some(body),
                RequestOptions {
                    user: Some(UserKind::A),
                    include_csrf: true,
                    content_type: true,
                    ..Default::default()
                },
            )
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
            "/api/v1/preferences",
            Some(r#"{"themeMode":"DARK"}"#),
            RequestOptions {
                user: Some(UserKind::A),
                include_csrf: true,
                ..Default::default()
            },
        )
        .await;
    assert_error(
        &missing_content_type,
        StatusCode::UNPROCESSABLE_ENTITY,
        "VALIDATION_ERROR",
    );
}

#[tokio::test]
async fn patch_persists_each_field_and_keeps_users_isolated() {
    let fixture = PreferenceFixture::new().await;
    let patches = [
        json!({ "locale": "zh-CN" }),
        json!({ "themeMode": "DARK" }),
        json!({ "layoutDensity": "COMPACT" }),
        json!({ "readingFontScale": 130 }),
    ];
    let expected = [
        json!({
            "locale": "zh-CN",
            "themeMode": "SYSTEM",
            "layoutDensity": "BALANCED",
            "readingFontScale": 100
        }),
        json!({
            "locale": "zh-CN",
            "themeMode": "DARK",
            "layoutDensity": "BALANCED",
            "readingFontScale": 100
        }),
        json!({
            "locale": "zh-CN",
            "themeMode": "DARK",
            "layoutDensity": "COMPACT",
            "readingFontScale": 100
        }),
        json!({
            "locale": "zh-CN",
            "themeMode": "DARK",
            "layoutDensity": "COMPACT",
            "readingFontScale": 130
        }),
    ];
    for (patch, expected) in patches.into_iter().zip(&expected) {
        let response = fixture
            .json_request(
                Method::PATCH,
                "/api/v1/preferences",
                Some(patch),
                Some(UserKind::A),
                true,
                Some("en"),
            )
            .await;
        assert_eq!(response.status, StatusCode::OK);
        assert_eq!(response.json(), *expected);
        assert_sensitive_cache_headers(&response);
    }

    let user_b = fixture
        .json_request(
            Method::PATCH,
            "/api/v1/preferences",
            Some(json!({
                "locale": "en",
                "themeMode": "LIGHT",
                "layoutDensity": "SPACIOUS",
                "readingFontScale": 85
            })),
            Some(UserKind::B),
            true,
            Some("zh-CN"),
        )
        .await;
    assert_eq!(
        user_b.json(),
        json!({
            "locale": "en",
            "themeMode": "LIGHT",
            "layoutDensity": "SPACIOUS",
            "readingFontScale": 85
        })
    );

    let persisted_a = fixture
        .json_request(
            Method::GET,
            "/api/v1/preferences",
            None,
            Some(UserKind::A),
            false,
            Some("en"),
        )
        .await;
    assert_eq!(persisted_a.json(), expected[3]);
}

#[tokio::test]
async fn v2_adds_reading_preferences_without_changing_the_v1_contract() {
    let fixture = PreferenceFixture::new().await;
    let initial = fixture
        .json_request(
            Method::GET,
            "/api/v2/preferences",
            None,
            Some(UserKind::A),
            false,
            Some("en"),
        )
        .await;
    assert_eq!(
        initial.json(),
        json!({
            "locale": "en",
            "themeMode": "SYSTEM",
            "layoutDensity": "BALANCED",
            "readingFontScale": 100,
            "readingFontFamily": "SERIF",
            "readingColorScheme": "AUTO",
            "linkOpenMode": "NEW_TAB"
        })
    );

    let updated = fixture
        .json_request(
            Method::PATCH,
            "/api/v2/preferences",
            Some(json!({
                "readingFontFamily": "SANS",
                "readingColorScheme": "SEPIA",
                "linkOpenMode": "CURRENT_TAB"
            })),
            Some(UserKind::A),
            true,
            Some("en"),
        )
        .await;
    assert_eq!(updated.status, StatusCode::OK);
    assert_eq!(updated.json()["readingFontFamily"], "SANS");
    assert_eq!(updated.json()["readingColorScheme"], "SEPIA");
    assert_eq!(updated.json()["linkOpenMode"], "CURRENT_TAB");

    let legacy = fixture
        .json_request(
            Method::GET,
            "/api/v1/preferences",
            None,
            Some(UserKind::A),
            false,
            Some("en"),
        )
        .await;
    assert_eq!(
        legacy.json(),
        json!({
            "locale": "en",
            "themeMode": "SYSTEM",
            "layoutDensity": "BALANCED",
            "readingFontScale": 100
        })
    );
    assert_sensitive_cache_headers(&legacy);
}

#[tokio::test]
async fn preference_namespace_has_exact_json_fallback_and_method_contracts() {
    let fixture = PreferenceFixture::new().await;
    for uri in ["/api/v1/preferences/", "/api/v1/preferences/unknown"] {
        let response = fixture
            .json_request(Method::GET, uri, None, Some(UserKind::A), false, None)
            .await;
        assert_error(&response, StatusCode::NOT_FOUND, "NOT_FOUND");
    }

    for method in [Method::POST, Method::PUT, Method::DELETE] {
        let response = fixture
            .json_request(
                method,
                "/api/v1/preferences",
                None,
                Some(UserKind::A),
                false,
                None,
            )
            .await;
        assert_error(
            &response,
            StatusCode::METHOD_NOT_ALLOWED,
            "METHOD_NOT_ALLOWED",
        );
    }
}

#[tokio::test]
async fn preference_mutation_budget_is_per_user_and_separate_from_categories() {
    let fixture = PreferenceFixture::new().await;
    for _ in 0..30 {
        let response = fixture
            .json_request(
                Method::PATCH,
                "/api/v1/preferences",
                Some(json!({ "themeMode": "DARK" })),
                Some(UserKind::A),
                true,
                None,
            )
            .await;
        assert_eq!(response.status, StatusCode::OK);
    }
    let limited = fixture
        .json_request(
            Method::PATCH,
            "/api/v1/preferences",
            Some(json!({ "themeMode": "LIGHT" })),
            Some(UserKind::A),
            true,
            None,
        )
        .await;
    assert_error(&limited, StatusCode::TOO_MANY_REQUESTS, "RATE_LIMITED");
    assert!(limited.headers.get(RETRY_AFTER).is_some());
    assert!(limited.json()["error"]["fields"]["retryAt"].is_string());

    let user_b = fixture
        .json_request(
            Method::PATCH,
            "/api/v1/preferences",
            Some(json!({ "themeMode": "LIGHT" })),
            Some(UserKind::B),
            true,
            None,
        )
        .await;
    assert_eq!(user_b.status, StatusCode::OK);

    let category = fixture
        .json_request(
            Method::POST,
            "/api/v1/categories",
            Some(json!({ "title": "Still available" })),
            Some(UserKind::A),
            true,
            None,
        )
        .await;
    assert_eq!(category.status, StatusCode::CREATED);
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
