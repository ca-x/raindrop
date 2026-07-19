#[allow(dead_code)]
mod support;

use std::{collections::BTreeSet, fs, sync::Arc};

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
    content::provider::{
        CreateProvider, ProviderCapabilities, ProviderKind, ProviderPolicy, ProviderRepository,
        ProviderScope, ProviderSecretKeyring,
    },
    db::{DatabaseConfig, connect, entities::plugin_config, migrate},
    plugins::PluginRegistryRepository,
    setup::SetupService,
};
use sea_orm::{DatabaseConnection, EntityTrait, sea_query::Expr};
use secrecy::{ExposeSecret, SecretString};
use serde_json::{Value, json};
use support::{
    database::{
        ENTRY_A_ID, HASH_D, SUBSCRIPTION_A_ID, USER_A_ID, insert_entry, insert_feed,
        insert_subscription, insert_user,
    },
    plugin::signed_bundle,
};
use tempfile::TempDir;
use time::macros::datetime;
use tower::ServiceExt;

const OPENAPI_PATH: &str = "docs/openapi/ai-content-v1.json";
const ENTRY_AI_PATH: &str = "/api/v1/entries/{entryId}/ai";
const ENTRY_AI_JOBS_PATH: &str = "/api/v1/entries/{entryId}/ai/jobs";
const JOB_PATH: &str = "/api/v1/ai/jobs/{jobId}";
const RESULT_PATH: &str = "/api/v1/ai/jobs/{jobId}/result";
const RETRY_PATH: &str = "/api/v1/ai/jobs/{jobId}/retry";
const CONFIG_PATH: &str = "/api/v1/ai/config";
const REAL_ENTRY_AI_PATH: &str = "/api/v1/entries/00000000-0000-4000-8000-000000000301/ai";
const METHODS: [&str; 8] = [
    "get", "put", "post", "delete", "options", "head", "patch", "trace",
];

#[test]
fn ai_content_openapi_freezes_routes_strict_schemas_and_safe_fields() {
    let document = load_openapi();
    assert_eq!(document["openapi"], "3.1.0");
    assert_eq!(
        documented_operations(&document),
        BTreeSet::from([
            ("GET".to_owned(), CONFIG_PATH.to_owned()),
            ("PUT".to_owned(), CONFIG_PATH.to_owned()),
            ("GET".to_owned(), ENTRY_AI_PATH.to_owned()),
            ("POST".to_owned(), ENTRY_AI_JOBS_PATH.to_owned()),
            ("GET".to_owned(), JOB_PATH.to_owned()),
            ("GET".to_owned(), RESULT_PATH.to_owned()),
            ("POST".to_owned(), RETRY_PATH.to_owned()),
        ])
    );
    assert_statuses(&document, CONFIG_PATH, "get", &[200, 401, 500]);
    assert_statuses(
        &document,
        CONFIG_PATH,
        "put",
        &[200, 401, 403, 404, 409, 422, 429, 500],
    );
    assert_statuses(&document, ENTRY_AI_PATH, "get", &[200, 401, 404, 422, 500]);
    assert_statuses(
        &document,
        ENTRY_AI_JOBS_PATH,
        "post",
        &[200, 201, 401, 403, 404, 409, 422, 429, 500, 503],
    );
    assert_statuses(&document, JOB_PATH, "get", &[200, 401, 404, 422, 500]);
    assert_statuses(
        &document,
        RESULT_PATH,
        "get",
        &[200, 401, 404, 409, 422, 500],
    );
    assert_statuses(
        &document,
        RETRY_PATH,
        "post",
        &[200, 201, 401, 403, 404, 409, 422, 429, 500, 503],
    );

    for schema_name in [
        "AiConfigEnvelope",
        "AiJob",
        "AiSummaryArtifact",
        "AiTranslationArtifact",
        "AiOperationOverview",
        "EntryAiOverview",
        "EnqueueAiJobRequest",
        "RetryAiJobRequest",
    ] {
        assert_eq!(
            document["components"]["schemas"][schema_name]["additionalProperties"], false,
            "{schema_name} should be strict"
        );
    }
    assert_eq!(
        document["components"]["schemas"]["AiJob"]["properties"]["maxAttempts"]["enum"],
        json!([3])
    );
    assert_eq!(
        document["components"]["schemas"]["AiOperationState"]["enum"],
        json!([
            "UNAVAILABLE",
            "DISABLED",
            "IDLE",
            "QUEUED",
            "RUNNING",
            "RETRY_WAIT",
            "SUCCEEDED",
            "FAILED"
        ])
    );
    assert_eq!(
        document["components"]["schemas"]["AiArtifact"]["anyOf"]
            .as_array()
            .expect("artifact variants should be an array")
            .len(),
        2
    );
    for response_name in [
        "AiConfig",
        "AiOverview",
        "AiJob",
        "CreatedAiJob",
        "AiArtifact",
        "Error",
        "RateLimited",
    ] {
        let headers = &document["components"]["responses"][response_name]["headers"];
        assert_eq!(
            headers["Cache-Control"]["$ref"],
            "#/components/headers/CacheControl"
        );
        assert_eq!(headers["Pragma"]["$ref"], "#/components/headers/Pragma");
    }
    assert_eq!(
        document["components"]["responses"]["CreatedAiJob"]["headers"]["Location"]["$ref"],
        "#/components/headers/Location"
    );
    assert_all_local_refs_resolve(&document, &document);

    let public_schemas = serde_json::to_string(&json!({
        "overview": document["components"]["schemas"]["EntryAiOverview"].clone(),
        "job": document["components"]["schemas"]["AiJob"].clone(),
        "artifact": document["components"]["schemas"]["AiArtifact"].clone()
    }))
    .expect("public AI schemas should serialize");
    for forbidden in [
        "payloadJson",
        "provenanceJson",
        "canonicalJson",
        "configHash",
        "identityHash",
        "entryContentHash",
        "providerEndpoint",
        "credential",
    ] {
        assert!(
            !public_schemas.contains(forbidden),
            "AI content OpenAPI leaks {forbidden}"
        );
    }
}

#[tokio::test]
async fn ai_content_openapi_matches_representative_real_router_responses() {
    let fixture = ContractFixture::new(true).await;
    let document = load_openapi();

    let unauthenticated = fixture
        .request(Method::GET, REAL_ENTRY_AI_PATH, None, false, false)
        .await;
    assert_observed(&document, ENTRY_AI_PATH, "get", &unauthenticated, 401);

    let overview = fixture
        .request(Method::GET, REAL_ENTRY_AI_PATH, None, true, false)
        .await;
    assert_observed(&document, ENTRY_AI_PATH, "get", &overview, 200);

    let missing_csrf = fixture
        .request(
            Method::POST,
            &format!("{REAL_ENTRY_AI_PATH}/jobs"),
            Some(enqueue_body("contract-missing-csrf")),
            true,
            false,
        )
        .await;
    assert_observed(&document, ENTRY_AI_JOBS_PATH, "post", &missing_csrf, 403);

    let invalid = fixture
        .request(
            Method::POST,
            &format!("{REAL_ENTRY_AI_PATH}/jobs"),
            Some(json!({ "operation": "SUMMARIZE" })),
            true,
            true,
        )
        .await;
    assert_observed(&document, ENTRY_AI_JOBS_PATH, "post", &invalid, 422);

    let created = fixture
        .request(
            Method::POST,
            &format!("{REAL_ENTRY_AI_PATH}/jobs"),
            Some(enqueue_body("contract-created")),
            true,
            true,
        )
        .await;
    assert_observed(&document, ENTRY_AI_JOBS_PATH, "post", &created, 201);
    let job_id = created.json()["jobId"].as_str().unwrap().to_owned();
    assert_eq!(
        created
            .headers
            .get(LOCATION)
            .and_then(|value| value.to_str().ok()),
        Some(format!("/api/v1/ai/jobs/{job_id}").as_str())
    );

    let existing = fixture
        .request(
            Method::POST,
            &format!("{REAL_ENTRY_AI_PATH}/jobs"),
            Some(enqueue_body("contract-created")),
            true,
            true,
        )
        .await;
    assert_observed(&document, ENTRY_AI_JOBS_PATH, "post", &existing, 200);

    let missing = fixture
        .request(
            Method::GET,
            "/api/v1/ai/jobs/00000000-0000-4000-8000-000000009999",
            None,
            true,
            false,
        )
        .await;
    assert_observed(&document, JOB_PATH, "get", &missing, 404);

    let not_ready = fixture
        .request(
            Method::GET,
            &format!("/api/v1/ai/jobs/{job_id}/result"),
            None,
            true,
            false,
        )
        .await;
    assert_observed(&document, RESULT_PATH, "get", &not_ready, 409);

    let no_keyring = ContractFixture::new(false).await;
    let unavailable = no_keyring
        .request(
            Method::POST,
            &format!("{REAL_ENTRY_AI_PATH}/jobs"),
            Some(enqueue_body("contract-no-keyring")),
            true,
            true,
        )
        .await;
    assert_observed(&document, ENTRY_AI_JOBS_PATH, "post", &unavailable, 503);

    let corrupt = ContractFixture::new(true).await;
    corrupt.corrupt_config_hash().await;
    let internal = corrupt
        .request(Method::GET, REAL_ENTRY_AI_PATH, None, true, false)
        .await;
    assert_observed(&document, ENTRY_AI_PATH, "get", &internal, 500);

    let rate = ContractFixture::new(true).await;
    let queued = rate
        .request(
            Method::POST,
            &format!("{REAL_ENTRY_AI_PATH}/jobs"),
            Some(enqueue_body("contract-rate-job")),
            true,
            true,
        )
        .await;
    let rate_job_id = queued.json()["jobId"].as_str().unwrap().to_owned();
    for index in 0..29 {
        let admitted = rate
            .request(
                Method::POST,
                &format!("/api/v1/ai/jobs/{rate_job_id}/retry"),
                Some(json!({ "idempotencyKey": format!("contract-rate-{index}") })),
                true,
                true,
            )
            .await;
        assert_eq!(admitted.status, StatusCode::CONFLICT);
    }
    let limited = rate
        .request(
            Method::POST,
            &format!("/api/v1/ai/jobs/{rate_job_id}/retry"),
            Some(json!({ "idempotencyKey": "contract-rate-limited" })),
            true,
            true,
        )
        .await;
    assert_observed(&document, RETRY_PATH, "post", &limited, 429);
    assert!(limited.headers.contains_key(RETRY_AFTER));
}

struct ContractFixture {
    _data: TempDir,
    database: DatabaseConnection,
    app: Router,
    cookie: String,
    csrf: String,
}

impl ContractFixture {
    async fn new(app_has_keyring: bool) -> Self {
        let data = tempfile::tempdir().expect("temporary AI content OpenAPI directory");
        let url = format!(
            "sqlite://{}?mode=rwc",
            data.path().join("ai-content-openapi.db").display()
        );
        let database = connect(&DatabaseConfig::new(SecretString::from(url)))
            .await
            .expect("AI content OpenAPI database should connect");
        migrate(&database)
            .await
            .expect("AI content OpenAPI database should migrate");
        let now = datetime!(2026-07-19 12:00:00 UTC);
        insert_user(&database, USER_A_ID, "ai-content-openapi").await;
        insert_feed(&database, now).await;
        insert_subscription(&database, SUBSCRIPTION_A_ID, USER_A_ID, now).await;
        insert_entry(
            &database,
            ENTRY_A_ID,
            1,
            "ai-content-openapi-entry",
            HASH_D,
            Some(1_752_926_400_000_000),
            now,
        )
        .await;
        let registry = PluginRegistryRepository::new(database.clone());
        registry
            .sync_bundled(&signed_bundle("1.0.0", b"AI content OpenAPI component"))
            .await
            .expect("official AI plugin should install");
        let keyring = Arc::new(provider_keyring());
        let providers = ProviderRepository::new(database.clone(), Some(Arc::clone(&keyring)));
        let provider = providers
            .create(provider_input())
            .await
            .expect("AI content OpenAPI provider should create");
        registry
            .replace_ai_config(
                "raindrop.ai-content",
                USER_A_ID,
                None,
                true,
                &config_json(provider.id()),
            )
            .await
            .expect("AI content OpenAPI config should create");
        let setup = SetupService::ready(data.path(), None, database.clone());
        let session = setup
            .sessions()
            .create(USER_A_ID)
            .await
            .expect("AI content OpenAPI session should create");
        let cookie = build_session_cookie(&session, false)
            .to_string()
            .split(';')
            .next()
            .expect("session cookie should contain a pair")
            .to_owned();
        let csrf = session.csrf_token.expose_secret().to_owned();
        let state = AppState::new(setup).with_provider_keyring(app_has_keyring.then_some(keyring));
        Self {
            _data: data,
            database,
            app: build_router(state),
            cookie,
            csrf,
        }
    }

    async fn request(
        &self,
        method: Method,
        uri: &str,
        body: Option<Value>,
        authenticated: bool,
        csrf: bool,
    ) -> CapturedResponse {
        let mut request = Request::builder().method(method).uri(uri);
        if authenticated {
            request = request.header(COOKIE, &self.cookie);
        }
        if csrf {
            request = request
                .header("x-csrf-token", &self.csrf)
                .header(ORIGIN, "http://ai-content-openapi.test")
                .header(HOST, "ai-content-openapi.test");
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
                    .expect("AI content OpenAPI request should build"),
            )
            .await
            .expect("AI content OpenAPI request should complete");
        CapturedResponse::from_response(response).await
    }

    async fn corrupt_config_hash(&self) {
        plugin_config::Entity::update_many()
            .col_expr(
                plugin_config::Column::ConfigHash,
                Expr::value("0".repeat(64)),
            )
            .exec(&self.database)
            .await
            .expect("AI config hash should corrupt");
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
                .expect("AI content OpenAPI body should collect")
                .to_bytes()
                .to_vec(),
        }
    }

    fn json(&self) -> Value {
        serde_json::from_slice(&self.body).expect("AI content OpenAPI response should be JSON")
    }
}

fn enqueue_body(key: &str) -> Value {
    json!({
        "operation": "SUMMARIZE",
        "targetLocale": null,
        "idempotencyKey": key
    })
}

fn provider_input() -> CreateProvider {
    CreateProvider {
        scope: ProviderScope::user(USER_A_ID).unwrap(),
        display_name: "AI content OpenAPI provider".to_owned(),
        kind: ProviderKind::OpenAiResponses,
        endpoint: None,
        model: "gpt-test-model".to_owned(),
        credential: SecretString::from("ai-content-openapi-secret-sentinel"),
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
    .expect("AI content OpenAPI config should serialize")
}

fn provider_keyring() -> ProviderSecretKeyring {
    let key = SecretString::from(format!("primary:{}", URL_SAFE_NO_PAD.encode([0x41_u8; 32])));
    ProviderSecretKeyring::from_entries(&[key])
        .expect("AI content OpenAPI keyring should construct")
}

fn assert_observed(
    document: &Value,
    path: &str,
    method: &str,
    response: &CapturedResponse,
    status: u16,
) {
    assert_eq!(response.status.as_u16(), status);
    assert!(
        document["paths"][path][method]["responses"]
            .get(status.to_string())
            .is_some()
    );
    assert_eq!(response.headers.get(CACHE_CONTROL).unwrap(), "no-store");
    assert_eq!(response.headers.get(PRAGMA).unwrap(), "no-cache");
}

fn load_openapi() -> Value {
    let artifact = fs::read_to_string(OPENAPI_PATH)
        .unwrap_or_else(|error| panic!("AI content OpenAPI should exist: {error}"));
    serde_json::from_str(&artifact).expect("AI content OpenAPI should be valid JSON")
}

fn documented_operations(document: &Value) -> BTreeSet<(String, String)> {
    let mut operations = BTreeSet::new();
    for (path, item) in document["paths"].as_object().unwrap() {
        for method in METHODS {
            if item.get(method).is_some() {
                operations.insert((method.to_ascii_uppercase(), path.clone()));
            }
        }
    }
    operations
}

fn assert_statuses(document: &Value, path: &str, method: &str, expected: &[u16]) {
    let actual = document["paths"][path][method]["responses"]
        .as_object()
        .unwrap()
        .keys()
        .map(|status| status.parse::<u16>().unwrap())
        .collect::<BTreeSet<_>>();
    assert_eq!(actual, expected.iter().copied().collect());
}

fn assert_all_local_refs_resolve(root: &Value, value: &Value) {
    match value {
        Value::Object(object) => {
            if let Some(reference) = object.get("$ref").and_then(Value::as_str) {
                let pointer = reference
                    .strip_prefix('#')
                    .expect("references should be local");
                assert!(
                    root.pointer(pointer).is_some(),
                    "unresolved ref {reference}"
                );
            }
            for nested in object.values() {
                assert_all_local_refs_resolve(root, nested);
            }
        }
        Value::Array(values) => {
            for nested in values {
                assert_all_local_refs_resolve(root, nested);
            }
        }
        _ => {}
    }
}
