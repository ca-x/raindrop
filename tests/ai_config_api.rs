#[allow(dead_code)]
mod support;

use std::{fs, sync::Arc};

use axum::{
    Router,
    body::Body,
    http::{
        Method, Request, StatusCode,
        header::{CACHE_CONTROL, CONTENT_TYPE, COOKIE, HOST, ORIGIN, PRAGMA, RETRY_AFTER},
    },
};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use http_body_util::BodyExt;
use raindrop::{
    app::{AppState, build_router},
    auth::build_session_cookie,
    content::provider::{
        CreateProvider, ProviderCapabilities, ProviderKind, ProviderPolicy, ProviderRepository,
        ProviderScope, ProviderSecretKeyring,
    },
    db::{DatabaseConfig, connect, migrate},
    plugins::PluginRegistryRepository,
    setup::SetupService,
};
use sea_orm::{ConnectionTrait, DatabaseConnection, DbBackend, Statement};
use secrecy::{ExposeSecret, SecretString};
use serde_json::{Value, json};
use support::{
    database::{USER_A_ID, USER_B_ID, insert_user},
    plugin::signed_bundle,
};
use tempfile::TempDir;
use tower::ServiceExt;

const CONFIG_PATH: &str = "/api/v1/ai/config";
const PLUGIN_KEY: &str = "raindrop.ai-content";
const OPENAPI_PATH: &str = "docs/openapi/ai-content-v1.json";

struct AiConfigFixture {
    _data: TempDir,
    database: DatabaseConnection,
    registry: PluginRegistryRepository,
    app: Router,
    user_a_cookie: String,
    user_a_csrf: String,
    user_b_cookie: String,
    user_b_csrf: String,
    instance_provider_id: String,
    user_a_provider_id: String,
    user_a_disabled_provider_id: String,
    user_b_provider_id: String,
}

#[derive(Clone, Copy)]
enum UserKind {
    A,
    B,
}

impl AiConfigFixture {
    async fn new(install_plugin: bool) -> Self {
        let data = tempfile::tempdir().expect("temporary AI config API directory");
        let url = format!(
            "sqlite://{}?mode=rwc",
            data.path().join("ai-config-api.db").display()
        );
        let database = connect(&DatabaseConfig::new(SecretString::from(url)))
            .await
            .expect("AI config API database should connect");
        migrate(&database)
            .await
            .expect("AI config API database should migrate");
        insert_user(&database, USER_A_ID, "ai-config-a").await;
        insert_user(&database, USER_B_ID, "ai-config-b").await;

        let registry = PluginRegistryRepository::new(database.clone());
        if install_plugin {
            registry
                .sync_bundled(&signed_bundle("1.0.0", b"AI config component"))
                .await
                .expect("official AI plugin should install");
        }

        let keyring = Arc::new(provider_keyring());
        let providers = ProviderRepository::new(database.clone(), Some(Arc::clone(&keyring)));
        let instance = providers
            .create(provider_input(
                ProviderScope::Instance,
                "Instance provider",
                true,
            ))
            .await
            .expect("instance provider should seed");
        let user_a = providers
            .create(provider_input(
                ProviderScope::user(USER_A_ID).unwrap(),
                "User A provider",
                true,
            ))
            .await
            .expect("user A provider should seed");
        let user_a_disabled = providers
            .create(provider_input(
                ProviderScope::user(USER_A_ID).unwrap(),
                "User A disabled",
                false,
            ))
            .await
            .expect("disabled provider should seed");
        let user_b = providers
            .create(provider_input(
                ProviderScope::user(USER_B_ID).unwrap(),
                "User B provider",
                true,
            ))
            .await
            .expect("user B provider should seed");

        let setup = SetupService::ready(data.path(), None, database.clone());
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
        let state = AppState::new(setup).with_provider_keyring(Some(keyring));
        Self {
            _data: data,
            database,
            registry,
            app: build_router(state),
            user_a_cookie: session_cookie(&session_a),
            user_a_csrf: session_a.csrf_token.expose_secret().to_owned(),
            user_b_cookie: session_cookie(&session_b),
            user_b_csrf: session_b.csrf_token.expose_secret().to_owned(),
            instance_provider_id: instance.id().to_owned(),
            user_a_provider_id: user_a.id().to_owned(),
            user_a_disabled_provider_id: user_a_disabled.id().to_owned(),
            user_b_provider_id: user_b.id().to_owned(),
        }
    }

    async fn request(
        &self,
        method: Method,
        body: Option<Value>,
        user: Option<UserKind>,
        include_csrf: bool,
    ) -> CapturedResponse {
        let mut request = Request::builder().method(method).uri(CONFIG_PATH);
        if let Some(user) = user {
            let (cookie, csrf) = match user {
                UserKind::A => (&self.user_a_cookie, &self.user_a_csrf),
                UserKind::B => (&self.user_b_cookie, &self.user_b_csrf),
            };
            request = request.header(COOKIE, cookie);
            if include_csrf {
                request = request
                    .header("x-csrf-token", csrf)
                    .header(ORIGIN, "http://ai-config.test")
                    .header(HOST, "ai-config.test");
            }
        }
        if body.is_some() {
            request = request.header(CONTENT_TYPE, "application/json");
        }
        let response = self
            .app
            .clone()
            .oneshot(
                request
                    .body(body.map_or_else(Body::empty, |value| Body::from(value.to_string())))
                    .expect("AI config request should build"),
            )
            .await
            .expect("AI config request should complete");
        CapturedResponse::from_response(response).await
    }

    async fn set_plugin_state(&self, state: &str) {
        self.database
            .execute(Statement::from_sql_and_values(
                DbBackend::Sqlite,
                "UPDATE plugin_installations SET system_state = ? WHERE plugin_key = ?",
                [state.into(), PLUGIN_KEY.into()],
            ))
            .await
            .expect("plugin state should update");
    }

    async fn stored_config_json(&self) -> Value {
        let config = self
            .registry
            .get_ai_config(PLUGIN_KEY, USER_A_ID)
            .await
            .expect("stored AI config should read")
            .expect("stored AI config should exist");
        serde_json::from_str(config.canonical_json())
            .expect("stored canonical AI config should be JSON")
    }
}

struct CapturedResponse {
    status: StatusCode,
    headers: axum::http::HeaderMap,
    body: Vec<u8>,
}

#[test]
fn config_openapi_is_strict_and_excludes_internal_config_state() {
    let document: Value = serde_json::from_str(
        &fs::read_to_string(OPENAPI_PATH).expect("AI content OpenAPI should exist"),
    )
    .expect("AI content OpenAPI should be valid JSON");
    assert_eq!(document["openapi"], "3.1.0");
    assert!(document["paths"][CONFIG_PATH]["get"].is_object());
    assert!(document["paths"][CONFIG_PATH]["put"].is_object());
    assert_eq!(
        document["components"]["schemas"]["AiConfigEnvelope"]["additionalProperties"],
        false
    );
    assert_eq!(
        document["components"]["schemas"]["PutAiConfigRequest"]["additionalProperties"],
        false
    );
    let response_schema =
        serde_json::to_string(&document["components"]["schemas"]["AiConfigEnvelope"])
            .expect("AI config response schema should serialize");
    for forbidden in [
        "canonicalJson",
        "configHash",
        "schemaVersion",
        "payloadJson",
        "provenanceJson",
        "credential",
    ] {
        assert!(
            !response_schema.contains(forbidden),
            "AI config response schema leaks {forbidden}"
        );
    }
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
                .expect("AI config response should collect")
                .to_bytes()
                .to_vec(),
        }
    }

    fn json(&self) -> Value {
        serde_json::from_slice(&self.body).expect("AI config response should be JSON")
    }
}

#[tokio::test]
async fn config_get_returns_typed_null_default_and_is_user_scoped() {
    let fixture = AiConfigFixture::new(true).await;
    let initial = fixture
        .request(Method::GET, None, Some(UserKind::A), false)
        .await;
    assert_eq!(initial.status, StatusCode::OK);
    assert_eq!(
        initial.json(),
        json!({
            "pluginState": "READY",
            "mcpState": "CONTRACT_READY_TRANSPORT_UNAVAILABLE",
            "config": null
        })
    );
    assert_sensitive_cache_headers(&initial);

    let created = fixture
        .request(
            Method::PUT,
            Some(config_body(
                Value::Null,
                true,
                &fixture.user_a_provider_id,
                true,
                &fixture.user_a_provider_id,
            )),
            Some(UserKind::A),
            true,
        )
        .await;
    assert_eq!(created.status, StatusCode::OK);
    assert_public_config(&created.json(), 0, true);

    let other_user = fixture
        .request(Method::GET, None, Some(UserKind::B), false)
        .await;
    assert_eq!(other_user.status, StatusCode::OK);
    assert!(other_user.json()["config"].is_null());
    assert_sensitive_cache_headers(&other_user);
}

#[tokio::test]
async fn config_put_creates_replaces_and_writes_fixed_internal_subtrees() {
    let fixture = AiConfigFixture::new(true).await;
    let created = fixture
        .request(
            Method::PUT,
            Some(config_body(
                Value::Null,
                true,
                &fixture.user_a_provider_id,
                true,
                &fixture.user_a_provider_id,
            )),
            Some(UserKind::A),
            true,
        )
        .await;
    assert_eq!(created.status, StatusCode::OK);
    assert_public_config(&created.json(), 0, true);
    let stored = fixture.stored_config_json().await;
    for operation in ["summarize", "translate"] {
        assert_eq!(
            stored["operations"][operation]["mcp"],
            json!({
                "mode": "DISABLED",
                "failurePolicy": "FAIL_OPEN",
                "maxToolCalls": 0,
                "tools": []
            })
        );
    }
    assert_eq!(
        stored["automatic"],
        json!({
            "enabled": false,
            "operations": ["SUMMARIZE", "TRANSLATE"],
            "allSubscribedFeeds": false,
            "feedIds": [],
            "categoryIds": []
        })
    );

    let mut replacement = config_body(
        json!(0),
        false,
        &fixture.user_a_provider_id,
        true,
        &fixture.instance_provider_id,
    );
    replacement["summary"]["style"] = json!("CONCISE");
    replacement["translation"]["defaultTargetLocale"] = json!("en-US");
    let replaced = fixture
        .request(Method::PUT, Some(replacement), Some(UserKind::A), true)
        .await;
    assert_eq!(replaced.status, StatusCode::OK);
    assert_public_config(&replaced.json(), 1, true);
    assert_eq!(replaced.json()["config"]["summary"]["style"], "CONCISE");
    assert_eq!(
        replaced.json()["config"]["translation"]["defaultTargetLocale"],
        "en-US"
    );

    let stale = fixture
        .request(
            Method::PUT,
            Some(config_body(
                json!(0),
                true,
                &fixture.user_a_provider_id,
                true,
                &fixture.user_a_provider_id,
            )),
            Some(UserKind::A),
            true,
        )
        .await;
    assert_error(&stale, StatusCode::CONFLICT, "REVISION_CONFLICT");

    let disabled = fixture
        .request(
            Method::PUT,
            Some(config_body(
                json!(1),
                false,
                &fixture.user_b_provider_id,
                false,
                &fixture.user_b_provider_id,
            )),
            Some(UserKind::A),
            true,
        )
        .await;
    assert_eq!(disabled.status, StatusCode::OK);
    assert_public_config(&disabled.json(), 2, false);
}

#[tokio::test]
async fn config_put_rejects_invalid_shape_state_and_enabled_provider_selection() {
    let fixture = AiConfigFixture::new(true).await;

    let mismatch = fixture
        .request(
            Method::PUT,
            Some({
                let mut body = config_body(
                    Value::Null,
                    true,
                    &fixture.user_a_provider_id,
                    false,
                    &fixture.user_a_provider_id,
                );
                body["isEnabled"] = json!(false);
                body
            }),
            Some(UserKind::A),
            true,
        )
        .await;
    assert_error(
        &mismatch,
        StatusCode::UNPROCESSABLE_ENTITY,
        "VALIDATION_ERROR",
    );

    for provider_id in [
        fixture.user_a_disabled_provider_id.as_str(),
        fixture.user_b_provider_id.as_str(),
        "00000000-0000-4000-8000-000000009999",
    ] {
        let unavailable = fixture
            .request(
                Method::PUT,
                Some(config_body(
                    Value::Null,
                    true,
                    provider_id,
                    false,
                    &fixture.user_a_provider_id,
                )),
                Some(UserKind::A),
                true,
            )
            .await;
        assert_error(&unavailable, StatusCode::CONFLICT, "AI_UNAVAILABLE");
    }

    let unknown_field = fixture
        .request(
            Method::PUT,
            Some({
                let mut body = config_body(
                    Value::Null,
                    true,
                    &fixture.user_a_provider_id,
                    true,
                    &fixture.user_a_provider_id,
                );
                body["unknown"] = json!(true);
                body
            }),
            Some(UserKind::A),
            true,
        )
        .await;
    assert_error(
        &unknown_field,
        StatusCode::UNPROCESSABLE_ENTITY,
        "VALIDATION_ERROR",
    );

    let invalid_locale = fixture
        .request(
            Method::PUT,
            Some({
                let mut body = config_body(
                    Value::Null,
                    true,
                    &fixture.user_a_provider_id,
                    true,
                    &fixture.user_a_provider_id,
                );
                body["translation"]["defaultTargetLocale"] = json!("not_a_locale");
                body
            }),
            Some(UserKind::A),
            true,
        )
        .await;
    assert_error(
        &invalid_locale,
        StatusCode::UNPROCESSABLE_ENTITY,
        "VALIDATION_ERROR",
    );

    let invalid_tokens = fixture
        .request(
            Method::PUT,
            Some({
                let mut body = config_body(
                    Value::Null,
                    true,
                    &fixture.user_a_provider_id,
                    true,
                    &fixture.user_a_provider_id,
                );
                body["summary"]["maxOutputTokens"] = json!(127);
                body
            }),
            Some(UserKind::A),
            true,
        )
        .await;
    assert_error(
        &invalid_tokens,
        StatusCode::UNPROCESSABLE_ENTITY,
        "VALIDATION_ERROR",
    );
}

#[tokio::test]
async fn config_plugin_states_are_explicit_and_unavailable_for_mutation() {
    let missing = AiConfigFixture::new(false).await;
    let unavailable = missing
        .request(Method::GET, None, Some(UserKind::A), false)
        .await;
    assert_eq!(unavailable.status, StatusCode::OK);
    assert_eq!(unavailable.json()["pluginState"], "UNAVAILABLE");
    assert!(unavailable.json()["config"].is_null());
    let missing_put = missing
        .request(
            Method::PUT,
            Some(config_body(
                Value::Null,
                true,
                &missing.user_a_provider_id,
                true,
                &missing.user_a_provider_id,
            )),
            Some(UserKind::A),
            true,
        )
        .await;
    assert_error(&missing_put, StatusCode::CONFLICT, "AI_UNAVAILABLE");

    for (stored, public) in [("DISABLED", "DISABLED"), ("QUARANTINED", "QUARANTINED")] {
        let fixture = AiConfigFixture::new(true).await;
        fixture.set_plugin_state(stored).await;
        let get = fixture
            .request(Method::GET, None, Some(UserKind::A), false)
            .await;
        assert_eq!(get.status, StatusCode::OK);
        assert_eq!(get.json()["pluginState"], public);
        let put = fixture
            .request(
                Method::PUT,
                Some(config_body(
                    Value::Null,
                    true,
                    &fixture.user_a_provider_id,
                    true,
                    &fixture.user_a_provider_id,
                )),
                Some(UserKind::A),
                true,
            )
            .await;
        assert_error(&put, StatusCode::CONFLICT, "AI_UNAVAILABLE");
    }
}

#[tokio::test]
async fn config_auth_csrf_rate_cache_and_methods_are_stable() {
    let fixture = AiConfigFixture::new(true).await;
    let unauthenticated = fixture.request(Method::GET, None, None, false).await;
    assert_error(
        &unauthenticated,
        StatusCode::UNAUTHORIZED,
        "AUTHENTICATION_REQUIRED",
    );
    let missing_csrf = fixture
        .request(
            Method::PUT,
            Some(config_body(
                Value::Null,
                true,
                &fixture.user_a_provider_id,
                true,
                &fixture.user_a_provider_id,
            )),
            Some(UserKind::A),
            false,
        )
        .await;
    assert_error(&missing_csrf, StatusCode::FORBIDDEN, "FORBIDDEN");

    for _ in 0..30 {
        let admitted = fixture
            .request(
                Method::PUT,
                Some(config_body(
                    json!(99),
                    true,
                    &fixture.user_a_provider_id,
                    true,
                    &fixture.user_a_provider_id,
                )),
                Some(UserKind::A),
                true,
            )
            .await;
        assert_error(&admitted, StatusCode::NOT_FOUND, "NOT_FOUND");
    }
    let limited = fixture
        .request(
            Method::PUT,
            Some(config_body(
                json!(99),
                true,
                &fixture.user_a_provider_id,
                true,
                &fixture.user_a_provider_id,
            )),
            Some(UserKind::A),
            true,
        )
        .await;
    assert_error(&limited, StatusCode::TOO_MANY_REQUESTS, "RATE_LIMITED");
    assert!(limited.headers.contains_key(RETRY_AFTER));

    let method = fixture
        .request(Method::POST, None, Some(UserKind::A), false)
        .await;
    assert_error(
        &method,
        StatusCode::METHOD_NOT_ALLOWED,
        "METHOD_NOT_ALLOWED",
    );
}

fn config_body(
    expected_revision: Value,
    summary_enabled: bool,
    summary_provider_id: &str,
    translation_enabled: bool,
    translation_provider_id: &str,
) -> Value {
    json!({
        "expectedRevision": expected_revision,
        "isEnabled": summary_enabled || translation_enabled,
        "summary": {
            "enabled": summary_enabled,
            "providerId": summary_provider_id,
            "style": "BALANCED",
            "maxOutputTokens": 1024
        },
        "translation": {
            "enabled": translation_enabled,
            "providerId": translation_provider_id,
            "defaultTargetLocale": "zh-CN",
            "maxOutputTokens": 4096
        }
    })
}

fn provider_input(scope: ProviderScope, display_name: &str, is_enabled: bool) -> CreateProvider {
    CreateProvider {
        scope,
        display_name: display_name.to_owned(),
        kind: ProviderKind::OpenAiResponses,
        endpoint: None,
        model: "gpt-test-model".to_owned(),
        credential: SecretString::from("ai-config-provider-secret-sentinel"),
        capabilities: ProviderCapabilities {
            supports_usage: true,
            supports_idempotency: true,
            supports_streaming: false,
        },
        policy: ProviderPolicy {
            max_concurrency: 2,
            requests_per_minute: Some(30),
            max_input_tokens_per_request: 128_000,
            max_output_tokens_per_request: 4_096,
            input_cost_micros_per_million_tokens: None,
            output_cost_micros_per_million_tokens: None,
            max_cost_micros_per_request: Some(250_000),
        },
        is_enabled,
    }
}

fn provider_keyring() -> ProviderSecretKeyring {
    let key = SecretString::from(format!("primary:{}", URL_SAFE_NO_PAD.encode([0x41_u8; 32])));
    ProviderSecretKeyring::from_entries(&[key]).expect("AI config keyring should construct")
}

fn session_cookie(session: &raindrop::auth::CreatedSession) -> String {
    build_session_cookie(session, false)
        .to_string()
        .split(';')
        .next()
        .expect("session cookie should have a name/value pair")
        .to_owned()
}

fn assert_public_config(response: &Value, revision: u64, is_enabled: bool) {
    assert_eq!(response["pluginState"], "READY");
    assert_eq!(response["mcpState"], "CONTRACT_READY_TRANSPORT_UNAVAILABLE");
    assert_eq!(response["config"]["revision"], revision);
    assert_eq!(response["config"]["isEnabled"], is_enabled);
    let serialized = response.to_string();
    for forbidden in [
        "canonicalJson",
        "configHash",
        "schemaVersion",
        "\"mcp\":",
        "\"automatic\":",
        "credential",
        "encryptedSecret",
    ] {
        assert!(
            !serialized.contains(forbidden),
            "response leaked {forbidden}"
        );
    }
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
