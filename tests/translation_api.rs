#[allow(dead_code)]
mod support;

use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use async_trait::async_trait;
use axum::{
    Router,
    body::Body,
    http::{
        Method, Request, StatusCode,
        header::{CACHE_CONTROL, CONTENT_TYPE, COOKIE, HOST, ORIGIN, PRAGMA},
    },
};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use http_body_util::BodyExt;
use raindrop::{
    app::{AppState, build_router},
    auth::build_session_cookie,
    content::provider::ProviderSecretKeyring,
    db::{DatabaseConfig, connect, migrate},
    setup::SetupService,
    translation::{DeepLxTranslateInput, DeepLxTranslatedText, DeepLxTransport, TranslationError},
};
use secrecy::{ExposeSecret, SecretString};
use serde_json::{Value, json};
use support::database::{
    ENTRY_A_ID, SUBSCRIPTION_A_ID, USER_A_ID, USER_B_ID, insert_entry, insert_feed,
    insert_subscription, insert_user,
};
use tempfile::TempDir;
use time::macros::datetime;
use tower::ServiceExt;

struct RecordingDeepLx {
    saw_api_key: AtomicBool,
}

#[async_trait]
impl DeepLxTransport for RecordingDeepLx {
    async fn translate(
        &self,
        input: DeepLxTranslateInput,
    ) -> Result<DeepLxTranslatedText, TranslationError> {
        self.saw_api_key
            .store(input.api_key.is_some(), Ordering::SeqCst);
        Ok(DeepLxTranslatedText {
            text: format!("translated: {}", input.text),
            detected_source_locale: Some("en".to_owned()),
        })
    }
}

struct TranslationFixture {
    _data: TempDir,
    app: Router,
    transport: Option<Arc<RecordingDeepLx>>,
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

impl TranslationFixture {
    async fn new(use_fake_transport: bool) -> Self {
        let data = tempfile::tempdir().expect("temporary directory should be created");
        let url = format!(
            "sqlite://{}?mode=rwc",
            data.path().join("translation-api.db").display()
        );
        let database = connect(&DatabaseConfig::new(SecretString::from(url)))
            .await
            .expect("translation API database should connect");
        migrate(&database)
            .await
            .expect("translation API database should migrate");
        insert_user(&database, USER_A_ID, "translation-a").await;
        insert_user(&database, USER_B_ID, "translation-b").await;
        let at = datetime!(2026-07-21 12:00:00 UTC);
        insert_feed(&database, at).await;
        insert_subscription(&database, SUBSCRIPTION_A_ID, USER_A_ID, at).await;
        insert_entry(
            &database,
            ENTRY_A_ID,
            2,
            "translation-entry",
            "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
            Some(at.unix_timestamp_nanos() as i64 / 1_000),
            at,
        )
        .await;

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
        let mut state = AppState::new(setup).with_provider_keyring(Some(Arc::new(keyring())));
        let transport = use_fake_transport.then(|| {
            Arc::new(RecordingDeepLx {
                saw_api_key: AtomicBool::new(false),
            })
        });
        if let Some(transport) = &transport {
            state = state.with_translation_deeplx_transport(transport.clone());
        }
        Self {
            _data: data,
            app: build_router(state),
            transport,
            user_a_cookie: session_cookie(&session_a),
            user_a_csrf: session_a.csrf_token.expose_secret().to_owned(),
            user_b_cookie: session_cookie(&session_b),
            user_b_csrf: session_b.csrf_token.expose_secret().to_owned(),
        }
    }

    async fn request(
        &self,
        method: Method,
        uri: &str,
        body: Option<&str>,
        user: Option<UserKind>,
        include_csrf: bool,
        content_type: bool,
    ) -> CapturedResponse {
        let mut request = Request::builder().method(method).uri(uri);
        if let Some(user) = user {
            let (cookie, csrf) = match user {
                UserKind::A => (&self.user_a_cookie, &self.user_a_csrf),
                UserKind::B => (&self.user_b_cookie, &self.user_b_csrf),
            };
            request = request.header(COOKIE, cookie);
            if include_csrf {
                request = request
                    .header("x-csrf-token", csrf)
                    .header(ORIGIN, "http://translation.test")
                    .header(HOST, "translation.test");
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
                    .expect("translation request should build"),
            )
            .await
            .expect("translation request should complete");
        CapturedResponse::from_response(response).await
    }

    async fn json_request(
        &self,
        method: Method,
        uri: &str,
        body: Option<Value>,
        user: Option<UserKind>,
        include_csrf: bool,
    ) -> CapturedResponse {
        let body = body.map(|value| value.to_string());
        self.request(
            method,
            uri,
            body.as_deref(),
            user,
            include_csrf,
            body.is_some(),
        )
        .await
    }

    async fn configure_deeplx(
        &self,
        user: UserKind,
        api_key: Option<&str>,
        base_url: Option<&str>,
    ) -> CapturedResponse {
        let mut deep_lx = json!({
            "displayName": "DeepLX",
            "description": "Private translation endpoint",
            "baseUrl": base_url,
        });
        if let Some(api_key) = api_key {
            deep_lx["apiKey"] = json!(api_key);
        }
        self.json_request(
            Method::PUT,
            "/api/v2/plugins/translation",
            Some(json!({
                "expectedRevision": null,
                "engine": "DEEPLX",
                "displayMode": "BILINGUAL",
                "isEnabled": true,
                "defaultTargetLocale": "zh-CN",
                "openAi": {
                    "providerId": null,
                    "maxOutputTokens": 4096,
                    "profile": "GENERAL",
                    "customSystemPrompt": null,
                    "customPrompt": null
                },
                "deepLx": deep_lx
            })),
            Some(user),
            true,
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
                .expect("translation response body should collect")
                .to_bytes()
                .to_vec(),
        }
    }

    fn json(&self) -> Value {
        serde_json::from_slice(&self.body).expect("translation response should be JSON")
    }
}

#[tokio::test]
async fn translation_config_requires_authentication_csrf_and_strict_json() {
    let fixture = TranslationFixture::new(true).await;
    let unauthenticated = fixture
        .json_request(
            Method::GET,
            "/api/v2/plugins/translation",
            None,
            None,
            false,
        )
        .await;
    assert_error(
        &unauthenticated,
        StatusCode::UNAUTHORIZED,
        "AUTHENTICATION_REQUIRED",
    );

    let missing_csrf = fixture
        .json_request(
            Method::PUT,
            "/api/v2/plugins/translation",
            Some(json!({ "unexpected": true })),
            Some(UserKind::A),
            false,
        )
        .await;
    assert_error(&missing_csrf, StatusCode::FORBIDDEN, "FORBIDDEN");

    for body in [r#"{"unexpected":true}"#, "{}", "{"] {
        let invalid = fixture
            .request(
                Method::PUT,
                "/api/v2/plugins/translation",
                Some(body),
                Some(UserKind::A),
                true,
                true,
            )
            .await;
        assert_error(
            &invalid,
            StatusCode::UNPROCESSABLE_ENTITY,
            "VALIDATION_ERROR",
        );
    }
}

#[tokio::test]
async fn deeplx_key_is_not_returned_and_drives_lookup_and_owned_article_translation() {
    let fixture = TranslationFixture::new(true).await;
    let configured = fixture
        .configure_deeplx(UserKind::A, Some("secret-key-sentinel"), None)
        .await;
    assert_eq!(configured.status, StatusCode::OK, "{}", configured.json());
    assert_eq!(configured.json()["deepLx"]["hasApiKey"], true);
    assert!(configured.json()["deepLx"].get("apiKey").is_none());
    assert!(!String::from_utf8_lossy(&configured.body).contains("secret-key-sentinel"));

    let lookup = fixture
        .json_request(
            Method::POST,
            "/api/v2/plugins/translation/lookup",
            Some(json!({ "text": "fox" })),
            Some(UserKind::A),
            true,
        )
        .await;
    assert_eq!(lookup.status, StatusCode::OK);
    assert_eq!(lookup.json()["translation"], "translated: fox");
    assert_eq!(lookup.json()["detectedSourceLocale"], "en");
    assert!(
        fixture
            .transport
            .as_ref()
            .expect("fake transport should exist")
            .saw_api_key
            .load(Ordering::SeqCst)
    );

    let translated = fixture
        .json_request(
            Method::POST,
            &format!("/api/v2/plugins/translation/entries/{ENTRY_A_ID}/translate"),
            None,
            Some(UserKind::A),
            true,
        )
        .await;
    assert_eq!(translated.status, StatusCode::OK);
    assert_eq!(translated.json()["title"], "translated: Entry 2");
    assert_eq!(
        translated.json()["segments"][0]["originalText"],
        "Safe content"
    );
    assert_eq!(
        translated.json()["segments"][0]["translatedText"],
        "translated: Safe content"
    );

    let other_configured = fixture.configure_deeplx(UserKind::B, None, None).await;
    assert_eq!(
        other_configured.status,
        StatusCode::OK,
        "{}",
        other_configured.json()
    );
    let other_user = fixture
        .json_request(
            Method::POST,
            &format!("/api/v2/plugins/translation/entries/{ENTRY_A_ID}/translate"),
            None,
            Some(UserKind::B),
            true,
        )
        .await;
    assert_error(&other_user, StatusCode::NOT_FOUND, "NOT_FOUND");
}

#[tokio::test]
async fn selection_translation_requires_authentication_csrf_strict_json_and_bounded_text() {
    let fixture = TranslationFixture::new(true).await;
    let unauthenticated = fixture
        .json_request(
            Method::POST,
            "/api/v2/plugins/translation/translate",
            Some(json!({ "text": "Selected paragraph" })),
            None,
            false,
        )
        .await;
    assert_error(
        &unauthenticated,
        StatusCode::UNAUTHORIZED,
        "AUTHENTICATION_REQUIRED",
    );

    let missing_csrf = fixture
        .json_request(
            Method::POST,
            "/api/v2/plugins/translation/translate",
            Some(json!({ "text": "Selected paragraph" })),
            Some(UserKind::A),
            false,
        )
        .await;
    assert_error(&missing_csrf, StatusCode::FORBIDDEN, "FORBIDDEN");

    for body in [
        json!({}),
        json!({ "text": "" }),
        json!({ "text": "unsafe\u{0007}text" }),
        json!({ "text": "x".repeat(8_001) }),
        json!({ "text": "safe", "unexpected": true }),
    ] {
        let invalid = fixture
            .json_request(
                Method::POST,
                "/api/v2/plugins/translation/translate",
                Some(body),
                Some(UserKind::A),
                true,
            )
            .await;
        assert_error(
            &invalid,
            StatusCode::UNPROCESSABLE_ENTITY,
            "VALIDATION_ERROR",
        );
    }
}

#[tokio::test]
async fn selection_translation_uses_the_saved_provider_configuration() {
    let fixture = TranslationFixture::new(true).await;
    let configured = fixture
        .configure_deeplx(UserKind::A, Some("secret-key-sentinel"), None)
        .await;
    assert_eq!(configured.status, StatusCode::OK, "{}", configured.json());

    let translated = fixture
        .json_request(
            Method::POST,
            "/api/v2/plugins/translation/translate",
            Some(json!({ "text": "  Selected paragraph.  " })),
            Some(UserKind::A),
            true,
        )
        .await;

    assert_eq!(translated.status, StatusCode::OK, "{}", translated.json());
    assert_eq!(
        translated.json()["translatedText"],
        "translated: Selected paragraph."
    );
    assert_eq!(translated.json()["providerLabel"], "DeepLX");
    assert_eq!(translated.json()["detectedSourceLocale"], "en");
    assert_eq!(translated.json()["targetLocale"], "zh-CN");
}

#[tokio::test]
async fn custom_deeplx_private_addresses_are_rejected_before_connection() {
    let fixture = TranslationFixture::new(false).await;
    let configured = fixture
        .configure_deeplx(
            UserKind::A,
            None,
            Some("https://127.0.0.1/private/translate"),
        )
        .await;
    assert_eq!(configured.status, StatusCode::OK, "{}", configured.json());

    let tested = fixture
        .json_request(
            Method::POST,
            "/api/v2/plugins/translation/test",
            Some(json!({
                "engine": "DEEPLX",
                "targetLocale": "zh-CN",
                "openAi": {
                    "providerId": null,
                    "maxOutputTokens": 4096,
                    "profile": "GENERAL",
                    "customSystemPrompt": null,
                    "customPrompt": null
                },
                "deepLx": {
                    "baseUrl": "https://127.0.0.1/private/translate"
                }
            })),
            Some(UserKind::A),
            true,
        )
        .await;
    assert_error(
        &tested,
        StatusCode::UNPROCESSABLE_ENTITY,
        "VALIDATION_ERROR",
    );
}

fn keyring() -> ProviderSecretKeyring {
    let key = SecretString::from(format!("primary:{}", URL_SAFE_NO_PAD.encode([0x51_u8; 32])));
    ProviderSecretKeyring::from_entries(&[key]).expect("translation keyring should construct")
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
