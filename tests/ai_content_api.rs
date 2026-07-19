#[allow(dead_code)]
mod support;

use std::sync::Arc;

use axum::{
    Router,
    body::Body,
    http::{
        Method, Request, StatusCode,
        header::{
            CACHE_CONTROL, CONTENT_TYPE, COOKIE, HOST, LOCATION, ORIGIN, PRAGMA, RETRY_AFTER,
        },
    },
};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use http_body_util::BodyExt;
use raindrop::{
    app::{AppState, build_router},
    auth::build_session_cookie,
    content::{
        jobs::{ContentJobOperation, ContentRepository},
        provider::{
            CreateProvider, ProviderCapabilities, ProviderKind, ProviderPolicy, ProviderRepository,
            ProviderScope, ProviderSecretKeyring,
        },
    },
    db::{
        entities::{content_artifact, content_job, content_job_result, plugin_config},
        migrate,
    },
    plugins::{PluginRegistryRepository, SummaryArtifact, TranslationArtifact},
    setup::SetupService,
};
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, DatabaseConnection, EntityTrait,
    PaginatorTrait, QueryFilter, sea_query::Expr,
};
use secrecy::{ExposeSecret, SecretString};
use serde_json::{Value, json};
use support::{
    database::{
        ENTRY_A_ID, HASH_D, SUBSCRIPTION_A_ID, USER_A_ID, USER_B_ID, connect_for_contract,
        insert_entry, insert_feed, insert_subscription, insert_user,
    },
    plugin::signed_bundle,
};
use tempfile::TempDir;
use time::{OffsetDateTime, macros::datetime};
use tower::ServiceExt;

const ENTRY_AI_PATH: &str = "/api/v1/entries/00000000-0000-4000-8000-000000000301/ai";
const PLUGIN_KEY: &str = "raindrop.ai-content";

struct AiContentFixture {
    _data: TempDir,
    database: DatabaseConnection,
    app: Router,
    user_a_cookie: String,
    user_a_csrf: String,
    user_b_cookie: String,
    user_b_csrf: String,
    provider_id: String,
}

#[derive(Clone, Copy)]
enum UserKind {
    A,
    B,
}

impl AiContentFixture {
    async fn new(app_has_keyring: bool, install_plugin: bool, configure: bool) -> Self {
        let data = tempfile::tempdir().expect("temporary AI content API directory");
        let url = format!(
            "sqlite://{}?mode=rwc",
            data.path().join("ai-content-api.db").display()
        );
        let database = connect_for_contract(SecretString::from(url)).await;
        migrate(&database)
            .await
            .expect("AI content API database should migrate");
        let now = datetime!(2026-07-19 12:00:00 UTC);
        insert_user(&database, USER_A_ID, "ai-content-a").await;
        insert_user(&database, USER_B_ID, "ai-content-b").await;
        insert_feed(&database, now).await;
        insert_subscription(&database, SUBSCRIPTION_A_ID, USER_A_ID, now).await;
        insert_entry(
            &database,
            ENTRY_A_ID,
            1,
            "ai-content-entry",
            HASH_D,
            Some(1_752_926_400_000_000),
            now,
        )
        .await;

        let registry = PluginRegistryRepository::new(database.clone());
        if install_plugin {
            registry
                .sync_bundled(&signed_bundle("1.0.0", b"AI content API component"))
                .await
                .expect("official AI plugin should install");
        }
        let keyring = Arc::new(provider_keyring());
        let providers = ProviderRepository::new(database.clone(), Some(Arc::clone(&keyring)));
        let provider = providers
            .create(provider_input())
            .await
            .expect("AI content provider should create");
        if configure {
            registry
                .replace_ai_config(
                    PLUGIN_KEY,
                    USER_A_ID,
                    None,
                    true,
                    &config_json(provider.id()),
                )
                .await
                .expect("AI content config should create");
        }

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
        let state = AppState::new(setup).with_provider_keyring(app_has_keyring.then_some(keyring));
        Self {
            _data: data,
            database,
            app: build_router(state),
            user_a_cookie: session_cookie(&session_a),
            user_a_csrf: session_a.csrf_token.expose_secret().to_owned(),
            user_b_cookie: session_cookie(&session_b),
            user_b_csrf: session_b.csrf_token.expose_secret().to_owned(),
            provider_id: provider.id().to_owned(),
        }
    }

    async fn request(
        &self,
        method: Method,
        uri: &str,
        body: Option<Value>,
        user: Option<UserKind>,
        include_csrf: bool,
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
                    .header(ORIGIN, "http://ai-content.test")
                    .header(HOST, "ai-content.test");
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
                    .expect("AI content request should build"),
            )
            .await
            .expect("AI content request should complete");
        CapturedResponse::from_response(response).await
    }

    async fn enqueue(
        &self,
        operation: &str,
        target_locale: Value,
        idempotency_key: &str,
    ) -> CapturedResponse {
        self.request(
            Method::POST,
            &format!("{ENTRY_AI_PATH}/jobs"),
            Some(json!({
                "operation": operation,
                "targetLocale": target_locale,
                "idempotencyKey": idempotency_key
            })),
            Some(UserKind::A),
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
                .expect("AI content response should collect")
                .to_bytes()
                .to_vec(),
        }
    }

    fn json(&self) -> Value {
        serde_json::from_slice(&self.body).unwrap_or_else(|error| {
            panic!(
                "AI content response should be JSON: status={} body={:?}: {error}",
                self.status,
                String::from_utf8_lossy(&self.body)
            )
        })
    }
}

#[tokio::test]
async fn overview_and_enqueue_tracer_cover_idle_queued_existing_and_keyring_unavailable() {
    let fixture = AiContentFixture::new(true, true, true).await;
    let initial = fixture
        .request(
            Method::GET,
            &format!("{ENTRY_AI_PATH}?translationLocale=ja"),
            None,
            Some(UserKind::A),
            false,
        )
        .await;
    assert_eq!(initial.status, StatusCode::OK);
    assert_eq!(initial.json()["availability"], "READY");
    assert_eq!(initial.json()["summary"]["state"], "IDLE");
    assert_eq!(initial.json()["translation"]["state"], "IDLE");
    assert_eq!(initial.json()["translation"]["targetLocale"], "ja");
    assert_public_overview(&initial);

    let queued = fixture
        .enqueue("SUMMARIZE", Value::Null, "reader:api-summary")
        .await;
    assert_eq!(queued.status, StatusCode::CREATED);
    assert_eq!(queued.json()["status"], "QUEUED");
    let job_id = queued.json()["jobId"].as_str().unwrap().to_owned();
    assert_eq!(
        queued
            .headers
            .get(LOCATION)
            .and_then(|value| value.to_str().ok()),
        Some(format!("/api/v1/ai/jobs/{job_id}").as_str())
    );
    assert_sensitive_cache_headers(&queued);

    let existing = fixture
        .enqueue("SUMMARIZE", Value::Null, "reader:api-summary")
        .await;
    assert_eq!(existing.status, StatusCode::OK);
    assert_eq!(existing.json()["jobId"], job_id);

    let overview = fixture
        .request(Method::GET, ENTRY_AI_PATH, None, Some(UserKind::A), false)
        .await;
    assert_eq!(overview.json()["summary"]["state"], "QUEUED");
    assert_eq!(overview.json()["summary"]["job"]["jobId"], job_id);

    let missing_target = fixture
        .request(
            Method::POST,
            &format!("{ENTRY_AI_PATH}/jobs"),
            Some(json!({
                "operation": "SUMMARIZE",
                "idempotencyKey": "reader:missing-target"
            })),
            Some(UserKind::A),
            true,
        )
        .await;
    assert_error(
        &missing_target,
        StatusCode::UNPROCESSABLE_ENTITY,
        "VALIDATION_ERROR",
    );

    let no_keyring = AiContentFixture::new(false, true, true).await;
    let unavailable = no_keyring
        .enqueue("SUMMARIZE", Value::Null, "reader:no-keyring")
        .await;
    assert_error(
        &unavailable,
        StatusCode::SERVICE_UNAVAILABLE,
        "AI_PROVIDER_KEYRING_UNAVAILABLE",
    );
    assert_eq!(job_count(&no_keyring.database).await, 0);
}

#[tokio::test]
async fn status_result_and_retry_are_typed_and_preserve_failed_history() {
    let fixture = AiContentFixture::new(true, true, true).await;
    let queued = fixture
        .enqueue("SUMMARIZE", Value::Null, "reader:status-result")
        .await;
    let job_id = queued.json()["jobId"].as_str().unwrap().to_owned();

    let status = fixture
        .request(
            Method::GET,
            &format!("/api/v1/ai/jobs/{job_id}"),
            None,
            Some(UserKind::A),
            false,
        )
        .await;
    assert_eq!(status.status, StatusCode::OK);
    assert_eq!(status.json()["status"], "QUEUED");
    assert_safe_job(&status);

    let not_ready = fixture
        .request(
            Method::GET,
            &format!("/api/v1/ai/jobs/{job_id}/result"),
            None,
            Some(UserKind::A),
            false,
        )
        .await;
    assert_error(&not_ready, StatusCode::CONFLICT, "AI_RESULT_NOT_READY");

    mark_job_failed(&fixture.database, &job_id).await;
    let retry = fixture
        .request(
            Method::POST,
            &format!("/api/v1/ai/jobs/{job_id}/retry"),
            Some(json!({ "idempotencyKey": "reader-retry:api" })),
            Some(UserKind::A),
            true,
        )
        .await;
    assert_eq!(retry.status, StatusCode::CREATED);
    let retry_id = retry.json()["jobId"].as_str().unwrap().to_owned();
    assert_ne!(retry_id, job_id);
    let old = ContentRepository::new(fixture.database.clone())
        .get_job(USER_A_ID, &job_id)
        .await
        .expect("old failed job should remain");
    assert_eq!(old.status().as_storage(), "FAILED");

    let not_retryable = fixture
        .request(
            Method::POST,
            &format!("/api/v1/ai/jobs/{retry_id}/retry"),
            Some(json!({ "idempotencyKey": "reader-retry:not-failed" })),
            Some(UserKind::A),
            true,
        )
        .await;
    assert_error(&not_retryable, StatusCode::CONFLICT, "AI_JOB_NOT_RETRYABLE");

    seed_artifact(
        &fixture.database,
        &retry_id,
        "00000000-0000-4000-8000-000000008101",
        ContentJobOperation::Summarize,
        None,
    )
    .await;
    let result = fixture
        .request(
            Method::GET,
            &format!("/api/v1/ai/jobs/{retry_id}/result"),
            None,
            Some(UserKind::A),
            false,
        )
        .await;
    assert_eq!(result.status, StatusCode::OK);
    assert_eq!(result.json()["kind"], "AI_SUMMARY");
    assert_eq!(result.json()["summary"], "Safe summary");
    assert!(result.json().get("payloadJson").is_none());
    assert!(result.json().get("provenanceJson").is_none());

    let translation = fixture
        .enqueue("TRANSLATE", json!("zh-CN"), "reader:translation-result")
        .await;
    let translation_id = translation.json()["jobId"].as_str().unwrap().to_owned();
    seed_artifact(
        &fixture.database,
        &translation_id,
        "00000000-0000-4000-8000-000000008102",
        ContentJobOperation::Translate,
        Some("zh-CN"),
    )
    .await;
    let translation_result = fixture
        .request(
            Method::GET,
            &format!("/api/v1/ai/jobs/{translation_id}/result"),
            None,
            Some(UserKind::A),
            false,
        )
        .await;
    assert_eq!(translation_result.status, StatusCode::OK);
    assert_eq!(translation_result.json()["kind"], "AI_TRANSLATION");
    assert_eq!(translation_result.json()["targetLocale"], "zh-CN");
    assert_eq!(translation_result.json()["bodyMarkdown"], "Translated body");

    let mismatched_payload = TranslationArtifact::parse(
        br#"{"schemaVersion":1,"detectedSourceLanguage":"en","targetLocale":"ja","title":"Mismatched title","bodyMarkdown":"Mismatched body"}"#,
    )
    .expect("mismatched translation artifact should parse")
    .canonical_json()
    .to_owned();
    content_artifact::Entity::update_many()
        .col_expr(
            content_artifact::Column::PayloadJson,
            Expr::value(mismatched_payload.clone()),
        )
        .col_expr(
            content_artifact::Column::PayloadSizeBytes,
            Expr::value(i32::try_from(mismatched_payload.len()).unwrap()),
        )
        .filter(content_artifact::Column::Id.eq("00000000-0000-4000-8000-000000008102"))
        .exec(&fixture.database)
        .await
        .expect("translation artifact locale should drift");
    let corrupt = fixture
        .request(
            Method::GET,
            &format!("/api/v1/ai/jobs/{translation_id}/result"),
            None,
            Some(UserKind::A),
            false,
        )
        .await;
    assert_error(
        &corrupt,
        StatusCode::INTERNAL_SERVER_ERROR,
        "INTERNAL_ERROR",
    );
    assert!(!String::from_utf8_lossy(&corrupt.body).contains("Mismatched body"));
}

#[tokio::test]
async fn overview_exposes_unavailable_disabled_retry_failed_and_succeeded_states() {
    let missing_plugin = AiContentFixture::new(true, false, false).await;
    let response = missing_plugin
        .request(Method::GET, ENTRY_AI_PATH, None, Some(UserKind::A), false)
        .await;
    assert_eq!(response.json()["availability"], "PLUGIN_UNAVAILABLE");
    assert_eq!(response.json()["summary"]["state"], "UNAVAILABLE");

    let not_configured = AiContentFixture::new(true, true, false).await;
    let response = not_configured
        .request(Method::GET, ENTRY_AI_PATH, None, Some(UserKind::A), false)
        .await;
    assert_eq!(response.json()["availability"], "NOT_CONFIGURED");

    let disabled = AiContentFixture::new(true, true, true).await;
    plugin_config::Entity::update_many()
        .col_expr(plugin_config::Column::IsEnabled, Expr::value(false))
        .exec(&disabled.database)
        .await
        .expect("config should disable");
    let response = disabled
        .request(Method::GET, ENTRY_AI_PATH, None, Some(UserKind::A), false)
        .await;
    assert_eq!(response.json()["availability"], "DISABLED");
    assert_eq!(response.json()["summary"]["state"], "DISABLED");

    let provider_unavailable = AiContentFixture::new(true, true, true).await;
    raindrop::db::entities::ai_provider::Entity::update_many()
        .col_expr(
            raindrop::db::entities::ai_provider::Column::IsEnabled,
            Expr::value(false),
        )
        .filter(
            raindrop::db::entities::ai_provider::Column::Id.eq(&provider_unavailable.provider_id),
        )
        .exec(&provider_unavailable.database)
        .await
        .expect("provider should disable");
    let response = provider_unavailable
        .request(Method::GET, ENTRY_AI_PATH, None, Some(UserKind::A), false)
        .await;
    assert_eq!(response.json()["availability"], "PROVIDER_UNAVAILABLE");
    assert_eq!(response.json()["summary"]["state"], "UNAVAILABLE");

    let jobs = AiContentFixture::new(true, true, true).await;
    let queued = jobs
        .enqueue("SUMMARIZE", Value::Null, "reader:state-machine")
        .await;
    let job_id = queued.json()["jobId"].as_str().unwrap().to_owned();
    set_job_state(&jobs.database, &job_id, "RUNNING", None).await;
    let running = jobs
        .request(Method::GET, ENTRY_AI_PATH, None, Some(UserKind::A), false)
        .await;
    assert_eq!(running.json()["summary"]["state"], "RUNNING");
    set_job_state(&jobs.database, &job_id, "RETRY_WAIT", None).await;
    let retry_wait = jobs
        .request(Method::GET, ENTRY_AI_PATH, None, Some(UserKind::A), false)
        .await;
    assert_eq!(retry_wait.json()["summary"]["state"], "RETRY_WAIT");
    set_job_state(
        &jobs.database,
        &job_id,
        "FAILED",
        Some("PROVIDER_UNAVAILABLE"),
    )
    .await;
    let failed = jobs
        .request(Method::GET, ENTRY_AI_PATH, None, Some(UserKind::A), false)
        .await;
    assert_eq!(failed.json()["summary"]["state"], "FAILED");
    assert_eq!(
        failed.json()["summary"]["job"]["lastErrorCode"],
        "PROVIDER_UNAVAILABLE"
    );
    seed_artifact(
        &jobs.database,
        &job_id,
        "00000000-0000-4000-8000-000000008103",
        ContentJobOperation::Summarize,
        None,
    )
    .await;
    let succeeded = jobs
        .request(Method::GET, ENTRY_AI_PATH, None, Some(UserKind::A), false)
        .await;
    assert_eq!(succeeded.json()["summary"]["state"], "SUCCEEDED");
    assert_eq!(
        succeeded.json()["summary"]["artifact"]["kind"],
        "AI_SUMMARY"
    );
}

#[tokio::test]
async fn content_routes_enforce_auth_csrf_tenant_paths_methods_rate_and_cache() {
    let fixture = AiContentFixture::new(true, true, true).await;
    let unauthenticated = fixture
        .request(Method::GET, ENTRY_AI_PATH, None, None, false)
        .await;
    assert_error(
        &unauthenticated,
        StatusCode::UNAUTHORIZED,
        "AUTHENTICATION_REQUIRED",
    );
    let missing_csrf = fixture
        .request(
            Method::POST,
            &format!("{ENTRY_AI_PATH}/jobs"),
            Some(json!({
                "operation": "SUMMARIZE",
                "targetLocale": null,
                "idempotencyKey": "reader:missing-csrf"
            })),
            Some(UserKind::A),
            false,
        )
        .await;
    assert_error(&missing_csrf, StatusCode::FORBIDDEN, "FORBIDDEN");

    let cross_entry = fixture
        .request(Method::GET, ENTRY_AI_PATH, None, Some(UserKind::B), false)
        .await;
    assert_error(&cross_entry, StatusCode::NOT_FOUND, "NOT_FOUND");
    let queued = fixture
        .enqueue("SUMMARIZE", Value::Null, "reader:tenant-job")
        .await;
    let job_id = queued.json()["jobId"].as_str().unwrap().to_owned();
    let cross_job = fixture
        .request(
            Method::GET,
            &format!("/api/v1/ai/jobs/{job_id}"),
            None,
            Some(UserKind::B),
            false,
        )
        .await;
    assert_error(&cross_job, StatusCode::NOT_FOUND, "NOT_FOUND");

    let malformed = fixture
        .request(
            Method::GET,
            "/api/v1/ai/jobs/not-a-job",
            None,
            Some(UserKind::A),
            false,
        )
        .await;
    assert_error(
        &malformed,
        StatusCode::UNPROCESSABLE_ENTITY,
        "VALIDATION_ERROR",
    );
    let unknown = fixture
        .request(
            Method::GET,
            &format!("/api/v1/ai/jobs/{job_id}/unknown"),
            None,
            Some(UserKind::A),
            false,
        )
        .await;
    assert_error(&unknown, StatusCode::NOT_FOUND, "NOT_FOUND");
    let trailing = fixture
        .request(
            Method::GET,
            &format!("/api/v1/ai/jobs/{job_id}/"),
            None,
            Some(UserKind::A),
            false,
        )
        .await;
    assert_error(&trailing, StatusCode::NOT_FOUND, "NOT_FOUND");
    let method = fixture
        .request(
            Method::DELETE,
            &format!("/api/v1/ai/jobs/{job_id}"),
            None,
            Some(UserKind::A),
            false,
        )
        .await;
    assert_error(
        &method,
        StatusCode::METHOD_NOT_ALLOWED,
        "METHOD_NOT_ALLOWED",
    );

    for index in 0..29 {
        let response = fixture
            .request(
                Method::POST,
                &format!("/api/v1/ai/jobs/{job_id}/retry"),
                Some(json!({ "idempotencyKey": format!("reader-retry:rate-{index}") })),
                Some(UserKind::A),
                true,
            )
            .await;
        assert_error(&response, StatusCode::CONFLICT, "AI_JOB_NOT_RETRYABLE");
    }
    let limited = fixture
        .request(
            Method::POST,
            &format!("/api/v1/ai/jobs/{job_id}/retry"),
            Some(json!({ "idempotencyKey": "reader-retry:limited" })),
            Some(UserKind::A),
            true,
        )
        .await;
    assert_error(&limited, StatusCode::TOO_MANY_REQUESTS, "RATE_LIMITED");
    assert!(limited.headers.contains_key(RETRY_AFTER));
}

fn provider_input() -> CreateProvider {
    CreateProvider {
        scope: ProviderScope::user(USER_A_ID).unwrap(),
        display_name: "AI content API provider".to_owned(),
        kind: ProviderKind::OpenAiResponses,
        endpoint: None,
        model: "gpt-test-model".to_owned(),
        credential: SecretString::from("ai-content-provider-secret-sentinel"),
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
        is_enabled: true,
    }
}

fn config_json(provider_id: &str) -> Vec<u8> {
    serde_json::to_vec(&json!({
        "schemaVersion": 1,
        "operations": {
            "summarize": {
                "enabled": true,
                "providerId": provider_id,
                "style": "BALANCED",
                "maxOutputTokens": 1024,
                "mcp": {
                    "mode": "DISABLED",
                    "failurePolicy": "FAIL_OPEN",
                    "maxToolCalls": 0,
                    "tools": []
                }
            },
            "translate": {
                "enabled": true,
                "providerId": provider_id,
                "defaultTargetLocale": "zh-CN",
                "maxOutputTokens": 4096,
                "mcp": {
                    "mode": "DISABLED",
                    "failurePolicy": "FAIL_OPEN",
                    "maxToolCalls": 0,
                    "tools": []
                }
            }
        },
        "automatic": {
            "enabled": false,
            "operations": ["SUMMARIZE", "TRANSLATE"],
            "allSubscribedFeeds": false,
            "feedIds": [],
            "categoryIds": []
        }
    }))
    .expect("AI content config should serialize")
}

fn provider_keyring() -> ProviderSecretKeyring {
    let key = SecretString::from(format!("primary:{}", URL_SAFE_NO_PAD.encode([0x41_u8; 32])));
    ProviderSecretKeyring::from_entries(&[key]).expect("AI content keyring should construct")
}

fn session_cookie(session: &raindrop::auth::CreatedSession) -> String {
    build_session_cookie(session, false)
        .to_string()
        .split(';')
        .next()
        .expect("session cookie should have a name/value pair")
        .to_owned()
}

async fn mark_job_failed(database: &DatabaseConnection, job_id: &str) {
    set_job_state(database, job_id, "FAILED", Some("PROVIDER_UNAVAILABLE")).await;
}

async fn set_job_state(
    database: &DatabaseConnection,
    job_id: &str,
    status: &str,
    error_code: Option<&str>,
) {
    content_job::Entity::update_many()
        .col_expr(content_job::Column::Status, Expr::value(status.to_owned()))
        .col_expr(
            content_job::Column::LastErrorCode,
            Expr::value(error_code.map(str::to_owned)),
        )
        .col_expr(
            content_job::Column::CompletedAt,
            Expr::value(matches!(status, "FAILED" | "SUCCEEDED").then(OffsetDateTime::now_utc)),
        )
        .filter(content_job::Column::Id.eq(job_id))
        .exec(database)
        .await
        .expect("job state should update");
}

async fn seed_artifact(
    database: &DatabaseConnection,
    job_id: &str,
    artifact_id: &str,
    operation: ContentJobOperation,
    target_locale: Option<&str>,
) {
    let repository = ContentRepository::new(database.clone());
    let job = repository
        .get_job(USER_A_ID, job_id)
        .await
        .expect("artifact job should load");
    assert_eq!(job.operation(), operation);
    assert_eq!(job.identity().target_locale(), target_locale);
    set_job_state(database, job_id, "SUCCEEDED", None).await;
    let identity = job.identity();
    let payload = match operation {
        ContentJobOperation::Summarize => SummaryArtifact::parse(
            br#"{"schemaVersion":1,"sourceLanguage":"en","summary":"Safe summary","bullets":["Point"],"conclusion":null}"#,
        )
        .expect("summary artifact should parse")
        .canonical_json()
        .to_owned(),
        ContentJobOperation::Translate => TranslationArtifact::parse(
            br#"{"schemaVersion":1,"detectedSourceLanguage":"en","targetLocale":"zh-CN","title":"Translated title","bodyMarkdown":"Translated body"}"#,
        )
        .expect("translation artifact should parse")
        .canonical_json()
        .to_owned(),
    };
    let now = OffsetDateTime::now_utc();
    content_artifact::ActiveModel {
        id: Set(artifact_id.to_owned()),
        user_id: Set(identity.user_id().to_owned()),
        entry_id: Set(identity.entry_id().to_owned()),
        producer_job_id: Set(job_id.to_owned()),
        kind: Set(identity.kind().as_storage().to_owned()),
        locale: Set(identity.target_locale().map(str::to_owned)),
        schema_id: Set(identity.schema_id().to_owned()),
        entry_content_hash: Set(identity.entry_content_hash().to_owned()),
        input_hash: Set(identity.input_hash().to_owned()),
        config_hash: Set(identity.config_hash().to_owned()),
        processor_key: Set(identity.plugin_key().to_owned()),
        processor_version: Set(identity.plugin_version().to_owned()),
        component_digest: Set(identity.component_digest().to_owned()),
        provider_binding_id: Set(identity.provider_binding_id().to_owned()),
        provider_kind: Set(identity.provider_kind().as_storage().to_owned()),
        provider_model: Set(identity.provider_model().to_owned()),
        provider_revision: Set(i64::try_from(identity.provider_revision()).unwrap()),
        provider_label: Set("gpt-test-model".to_owned()),
        prompt_version: Set(identity.prompt_version().to_owned()),
        mcp_provenance_hash: Set(identity.mcp_provenance_hash().to_owned()),
        identity_hash: Set(identity.hash().to_owned()),
        payload_size_bytes: Set(i32::try_from(payload.len()).unwrap()),
        payload_json: Set(payload),
        provenance_json: Set("{\"schemaVersion\":1}".to_owned()),
        created_at: Set(now),
    }
    .insert(database)
    .await
    .expect("AI artifact should insert");
    content_job_result::ActiveModel {
        job_id: Set(job_id.to_owned()),
        artifact_id: Set(artifact_id.to_owned()),
        was_reused: Set(false),
        linked_at: Set(now),
    }
    .insert(database)
    .await
    .expect("AI job result should insert");
}

fn assert_public_overview(response: &CapturedResponse) {
    assert_sensitive_cache_headers(response);
    let serialized = String::from_utf8_lossy(&response.body);
    for forbidden in [
        "payloadJson",
        "provenanceJson",
        "configHash",
        "identityHash",
        "entryContentHash",
        "providerEndpoint",
        "credential",
    ] {
        assert!(
            !serialized.contains(forbidden),
            "overview leaked {forbidden}"
        );
    }
}

fn assert_safe_job(response: &CapturedResponse) {
    assert_sensitive_cache_headers(response);
    let value = response.json();
    let keys = value
        .as_object()
        .unwrap()
        .keys()
        .cloned()
        .collect::<Vec<_>>();
    assert_eq!(
        keys.len(),
        [
            "jobId",
            "status",
            "attempts",
            "maxAttempts",
            "nextAttemptAt",
            "lastErrorCode",
            "createdAt",
            "startedAt",
            "completedAt"
        ]
        .len()
    );
    let serialized = value.to_string();
    for forbidden in [
        "payloadJson",
        "provenanceJson",
        "configHash",
        "identityHash",
        "providerModel",
        "providerEndpoint",
    ] {
        assert!(!serialized.contains(forbidden), "job leaked {forbidden}");
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

async fn job_count(database: &DatabaseConnection) -> u64 {
    content_job::Entity::find()
        .count(database)
        .await
        .expect("AI jobs should count")
}
