#[allow(dead_code)]
mod support;

use std::sync::Arc;

use axum::{
    Router,
    body::Body,
    http::{
        Method, Request, StatusCode,
        header::{CACHE_CONTROL, CONTENT_TYPE, COOKIE, HOST, LOCATION, ORIGIN, PRAGMA},
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
use support::database::{USER_A_ID, USER_B_ID, insert_user};
use tempfile::TempDir;
use tower::ServiceExt;

struct ProviderFixture {
    _data: TempDir,
    database: DatabaseConnection,
    app: Router,
    user_a_cookie: String,
    user_a_csrf: String,
    user_b_cookie: String,
    user_b_csrf: String,
    instance_provider_id: String,
    user_a_provider_id: String,
    user_b_provider_id: String,
}

#[derive(Clone, Copy)]
enum UserKind {
    A,
    B,
}

impl ProviderFixture {
    async fn new(app_has_keyring: bool) -> Self {
        Self::new_with_user_a_provider_count(app_has_keyring, 1).await
    }

    async fn new_with_user_a_provider_count(
        app_has_keyring: bool,
        user_a_provider_count: usize,
    ) -> Self {
        assert!(user_a_provider_count >= 1);
        let data = tempfile::tempdir().expect("temporary provider API directory");
        let url = format!(
            "sqlite://{}?mode=rwc",
            data.path().join("provider-api.db").display()
        );
        let database = connect(&DatabaseConfig::new(SecretString::from(url)))
            .await
            .expect("provider API database should connect");
        migrate(&database)
            .await
            .expect("provider API database should migrate");
        insert_user(&database, USER_A_ID, "provider-api-a").await;
        insert_user(&database, USER_B_ID, "provider-api-b").await;

        let keyring = Arc::new(provider_keyring());
        let repository = ProviderRepository::new(database.clone(), Some(Arc::clone(&keyring)));
        let instance = repository
            .create(provider_input(
                ProviderScope::Instance,
                "Shared instance",
                "instance-credential-sentinel",
            ))
            .await
            .expect("instance provider should seed");
        let user_a = repository
            .create(provider_input(
                ProviderScope::user(USER_A_ID).unwrap(),
                "User A provider",
                "user-a-credential-sentinel",
            ))
            .await
            .expect("user A provider should seed");
        for index in 1..user_a_provider_count {
            repository
                .create(provider_input(
                    ProviderScope::user(USER_A_ID).unwrap(),
                    &format!("User A provider {index}"),
                    "user-a-extra-credential-sentinel",
                ))
                .await
                .expect("additional user A provider should seed");
        }
        let user_b = repository
            .create(provider_input(
                ProviderScope::user(USER_B_ID).unwrap(),
                "User B private",
                "user-b-credential-sentinel",
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
        let user_a_cookie = session_cookie(&session_a);
        let user_b_cookie = session_cookie(&session_b);
        let user_a_csrf = session_a.csrf_token.expose_secret().to_owned();
        let user_b_csrf = session_b.csrf_token.expose_secret().to_owned();
        let state = AppState::new(setup).with_provider_keyring(app_has_keyring.then_some(keyring));
        Self {
            _data: data,
            database,
            app: build_router(state),
            user_a_cookie,
            user_a_csrf,
            user_b_cookie,
            user_b_csrf,
            instance_provider_id: instance.id().to_owned(),
            user_a_provider_id: user_a.id().to_owned(),
            user_b_provider_id: user_b.id().to_owned(),
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
                UserKind::A => (&self.user_a_cookie, Some(&self.user_a_csrf)),
                UserKind::B => (&self.user_b_cookie, Some(&self.user_b_csrf)),
            };
            request = request.header(COOKIE, cookie);
            if include_csrf {
                request = request
                    .header("x-csrf-token", csrf.expect("user A CSRF should exist"))
                    .header(ORIGIN, "http://providers.test")
                    .header(HOST, "providers.test");
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
                    .expect("provider request should build"),
            )
            .await
            .expect("provider request should complete");
        CapturedResponse::from_response(response).await
    }

    async fn corrupt_provider_kind(&self, provider_id: &str) {
        self.database
            .execute(Statement::from_sql_and_values(
                DbBackend::Sqlite,
                "UPDATE ai_providers SET kind = ? WHERE id = ?",
                ["CORRUPT_KIND".into(), provider_id.to_owned().into()],
            ))
            .await
            .expect("provider kind should be corrupted for the internal-error fixture");
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
                .expect("provider response should collect")
                .to_bytes()
                .to_vec(),
        }
    }

    fn json(&self) -> Value {
        serde_json::from_slice(&self.body).expect("provider response should be JSON")
    }
}

#[tokio::test]
async fn provider_tracer_is_user_scoped_and_never_returns_credentials() {
    let fixture = ProviderFixture::new(true).await;
    let listed = fixture
        .request(
            Method::GET,
            "/api/v1/ai/providers",
            None,
            Some(UserKind::A),
            false,
        )
        .await;
    assert_eq!(listed.status, StatusCode::OK);
    assert_eq!(listed.json()["keyringStatus"], "AVAILABLE");
    let items = listed.json()["items"]
        .as_array()
        .expect("provider items should be an array")
        .clone();
    assert_eq!(items.len(), 2);
    assert!(items.iter().any(|item| {
        item["providerId"] == fixture.instance_provider_id
            && item["scope"] == "INSTANCE"
            && item["canEdit"] == false
    }));
    assert!(items.iter().any(|item| {
        item["providerId"] == fixture.user_a_provider_id
            && item["scope"] == "USER"
            && item["canEdit"] == true
    }));
    assert_sensitive_cache_headers(&listed);
    assert_secret_absent(&listed);

    let created = fixture
        .request(
            Method::POST,
            "/api/v1/ai/providers",
            Some(create_body(
                "Created provider",
                "created-credential-sentinel",
            )),
            Some(UserKind::A),
            true,
        )
        .await;
    assert_eq!(created.status, StatusCode::CREATED);
    assert_sensitive_cache_headers(&created);
    assert_eq!(created.json()["scope"], "USER");
    assert_eq!(created.json()["kind"], "OPENAI_RESPONSES");
    assert_eq!(created.json()["endpoint"], "https://api.openai.com/");
    assert_eq!(created.json()["revision"], 0);
    let created_json = created.json();
    let provider_id = created_json["providerId"]
        .as_str()
        .expect("created provider ID should be present")
        .to_owned();
    assert_eq!(
        created
            .headers
            .get(LOCATION)
            .and_then(|value| value.to_str().ok()),
        Some(format!("/api/v1/ai/providers/{provider_id}").as_str())
    );
    assert_secret_absent(&created);

    let detail = fixture
        .request(
            Method::GET,
            &format!("/api/v1/ai/providers/{provider_id}"),
            None,
            Some(UserKind::A),
            false,
        )
        .await;
    assert_eq!(detail.status, StatusCode::OK);
    assert_sensitive_cache_headers(&detail);
    assert_eq!(detail.json()["displayName"], "Created provider");
    assert_secret_absent(&detail);

    let updated = fixture
        .request(
            Method::PATCH,
            &format!("/api/v1/ai/providers/{provider_id}"),
            Some(json!({
                "expectedRevision": 0,
                "displayName": "Renamed provider",
                "credential": null
            })),
            Some(UserKind::A),
            true,
        )
        .await;
    assert_eq!(updated.status, StatusCode::OK);
    assert_sensitive_cache_headers(&updated);
    assert_eq!(updated.json()["displayName"], "Renamed provider");
    assert_eq!(updated.json()["revision"], 1);
    assert_secret_absent(&updated);

    let cross_user = fixture
        .request(
            Method::GET,
            &format!("/api/v1/ai/providers/{provider_id}"),
            None,
            Some(UserKind::B),
            false,
        )
        .await;
    assert_error(&cross_user, StatusCode::NOT_FOUND, "NOT_FOUND");

    let instance_patch = fixture
        .request(
            Method::PATCH,
            &format!("/api/v1/ai/providers/{}", fixture.instance_provider_id),
            Some(json!({ "expectedRevision": 0, "isEnabled": false })),
            Some(UserKind::A),
            true,
        )
        .await;
    assert_error(&instance_patch, StatusCode::NOT_FOUND, "NOT_FOUND");
}

#[tokio::test]
async fn provider_mutations_require_authentication_csrf_and_valid_input() {
    let fixture = ProviderFixture::new(true).await;
    let unauthenticated = fixture
        .request(Method::GET, "/api/v1/ai/providers", None, None, false)
        .await;
    assert_error(
        &unauthenticated,
        StatusCode::UNAUTHORIZED,
        "AUTHENTICATION_REQUIRED",
    );

    let missing_csrf = fixture
        .request(
            Method::POST,
            "/api/v1/ai/providers",
            Some(create_body("Provider", "credential")),
            Some(UserKind::A),
            false,
        )
        .await;
    assert_error(&missing_csrf, StatusCode::FORBIDDEN, "FORBIDDEN");

    let unknown_field = fixture
        .request(
            Method::POST,
            "/api/v1/ai/providers",
            Some({
                let mut body = create_body("Provider", "credential");
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

    let missing_nullable_endpoint = fixture
        .request(
            Method::POST,
            "/api/v1/ai/providers",
            Some({
                let mut body = create_body("Missing endpoint", "credential");
                body.as_object_mut()
                    .expect("create body should be an object")
                    .remove("endpoint");
                body
            }),
            Some(UserKind::A),
            true,
        )
        .await;
    assert_error(
        &missing_nullable_endpoint,
        StatusCode::UNPROCESSABLE_ENTITY,
        "VALIDATION_ERROR",
    );

    let empty_patch = fixture
        .request(
            Method::PATCH,
            &format!("/api/v1/ai/providers/{}", fixture.user_a_provider_id),
            Some(json!({ "expectedRevision": 0 })),
            Some(UserKind::A),
            true,
        )
        .await;
    assert_error(
        &empty_patch,
        StatusCode::UNPROCESSABLE_ENTITY,
        "VALIDATION_ERROR",
    );

    let malformed_id = fixture
        .request(
            Method::GET,
            "/api/v1/ai/providers/not-a-provider-id",
            None,
            Some(UserKind::A),
            false,
        )
        .await;
    assert_error(
        &malformed_id,
        StatusCode::UNPROCESSABLE_ENTITY,
        "VALIDATION_ERROR",
    );

    let revision_conflict = fixture
        .request(
            Method::PATCH,
            &format!("/api/v1/ai/providers/{}", fixture.user_a_provider_id),
            Some(json!({ "expectedRevision": 99, "isEnabled": false })),
            Some(UserKind::A),
            true,
        )
        .await;
    assert_error(
        &revision_conflict,
        StatusCode::CONFLICT,
        "REVISION_CONFLICT",
    );

    let invalid_model = fixture
        .request(
            Method::PATCH,
            &format!("/api/v1/ai/providers/{}", fixture.user_a_provider_id),
            Some(json!({ "expectedRevision": 0, "model": "" })),
            Some(UserKind::A),
            true,
        )
        .await;
    assert_error(
        &invalid_model,
        StatusCode::UNPROCESSABLE_ENTITY,
        "VALIDATION_ERROR",
    );

    let null_non_nullable_patch = fixture
        .request(
            Method::PATCH,
            &format!("/api/v1/ai/providers/{}", fixture.user_a_provider_id),
            Some(json!({
                "expectedRevision": 0,
                "displayName": "Still present",
                "isEnabled": null
            })),
            Some(UserKind::A),
            true,
        )
        .await;
    assert_error(
        &null_non_nullable_patch,
        StatusCode::UNPROCESSABLE_ENTITY,
        "VALIDATION_ERROR",
    );
}

#[tokio::test]
async fn missing_keyring_keeps_metadata_recovery_but_blocks_secret_changes() {
    let fixture = ProviderFixture::new(false).await;
    let listed = fixture
        .request(
            Method::GET,
            "/api/v1/ai/providers",
            None,
            Some(UserKind::A),
            false,
        )
        .await;
    assert_eq!(listed.status, StatusCode::OK);
    assert_eq!(listed.json()["keyringStatus"], "UNAVAILABLE");
    assert_eq!(listed.json()["items"].as_array().unwrap().len(), 2);

    let metadata_update = fixture
        .request(
            Method::PATCH,
            &format!("/api/v1/ai/providers/{}", fixture.user_a_provider_id),
            Some(json!({ "expectedRevision": 0, "isEnabled": false })),
            Some(UserKind::A),
            true,
        )
        .await;
    assert_eq!(metadata_update.status, StatusCode::OK);
    assert_eq!(metadata_update.json()["isEnabled"], false);

    let create = fixture
        .request(
            Method::POST,
            "/api/v1/ai/providers",
            Some(create_body("Unavailable", "credential")),
            Some(UserKind::A),
            true,
        )
        .await;
    assert_error(
        &create,
        StatusCode::SERVICE_UNAVAILABLE,
        "AI_PROVIDER_KEYRING_UNAVAILABLE",
    );

    let rotate = fixture
        .request(
            Method::PATCH,
            &format!("/api/v1/ai/providers/{}", fixture.user_a_provider_id),
            Some(json!({
                "expectedRevision": 1,
                "credential": "replacement-credential"
            })),
            Some(UserKind::A),
            true,
        )
        .await;
    assert_error(
        &rotate,
        StatusCode::SERVICE_UNAVAILABLE,
        "AI_PROVIDER_KEYRING_UNAVAILABLE",
    );
}

#[tokio::test]
async fn provider_paths_methods_limits_and_internal_errors_are_stable() {
    let fixture = ProviderFixture::new(true).await;
    let trailing_slash = fixture
        .request(
            Method::GET,
            "/api/v1/ai/providers/",
            None,
            Some(UserKind::A),
            false,
        )
        .await;
    assert_error(&trailing_slash, StatusCode::NOT_FOUND, "NOT_FOUND");

    let unknown_child = fixture
        .request(
            Method::GET,
            &format!(
                "/api/v1/ai/providers/{}/unknown",
                fixture.user_a_provider_id
            ),
            None,
            Some(UserKind::A),
            false,
        )
        .await;
    assert_error(&unknown_child, StatusCode::NOT_FOUND, "NOT_FOUND");

    let method_not_allowed = fixture
        .request(
            Method::DELETE,
            &format!("/api/v1/ai/providers/{}", fixture.user_a_provider_id),
            None,
            Some(UserKind::A),
            false,
        )
        .await;
    assert_error(
        &method_not_allowed,
        StatusCode::METHOD_NOT_ALLOWED,
        "METHOD_NOT_ALLOWED",
    );

    fixture
        .corrupt_provider_kind(&fixture.user_a_provider_id)
        .await;
    let internal = fixture
        .request(
            Method::GET,
            &format!("/api/v1/ai/providers/{}", fixture.user_a_provider_id),
            None,
            Some(UserKind::A),
            false,
        )
        .await;
    assert_error(
        &internal,
        StatusCode::INTERNAL_SERVER_ERROR,
        "INTERNAL_ERROR",
    );
    assert_secret_absent(&internal);

    let limit_fixture = ProviderFixture::new_with_user_a_provider_count(true, 32).await;
    let limit = limit_fixture
        .request(
            Method::POST,
            "/api/v1/ai/providers",
            Some(create_body("Over limit", "limit-credential-sentinel")),
            Some(UserKind::A),
            true,
        )
        .await;
    assert_error(&limit, StatusCode::CONFLICT, "PROVIDER_LIMIT_REACHED");
    assert_secret_absent(&limit);
}

#[tokio::test]
async fn provider_mutation_rate_limit_is_isolated_per_user() {
    let fixture = ProviderFixture::new(true).await;
    for _ in 0..30 {
        let response = fixture
            .request(
                Method::PATCH,
                &format!("/api/v1/ai/providers/{}", fixture.user_a_provider_id),
                Some(json!({ "expectedRevision": 99, "isEnabled": false })),
                Some(UserKind::A),
                true,
            )
            .await;
        assert_error(&response, StatusCode::CONFLICT, "REVISION_CONFLICT");
    }

    let limited = fixture
        .request(
            Method::PATCH,
            &format!("/api/v1/ai/providers/{}", fixture.user_a_provider_id),
            Some(json!({ "expectedRevision": 99, "isEnabled": false })),
            Some(UserKind::A),
            true,
        )
        .await;
    assert_error(&limited, StatusCode::TOO_MANY_REQUESTS, "RATE_LIMITED");
    assert!(limited.headers.contains_key("retry-after"));

    let user_b = fixture
        .request(
            Method::PATCH,
            &format!("/api/v1/ai/providers/{}", fixture.user_b_provider_id),
            Some(json!({ "expectedRevision": 0, "isEnabled": false })),
            Some(UserKind::B),
            true,
        )
        .await;
    assert_eq!(user_b.status, StatusCode::OK);
    assert_sensitive_cache_headers(&user_b);
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

fn provider_input(scope: ProviderScope, display_name: &str, credential: &str) -> CreateProvider {
    CreateProvider {
        scope,
        display_name: display_name.to_owned(),
        kind: ProviderKind::OpenAiResponses,
        endpoint: None,
        model: "gpt-test-model".to_owned(),
        credential: SecretString::from(credential.to_owned()),
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
    ProviderSecretKeyring::from_entries(&[key]).expect("provider fixture keyring should construct")
}

fn session_cookie(session: &raindrop::auth::CreatedSession) -> String {
    build_session_cookie(session, false)
        .to_string()
        .split(';')
        .next()
        .expect("session cookie should have a name/value pair")
        .to_owned()
}

fn assert_secret_absent(response: &CapturedResponse) {
    let body = String::from_utf8_lossy(&response.body);
    for sentinel in [
        "credential",
        "encryptedSecret",
        "primary:",
        "created-credential-sentinel",
        "user-a-credential-sentinel",
        "instance-credential-sentinel",
    ] {
        assert!(
            !body.contains(sentinel),
            "provider response must not contain {sentinel}"
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
