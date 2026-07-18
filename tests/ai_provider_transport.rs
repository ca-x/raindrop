#[allow(dead_code)]
mod support;

use std::{
    fs,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

use async_trait::async_trait;
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use http::StatusCode;
use raindrop::{
    content::provider::{
        CreateProvider, EncodedProviderRequest, FinishReason, ProviderBinding,
        ProviderCallErrorKind, ProviderCapabilities, ProviderClient, ProviderCoreErrorKind,
        ProviderEndpoint, ProviderKind, ProviderMetadata, ProviderPolicy, ProviderRepository,
        ProviderRetryAfter, ProviderScope, ProviderSecretKeyring, ProviderTimeoutStage,
        ProviderTransport, ProviderTransportError, ProviderTransportErrorKind,
        ProviderTransportResponse, StructuredGenerationRequest, TokenUsage,
    },
    db::{entities::ai_provider, migrate},
};
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, DatabaseConnection, EntityTrait, IntoActiveModel,
};
use secrecy::{ExposeSecret, SecretString};
use serde_json::{Value, json};
use support::database::{USER_A_ID, connect_for_contract, insert_user};
use tempfile::{TempDir, tempdir};
use time::{Duration, OffsetDateTime};

struct ExpectedRequest {
    provider_id: String,
    endpoint: String,
    path: &'static str,
    body: Value,
    secret_header: &'static str,
    secret_value: &'static str,
    public_headers: &'static [(&'static str, &'static str)],
}

struct RecordingTransport {
    expected: ExpectedRequest,
    calls: Arc<AtomicUsize>,
    response: &'static [u8],
}

struct StaticTransport {
    status: StatusCode,
    body: Vec<u8>,
    retry_after: Option<ProviderRetryAfter>,
    calls: Arc<AtomicUsize>,
}

#[async_trait]
impl ProviderTransport for StaticTransport {
    async fn execute(
        &self,
        _provider_id: &str,
        _endpoint: &ProviderEndpoint,
        _request: EncodedProviderRequest,
    ) -> Result<ProviderTransportResponse, ProviderTransportError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(ProviderTransportResponse::new(
            self.status,
            self.body.clone(),
            self.retry_after,
        ))
    }
}

struct ErrorTransport {
    kind: ProviderTransportErrorKind,
    calls: Arc<AtomicUsize>,
}

#[async_trait]
impl ProviderTransport for ErrorTransport {
    async fn execute(
        &self,
        provider_id: &str,
        _endpoint: &ProviderEndpoint,
        _request: EncodedProviderRequest,
    ) -> Result<ProviderTransportResponse, ProviderTransportError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if self.kind == ProviderTransportErrorKind::Timeout {
            Err(ProviderTransportError::timeout(
                provider_id,
                ProviderTimeoutStage::FirstByte,
            ))
        } else {
            Err(ProviderTransportError::new(provider_id, self.kind))
        }
    }
}

#[async_trait]
impl ProviderTransport for RecordingTransport {
    async fn execute(
        &self,
        provider_id: &str,
        endpoint: &ProviderEndpoint,
        request: EncodedProviderRequest,
    ) -> Result<ProviderTransportResponse, ProviderTransportError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        assert!(
            provider_id == self.expected.provider_id,
            "provider ID mismatch"
        );
        assert!(
            endpoint.as_str() == self.expected.endpoint,
            "endpoint mismatch"
        );
        assert!(
            request.path() == self.expected.path,
            "adapter path mismatch"
        );
        assert!(
            serde_json::from_slice::<Value>(request.body())
                .ok()
                .as_ref()
                == Some(&self.expected.body),
            "adapter body mismatch"
        );
        let secret = request
            .headers()
            .iter()
            .find(|header| header.name().as_str() == self.expected.secret_header)
            .and_then(|header| header.secret_value())
            .map(ExposeSecret::expose_secret);
        assert!(
            secret == Some(self.expected.secret_value),
            "secret header mismatch"
        );
        for (name, expected) in self.expected.public_headers {
            let actual = request
                .headers()
                .iter()
                .find(|header| header.name().as_str() == *name)
                .and_then(|header| header.public_value())
                .and_then(|value| value.to_str().ok());
            assert!(actual == Some(*expected), "public header mismatch");
        }
        assert_eq!(
            request.headers().len(),
            self.expected.public_headers.len() + 1
        );
        Ok(ProviderTransportResponse::new(
            StatusCode::OK,
            self.response.to_vec(),
            None,
        ))
    }
}

#[tokio::test]
async fn provider_client_executes_all_protocol_bindings_once() {
    let cases = [
        (
            ProviderKind::AnthropicMessages,
            None,
            "https://api.anthropic.com/",
            "/v1/messages",
            include_str!("fixtures/ai-provider/v1/anthropic-request.json"),
            "x-api-key",
            "provider-key",
            &[
                ("content-type", "application/json"),
                ("accept", "application/json"),
                ("anthropic-version", "2023-06-01"),
                ("idempotency-key", "job-request-1"),
            ][..],
            include_bytes!("fixtures/ai-provider/v1/anthropic-response.json").as_slice(),
            "claude-fixture",
            12,
            5,
        ),
        (
            ProviderKind::OpenAiResponses,
            Some("https://gateway.example/tenant/"),
            "https://gateway.example/tenant/",
            "/v1/responses",
            include_str!("fixtures/ai-provider/v1/openai-responses-request.json"),
            "authorization",
            "Bearer provider-key",
            &[
                ("content-type", "application/json"),
                ("accept", "application/json"),
                ("idempotency-key", "job-request-1"),
            ][..],
            include_bytes!("fixtures/ai-provider/v1/openai-responses-response.json").as_slice(),
            "gpt-responses-fixture",
            14,
            6,
        ),
        (
            ProviderKind::OpenAiChatCompletions,
            None,
            "https://api.openai.com/",
            "/v1/chat/completions",
            include_str!("fixtures/ai-provider/v1/openai-chat-request.json"),
            "authorization",
            "Bearer provider-key",
            &[
                ("content-type", "application/json"),
                ("accept", "application/json"),
                ("idempotency-key", "job-request-1"),
            ][..],
            include_bytes!("fixtures/ai-provider/v1/openai-chat-response.json").as_slice(),
            "gpt-chat-fixture",
            16,
            7,
        ),
        (
            ProviderKind::GoogleGemini,
            None,
            "https://generativelanguage.googleapis.com/",
            "/v1beta/models/model-v1:generateContent",
            include_str!("fixtures/ai-provider/v1/gemini-request.json"),
            "x-goog-api-key",
            "provider-key",
            &[
                ("content-type", "application/json"),
                ("accept", "application/json"),
                ("x-goog-request-id", "job-request-1"),
            ][..],
            include_bytes!("fixtures/ai-provider/v1/gemini-response.json").as_slice(),
            "gemini-fixture",
            18,
            8,
        ),
    ];

    for (
        kind,
        endpoint,
        expected_endpoint,
        path,
        request_fixture,
        secret_header,
        secret_value,
        public_headers,
        response_fixture,
        model_label,
        input_tokens,
        output_tokens,
    ) in cases
    {
        let binding = create_binding(kind, endpoint, "provider-key").await;
        let calls = Arc::new(AtomicUsize::new(0));
        let transport = RecordingTransport {
            expected: ExpectedRequest {
                provider_id: binding.metadata().id().to_owned(),
                endpoint: expected_endpoint.to_owned(),
                path,
                body: serde_json::from_str(request_fixture).expect("request fixture should parse"),
                secret_header,
                secret_value,
                public_headers,
            },
            calls: calls.clone(),
            response: response_fixture,
        };
        let response = ProviderClient::new(transport)
            .generate(&binding, &valid_request())
            .await
            .expect("provider call should succeed");

        assert_eq!(calls.load(Ordering::SeqCst), 1, "{kind:?}");
        assert_eq!(
            response.output,
            json!({ "summary": "fixture summary" }),
            "{kind:?}"
        );
        assert_eq!(response.finish_reason, FinishReason::Stop, "{kind:?}");
        assert_eq!(response.model_label, model_label, "{kind:?}");
        assert_eq!(
            response.usage,
            TokenUsage {
                input_tokens: Some(input_tokens),
                output_tokens: Some(output_tokens),
            },
            "{kind:?}"
        );
    }
}

#[tokio::test]
async fn provider_client_rejects_model_and_policy_mismatch_before_transport() {
    let binding = create_binding(ProviderKind::OpenAiResponses, None, "provider-key").await;
    for (request, expected) in [
        (
            {
                let mut request = valid_request();
                request.model = "other-model".to_owned();
                request
            },
            ProviderCallErrorKind::InvalidRequest,
        ),
        (
            {
                let mut request = valid_request();
                request.max_output_tokens = 4_097;
                request
            },
            ProviderCallErrorKind::RequestTooLarge,
        ),
    ] {
        let calls = Arc::new(AtomicUsize::new(0));
        let error = ProviderClient::new(StaticTransport {
            status: StatusCode::OK,
            body: include_bytes!("fixtures/ai-provider/v1/openai-responses-response.json").to_vec(),
            retry_after: None,
            calls: calls.clone(),
        })
        .generate(&binding, &request)
        .await
        .expect_err("binding/request mismatch should fail");
        assert_eq!(error.kind(), expected);
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }
}

#[tokio::test]
async fn provider_client_normalizes_transport_failures_without_leaking_context() {
    let binding = create_binding(
        ProviderKind::OpenAiResponses,
        Some("https://gateway.example/endpoint-sentinel/"),
        "credential-sentinel",
    )
    .await;
    for (transport_kind, expected) in [
        (
            ProviderTransportErrorKind::InvalidHeaders,
            ProviderCallErrorKind::Transport,
        ),
        (
            ProviderTransportErrorKind::Network,
            ProviderCallErrorKind::Transport,
        ),
        (
            ProviderTransportErrorKind::RedirectDenied,
            ProviderCallErrorKind::Transport,
        ),
        (
            ProviderTransportErrorKind::PeerMismatch,
            ProviderCallErrorKind::Transport,
        ),
        (
            ProviderTransportErrorKind::Decode,
            ProviderCallErrorKind::Transport,
        ),
        (
            ProviderTransportErrorKind::Timeout,
            ProviderCallErrorKind::Timeout,
        ),
        (
            ProviderTransportErrorKind::ResponseTooLarge,
            ProviderCallErrorKind::ResponseTooLarge,
        ),
    ] {
        let calls = Arc::new(AtomicUsize::new(0));
        let error = ProviderClient::new(ErrorTransport {
            kind: transport_kind,
            calls: calls.clone(),
        })
        .generate(&binding, &sensitive_request())
        .await
        .expect_err("transport failure should be normalized");
        assert_eq!(error.kind(), expected, "{transport_kind:?}");
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        let rendered = format!("{error:?} {error}");
        assert!(rendered.contains(binding.metadata().id()));
        for sentinel in [
            "endpoint-sentinel",
            "model-v1",
            "credential-sentinel",
            "prompt-sentinel",
            "input-sentinel",
            "schema-sentinel",
            "output-sentinel",
        ] {
            assert!(!rendered.contains(sentinel), "error leaked {sentinel}");
        }
    }
}

#[tokio::test]
async fn provider_client_maps_adapter_failures_and_preserves_retry_deadline() {
    let binding = create_binding(ProviderKind::OpenAiResponses, None, "provider-key").await;
    let retry_at = OffsetDateTime::UNIX_EPOCH + Duration::minutes(5);
    let mut invalid_output: Value = serde_json::from_str(include_str!(
        "fixtures/ai-provider/v1/openai-responses-response.json"
    ))
    .expect("response fixture should parse");
    invalid_output["output"][0]["content"][0]["text"] = json!("[1,2,3]");
    let cases = [
        (
            StatusCode::UNAUTHORIZED,
            Vec::new(),
            None,
            ProviderCallErrorKind::Authentication,
        ),
        (
            StatusCode::REQUEST_TIMEOUT,
            Vec::new(),
            None,
            ProviderCallErrorKind::Timeout,
        ),
        (
            StatusCode::TOO_MANY_REQUESTS,
            Vec::new(),
            Some(ProviderRetryAfter::from_deadline(retry_at)),
            ProviderCallErrorKind::RateLimited,
        ),
        (
            StatusCode::BAD_REQUEST,
            Vec::new(),
            None,
            ProviderCallErrorKind::Rejected,
        ),
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Vec::new(),
            None,
            ProviderCallErrorKind::Upstream,
        ),
        (
            StatusCode::OK,
            br#"{"#.to_vec(),
            None,
            ProviderCallErrorKind::MalformedResponse,
        ),
        (
            StatusCode::OK,
            serde_json::to_vec(&invalid_output).expect("invalid fixture should encode"),
            None,
            ProviderCallErrorKind::OutputSchemaInvalid,
        ),
        (
            StatusCode::OK,
            vec![b'x'; 2 * 1024 * 1024 + 1],
            None,
            ProviderCallErrorKind::ResponseTooLarge,
        ),
    ];

    for (status, body, retry_after, expected) in cases {
        let calls = Arc::new(AtomicUsize::new(0));
        let error = ProviderClient::new(StaticTransport {
            status,
            body,
            retry_after,
            calls: calls.clone(),
        })
        .generate(&binding, &valid_request())
        .await
        .expect_err("adapter failure should be normalized");
        assert_eq!(error.kind(), expected, "{status}");
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(
            error.retry_after_at(),
            if status == StatusCode::TOO_MANY_REQUESTS {
                Some(retry_at)
            } else {
                None
            }
        );
    }
}

#[tokio::test]
async fn disabled_and_corrupt_records_cannot_form_provider_bindings() {
    let (_data, database, repository) = repository_fixture().await;
    let disabled = create_provider(
        &repository,
        ProviderKind::OpenAiResponses,
        None,
        "provider-key",
        false,
    )
    .await;
    assert_eq!(
        repository
            .load_enabled_binding(disabled.id(), USER_A_ID)
            .await
            .expect_err("disabled provider should not bind")
            .kind(),
        ProviderCoreErrorKind::ProviderDisabled
    );

    let corrupt = create_provider(
        &repository,
        ProviderKind::OpenAiResponses,
        None,
        "provider-key",
        true,
    )
    .await;
    let stored = ai_provider::Entity::find_by_id(corrupt.id())
        .one(&database)
        .await
        .expect("provider row should query")
        .expect("provider row should exist");
    let mut stored = stored.into_active_model();
    stored.kind = Set("UNKNOWN_PROVIDER_KIND".to_owned());
    stored
        .update(&database)
        .await
        .expect("test corruption should persist");
    assert_eq!(
        repository
            .load_enabled_binding(corrupt.id(), USER_A_ID)
            .await
            .expect_err("corrupt provider should not bind")
            .kind(),
        ProviderCoreErrorKind::CorruptData
    );
}

#[test]
fn provider_wire_keys_remain_confined_to_adapter_modules() {
    let provider_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/content/provider");
    let allowed = [
        "anthropic.rs",
        "gemini.rs",
        "openai_chat.rs",
        "openai_responses.rs",
    ];
    let markers = [
        "x-api-key",
        "anthropic-version",
        "x-goog-api-key",
        "x-goog-request-id",
        "/v1/messages",
        "/v1/responses",
        "/v1/chat/completions",
        ":generateContent",
    ];
    for path in rust_sources(&provider_dir) {
        let source = fs::read_to_string(&path).expect("provider source should read");
        let production = source.split("#[cfg(test)]").next().unwrap_or(&source);
        let file_name = path
            .file_name()
            .and_then(|name| name.to_str())
            .expect("provider source should have a UTF-8 file name");
        if !allowed.contains(&file_name) {
            for marker in markers {
                assert!(
                    !production.contains(marker),
                    "provider wire marker {marker} escaped into {}",
                    path.display()
                );
            }
        }
        if path.starts_with(provider_dir.join("transport")) {
            for provider_name in ["Anthropic", "OpenAi", "Gemini", "ProviderKind::"] {
                assert!(
                    !production.contains(provider_name),
                    "transport branches on provider name {provider_name} in {}",
                    path.display()
                );
            }
        }
    }
}

async fn create_binding(
    kind: ProviderKind,
    endpoint: Option<&str>,
    credential: &str,
) -> ProviderBinding {
    let (_data, _database, repository) = repository_fixture().await;
    let metadata = create_provider(&repository, kind, endpoint, credential, true).await;
    repository
        .load_enabled_binding(metadata.id(), USER_A_ID)
        .await
        .expect("binding should load")
}

async fn repository_fixture() -> (TempDir, DatabaseConnection, ProviderRepository) {
    let data = tempdir().expect("temporary directory should create");
    let database_url = format!(
        "sqlite://{}?mode=rwc",
        data.path().join("provider-client.db").display()
    );
    let database = connect_for_contract(SecretString::from(database_url)).await;
    migrate(&database).await.expect("database should migrate");
    insert_user(&database, USER_A_ID, "provider-client-user").await;
    let repository = ProviderRepository::new(database.clone(), provider_keyring());
    (data, database, repository)
}

async fn create_provider(
    repository: &ProviderRepository,
    kind: ProviderKind,
    endpoint: Option<&str>,
    credential: &str,
    is_enabled: bool,
) -> ProviderMetadata {
    repository
        .create(CreateProvider {
            scope: ProviderScope::Instance,
            display_name: "Provider client test".to_owned(),
            kind,
            endpoint: endpoint.map(str::to_owned),
            model: "model-v1".to_owned(),
            credential: SecretString::from(credential.to_owned()),
            capabilities: ProviderCapabilities {
                supports_usage: true,
                supports_idempotency: true,
                supports_streaming: false,
            },
            policy: ProviderPolicy {
                max_concurrency: 2,
                requests_per_minute: Some(60),
                max_input_tokens_per_request: 128_000,
                max_output_tokens_per_request: 4_096,
                input_cost_micros_per_million_tokens: None,
                output_cost_micros_per_million_tokens: None,
                max_cost_micros_per_request: None,
            },
            is_enabled,
        })
        .await
        .expect("provider should create")
}

fn rust_sources(root: &Path) -> Vec<PathBuf> {
    let mut sources = Vec::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(directory) = pending.pop() {
        for entry in fs::read_dir(directory).expect("provider source directory should read") {
            let path = entry.expect("provider source entry should read").path();
            if path.is_dir() {
                pending.push(path);
            } else if path.extension().and_then(|value| value.to_str()) == Some("rs") {
                sources.push(path);
            }
        }
    }
    sources.sort();
    sources
}

fn provider_keyring() -> ProviderSecretKeyring {
    let entry = SecretString::from(format!("primary:{}", URL_SAFE_NO_PAD.encode([41_u8; 32])));
    ProviderSecretKeyring::from_entries(&[entry]).expect("test keyring should construct")
}

fn valid_request() -> StructuredGenerationRequest {
    StructuredGenerationRequest {
        model: "model-v1".to_owned(),
        system_instruction: "Return only the requested JSON object.".to_owned(),
        untrusted_input: json!({ "article": "untrusted article text" }),
        output_schema: raindrop::content::provider::OutputSchema {
            name: "ai_summary_v1".to_owned(),
            schema: json!({
                "type": "object",
                "additionalProperties": false,
                "required": ["summary"],
                "properties": { "summary": { "type": "string" } }
            }),
        },
        max_output_tokens: 4_096,
        idempotency_key: "job-request-1".to_owned(),
    }
}

fn sensitive_request() -> StructuredGenerationRequest {
    StructuredGenerationRequest {
        model: "model-v1".to_owned(),
        system_instruction: "prompt-sentinel".to_owned(),
        untrusted_input: json!({ "article": "input-sentinel" }),
        output_schema: raindrop::content::provider::OutputSchema {
            name: "schema_sentinel".to_owned(),
            schema: json!({
                "type": "object",
                "description": "schema-sentinel",
                "properties": {
                    "summary": { "type": "string", "const": "output-sentinel" }
                }
            }),
        },
        max_output_tokens: 4_096,
        idempotency_key: "request-sentinel".to_owned(),
    }
}
