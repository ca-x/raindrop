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
    db::{DatabaseConfig, connect, migrate},
    setup::SetupService,
};
use sea_orm::{ConnectionTrait, DatabaseConnection, DbBackend, Statement};
use secrecy::{ExposeSecret, SecretString};
use serde_json::{Value, json};
use support::database::{USER_A_ID, insert_user};
use tempfile::TempDir;
use tower::ServiceExt;

const OPENAPI_PATH: &str = "docs/openapi/ai-provider-v1.json";
const PROVIDERS_PATH: &str = "/api/v1/ai/providers";
const PROVIDER_PATH: &str = "/api/v1/ai/providers/{providerId}";
const METHODS: [&str; 8] = [
    "get", "put", "post", "delete", "options", "head", "patch", "trace",
];

#[test]
fn provider_openapi_freezes_the_public_surface_and_secret_boundary() {
    let document = load_openapi();
    assert_eq!(document["openapi"], "3.1.0");
    assert_eq!(
        documented_operations(&document),
        BTreeSet::from([
            ("GET".to_owned(), PROVIDERS_PATH.to_owned()),
            ("POST".to_owned(), PROVIDERS_PATH.to_owned()),
            ("GET".to_owned(), PROVIDER_PATH.to_owned()),
            ("PATCH".to_owned(), PROVIDER_PATH.to_owned()),
        ])
    );
    assert_statuses(&document, PROVIDERS_PATH, "get", &[200, 401, 500]);
    assert_statuses(
        &document,
        PROVIDERS_PATH,
        "post",
        &[201, 401, 403, 409, 422, 429, 500, 503],
    );
    assert_statuses(&document, PROVIDER_PATH, "get", &[200, 401, 404, 422, 500]);
    assert_statuses(
        &document,
        PROVIDER_PATH,
        "patch",
        &[200, 401, 403, 404, 409, 422, 429, 500, 503],
    );

    for (path, method) in [
        (PROVIDERS_PATH, "get"),
        (PROVIDERS_PATH, "post"),
        (PROVIDER_PATH, "get"),
        (PROVIDER_PATH, "patch"),
    ] {
        assert_eq!(
            document["paths"][path][method]["security"],
            json!([{ "sessionCookie": [] }])
        );
    }
    for (path, method) in [(PROVIDERS_PATH, "post"), (PROVIDER_PATH, "patch")] {
        assert!(
            document["paths"][path][method]["parameters"]
                .as_array()
                .expect("mutation parameters should be an array")
                .iter()
                .any(|parameter| parameter["$ref"] == "#/components/parameters/CsrfToken")
        );
    }

    let provider = &document["components"]["schemas"]["Provider"];
    assert_eq!(provider["additionalProperties"], false);
    assert_required(
        &document,
        "Provider",
        &[
            "providerId",
            "scope",
            "canEdit",
            "displayName",
            "kind",
            "endpoint",
            "model",
            "capabilities",
            "policy",
            "isEnabled",
            "revision",
            "createdAt",
            "updatedAt",
        ],
    );
    assert_exact_properties(
        provider,
        &[
            "providerId",
            "scope",
            "canEdit",
            "displayName",
            "kind",
            "endpoint",
            "model",
            "capabilities",
            "policy",
            "isEnabled",
            "revision",
            "createdAt",
            "updatedAt",
        ],
    );
    assert_eq!(
        document["components"]["schemas"]["ProviderKind"]["enum"],
        json!([
            "ANTHROPIC_MESSAGES",
            "OPENAI_RESPONSES",
            "OPENAI_CHAT_COMPLETIONS",
            "GOOGLE_GEMINI"
        ])
    );
    assert_eq!(
        provider["properties"]["scope"]["enum"],
        json!(["INSTANCE", "USER"])
    );

    let list = &document["components"]["schemas"]["ProviderList"];
    assert_required(&document, "ProviderList", &["keyringStatus", "items"]);
    assert_eq!(
        list["properties"]["keyringStatus"]["enum"],
        json!(["AVAILABLE", "UNAVAILABLE"])
    );

    let create = &document["components"]["schemas"]["CreateProviderRequest"];
    assert_eq!(create["additionalProperties"], false);
    assert_required(
        &document,
        "CreateProviderRequest",
        &[
            "displayName",
            "kind",
            "endpoint",
            "model",
            "credential",
            "capabilities",
            "policy",
            "isEnabled",
        ],
    );
    assert_eq!(create["properties"]["credential"]["writeOnly"], true);
    assert_eq!(create["properties"]["credential"]["maxLength"], 8192);
    assert_eq!(
        create["properties"]["endpoint"]["type"],
        json!(["string", "null"])
    );

    let patch = &document["components"]["schemas"]["UpdateProviderRequest"];
    assert_eq!(patch["additionalProperties"], false);
    assert_eq!(patch["minProperties"], 2);
    assert_required(&document, "UpdateProviderRequest", &["expectedRevision"]);
    assert_eq!(patch["properties"]["credential"]["writeOnly"], true);
    assert_eq!(
        patch["properties"]["credential"]["type"],
        json!(["string", "null"])
    );

    let policy = &document["components"]["schemas"]["ProviderPolicy"];
    assert_eq!(policy["properties"]["maxConcurrency"]["maximum"], 64);
    assert_eq!(
        policy["properties"]["requestsPerMinute"]["maximum"],
        1_000_000
    );
    assert_eq!(
        policy["properties"]["maxInputTokensPerRequest"]["maximum"],
        1_048_576
    );
    assert_eq!(
        policy["properties"]["maxOutputTokensPerRequest"]["maximum"],
        16_384
    );

    for response_name in [
        "ProviderList",
        "Provider",
        "CreatedProvider",
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
        document["components"]["responses"]["CreatedProvider"]["headers"]["Location"]["$ref"],
        "#/components/headers/Location"
    );
    assert_eq!(
        document["components"]["responses"]["RateLimited"]["headers"]["Retry-After"]["$ref"],
        "#/components/headers/RetryAfter"
    );
    assert_required(&document, "ApiError", &["code", "message", "requestId"]);
    assert_all_local_refs_resolve(&document, &document);

    for schema_name in ["Provider", "ProviderList"] {
        let serialized = serde_json::to_string(&document["components"]["schemas"][schema_name])
            .expect("provider response schema should serialize")
            .to_ascii_lowercase();
        for forbidden in [
            "credential",
            "encryptedsecret",
            "encrypted_secret",
            "keyid",
            "key_id",
            "owneruserid",
            "owner_user_id",
            "rawconfig",
        ] {
            assert!(
                !serialized.contains(forbidden),
                "provider response schema leaks internal field {forbidden}"
            );
        }
    }
}

#[tokio::test]
async fn provider_openapi_matches_representative_real_router_responses() {
    let fixture = ContractFixture::new(true).await;
    let document = load_openapi();

    let unauthenticated = fixture
        .request(Method::GET, PROVIDERS_PATH, None, false, false)
        .await;
    assert_observed(&document, PROVIDERS_PATH, "get", &unauthenticated, 401);
    assert_error_envelope(&unauthenticated.json());

    let listed = fixture
        .request(Method::GET, PROVIDERS_PATH, None, true, false)
        .await;
    assert_observed(&document, PROVIDERS_PATH, "get", &listed, 200);
    assert_eq!(listed.json()["keyringStatus"], "AVAILABLE");
    assert_provider(&listed.json()["items"][0]);

    let missing_csrf = fixture
        .request(
            Method::POST,
            PROVIDERS_PATH,
            Some(create_body("Missing CSRF", "csrf-secret-sentinel")),
            true,
            false,
        )
        .await;
    assert_observed(&document, PROVIDERS_PATH, "post", &missing_csrf, 403);
    assert_error_envelope(&missing_csrf.json());

    let invalid = fixture
        .request(
            Method::POST,
            PROVIDERS_PATH,
            Some(json!({ "displayName": "Incomplete" })),
            true,
            true,
        )
        .await;
    assert_observed(&document, PROVIDERS_PATH, "post", &invalid, 422);
    assert_error_envelope(&invalid.json());

    let created = fixture
        .request(
            Method::POST,
            PROVIDERS_PATH,
            Some(create_body("Created", "created-secret-sentinel")),
            true,
            true,
        )
        .await;
    assert_observed(&document, PROVIDERS_PATH, "post", &created, 201);
    assert_provider(&created.json());
    let provider_id = created.json()["providerId"]
        .as_str()
        .expect("created provider ID should exist")
        .to_owned();
    assert_eq!(
        created
            .headers
            .get(LOCATION)
            .and_then(|value| value.to_str().ok()),
        Some(format!("{PROVIDERS_PATH}/{provider_id}").as_str())
    );
    assert_secret_absent(&created);

    let not_found = fixture
        .request(
            Method::GET,
            "/api/v1/ai/providers/00000000-0000-4000-8000-000000009999",
            None,
            true,
            false,
        )
        .await;
    assert_observed(&document, PROVIDER_PATH, "get", &not_found, 404);

    let conflict = fixture
        .request(
            Method::PATCH,
            &format!("{PROVIDERS_PATH}/{provider_id}"),
            Some(json!({ "expectedRevision": 99, "isEnabled": false })),
            true,
            true,
        )
        .await;
    assert_observed(&document, PROVIDER_PATH, "patch", &conflict, 409);

    fixture.corrupt_provider_kind().await;
    let internal = fixture
        .request(
            Method::GET,
            &format!("{PROVIDERS_PATH}/{}", fixture.provider_id),
            None,
            true,
            false,
        )
        .await;
    assert_observed(&document, PROVIDER_PATH, "get", &internal, 500);
    assert_secret_absent(&internal);

    let unavailable_fixture = ContractFixture::new(false).await;
    let unavailable = unavailable_fixture
        .request(
            Method::POST,
            PROVIDERS_PATH,
            Some(create_body("Unavailable", "unavailable-secret-sentinel")),
            true,
            true,
        )
        .await;
    assert_observed(&document, PROVIDERS_PATH, "post", &unavailable, 503);
    assert_secret_absent(&unavailable);

    let rate_fixture = ContractFixture::new(true).await;
    for _ in 0..30 {
        let admitted = rate_fixture
            .request(
                Method::PATCH,
                &format!("{PROVIDERS_PATH}/{}", rate_fixture.provider_id),
                Some(json!({ "expectedRevision": 99, "isEnabled": false })),
                true,
                true,
            )
            .await;
        assert_eq!(admitted.status, StatusCode::CONFLICT);
    }
    let limited = rate_fixture
        .request(
            Method::PATCH,
            &format!("{PROVIDERS_PATH}/{}", rate_fixture.provider_id),
            Some(json!({ "expectedRevision": 99, "isEnabled": false })),
            true,
            true,
        )
        .await;
    assert_observed(&document, PROVIDER_PATH, "patch", &limited, 429);
    assert!(limited.headers.contains_key(RETRY_AFTER));
}

struct ContractFixture {
    _data: TempDir,
    database: DatabaseConnection,
    app: Router,
    cookie: String,
    csrf: String,
    provider_id: String,
}

impl ContractFixture {
    async fn new(app_has_keyring: bool) -> Self {
        let data = tempfile::tempdir().expect("temporary provider OpenAPI directory");
        let url = format!(
            "sqlite://{}?mode=rwc",
            data.path().join("ai-provider-openapi.db").display()
        );
        let database = connect(&DatabaseConfig::new(SecretString::from(url)))
            .await
            .expect("provider OpenAPI database should connect");
        migrate(&database)
            .await
            .expect("provider OpenAPI database should migrate");
        insert_user(&database, USER_A_ID, "provider-openapi").await;

        let keyring = Arc::new(provider_keyring());
        let repository = ProviderRepository::new(database.clone(), Some(Arc::clone(&keyring)));
        let provider = repository
            .create(provider_input())
            .await
            .expect("provider OpenAPI row should seed");
        let setup = SetupService::ready(data.path(), None, database.clone());
        let session = setup
            .sessions()
            .create(USER_A_ID)
            .await
            .expect("provider OpenAPI session should create");
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
            provider_id: provider.id().to_owned(),
        }
    }

    async fn request(
        &self,
        method: Method,
        uri: &str,
        body: Option<Value>,
        authenticated: bool,
        valid_csrf: bool,
    ) -> CapturedResponse {
        let mut request = Request::builder().method(method).uri(uri);
        if authenticated {
            request = request.header(COOKIE, &self.cookie);
        }
        if valid_csrf {
            request = request
                .header("x-csrf-token", &self.csrf)
                .header(ORIGIN, "http://provider-openapi.test")
                .header(HOST, "provider-openapi.test");
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
                    .expect("provider OpenAPI request should build"),
            )
            .await
            .expect("provider OpenAPI request should complete");
        CapturedResponse::from_response(response).await
    }

    async fn corrupt_provider_kind(&self) {
        self.database
            .execute(Statement::from_sql_and_values(
                DbBackend::Sqlite,
                "UPDATE ai_providers SET kind = ? WHERE id = ?",
                ["CORRUPT_KIND".into(), self.provider_id.clone().into()],
            ))
            .await
            .expect("provider kind should be corrupted");
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
                .expect("provider OpenAPI body should collect")
                .to_bytes()
                .to_vec(),
        }
    }

    fn json(&self) -> Value {
        serde_json::from_slice(&self.body).expect("provider OpenAPI response should be JSON")
    }
}

fn create_body(display_name: &str, credential: &str) -> Value {
    json!({
        "displayName": display_name,
        "kind": "OPENAI_RESPONSES",
        "endpoint": null,
        "model": "gpt-test-model",
        "credential": credential,
        "capabilities": {
            "supportsUsage": true,
            "supportsIdempotency": true
        },
        "policy": {
            "maxConcurrency": 2,
            "requestsPerMinute": 30,
            "maxInputTokensPerRequest": 128000,
            "maxOutputTokensPerRequest": 4096,
            "inputCostMicrosPerMillionTokens": null,
            "outputCostMicrosPerMillionTokens": null,
            "maxCostMicrosPerRequest": 250000
        },
        "isEnabled": true
    })
}

fn provider_input() -> CreateProvider {
    CreateProvider {
        scope: ProviderScope::user(USER_A_ID).unwrap(),
        display_name: "Provider OpenAPI".to_owned(),
        kind: ProviderKind::OpenAiResponses,
        endpoint: None,
        model: "gpt-test-model".to_owned(),
        credential: SecretString::from("provider-openapi-secret-sentinel"),
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

fn provider_keyring() -> ProviderSecretKeyring {
    let key = SecretString::from(format!("primary:{}", URL_SAFE_NO_PAD.encode([0x41_u8; 32])));
    ProviderSecretKeyring::from_entries(&[key]).expect("provider OpenAPI keyring should construct")
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
            .is_some(),
        "observed {method} provider status {status} must be documented"
    );
    assert_eq!(response.headers.get(CACHE_CONTROL).unwrap(), "no-store");
    assert_eq!(response.headers.get(PRAGMA).unwrap(), "no-cache");
    assert_eq!(
        response
            .headers
            .get(CONTENT_TYPE)
            .and_then(|value| value.to_str().ok()),
        Some("application/json")
    );
}

fn assert_provider(value: &Value) {
    let object = value
        .as_object()
        .expect("provider response should be an object");
    assert_eq!(
        object.keys().cloned().collect::<BTreeSet<_>>(),
        BTreeSet::from([
            "canEdit".to_owned(),
            "capabilities".to_owned(),
            "createdAt".to_owned(),
            "displayName".to_owned(),
            "endpoint".to_owned(),
            "isEnabled".to_owned(),
            "kind".to_owned(),
            "model".to_owned(),
            "policy".to_owned(),
            "providerId".to_owned(),
            "revision".to_owned(),
            "scope".to_owned(),
            "updatedAt".to_owned(),
        ])
    );
    assert!(value["providerId"].is_string());
    assert!(matches!(value["scope"].as_str(), Some("INSTANCE" | "USER")));
    assert!(
        value["endpoint"]
            .as_str()
            .is_some_and(|value| value.starts_with("https://"))
    );
}

fn assert_error_envelope(value: &Value) {
    assert!(value["error"]["code"].is_string());
    assert!(value["error"]["message"].is_string());
    assert!(value["error"]["requestId"].is_string());
}

fn assert_secret_absent(response: &CapturedResponse) {
    let body = String::from_utf8_lossy(&response.body);
    for sentinel in [
        "encryptedSecret",
        "primary:",
        "created-secret-sentinel",
        "unavailable-secret-sentinel",
        "provider-openapi-secret-sentinel",
    ] {
        assert!(!body.contains(sentinel), "response leaked {sentinel}");
    }
    assert_no_secret_fields(&response.json());
}

fn assert_no_secret_fields(value: &Value) {
    match value {
        Value::Object(object) => {
            for (key, nested) in object {
                assert!(
                    !matches!(
                        key.as_str(),
                        "credential" | "encryptedSecret" | "keyId" | "ownerUserId"
                    ),
                    "response leaked secret field {key}"
                );
                assert_no_secret_fields(nested);
            }
        }
        Value::Array(values) => {
            for nested in values {
                assert_no_secret_fields(nested);
            }
        }
        _ => {}
    }
}

fn load_openapi() -> Value {
    let artifact = fs::read_to_string(OPENAPI_PATH)
        .unwrap_or_else(|error| panic!("provider OpenAPI should exist: {error}"));
    serde_json::from_str(&artifact).expect("provider OpenAPI should be valid JSON")
}

fn documented_operations(document: &Value) -> BTreeSet<(String, String)> {
    let mut operations = BTreeSet::new();
    for (path, item) in document["paths"]
        .as_object()
        .expect("OpenAPI paths should be an object")
    {
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
        .expect("operation responses should be an object")
        .keys()
        .map(|status| status.parse::<u16>().expect("status should be numeric"))
        .collect::<BTreeSet<_>>();
    assert_eq!(actual, expected.iter().copied().collect());
}

fn assert_required(document: &Value, schema_name: &str, expected: &[&str]) {
    let actual = document["components"]["schemas"][schema_name]["required"]
        .as_array()
        .expect("required should be an array")
        .iter()
        .map(|value| value.as_str().expect("required field should be a string"))
        .collect::<BTreeSet<_>>();
    assert_eq!(actual, expected.iter().copied().collect());
}

fn assert_exact_properties(schema: &Value, expected: &[&str]) {
    let actual = schema["properties"]
        .as_object()
        .expect("properties should be an object")
        .keys()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    assert_eq!(actual, expected.iter().copied().collect());
}

fn assert_all_local_refs_resolve(root: &Value, value: &Value) {
    match value {
        Value::Object(object) => {
            if let Some(reference) = object.get("$ref").and_then(Value::as_str) {
                let pointer = reference
                    .strip_prefix('#')
                    .expect("OpenAPI references should be local");
                assert!(
                    root.pointer(pointer).is_some(),
                    "unresolved reference {reference}"
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
