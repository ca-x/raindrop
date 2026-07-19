#[allow(dead_code)]
mod support;

use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use async_trait::async_trait;
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use http::StatusCode;
use raindrop::{
    content::{
        ai::ProviderAiBroker,
        jobs::{
            ArtifactIdentity, ArtifactIdentityInput, ArtifactKind, ClaimContentJob, ClaimOutcome,
            ContentJobClaim, ContentJobOperation, ContentJobTrigger, ContentRepository,
            EnqueueContentJob, EnqueueContentJobInput, EnqueueResult, JobStatus,
        },
        provider::{
            CreateProvider, EncodedProviderRequest, ProviderCapabilities, ProviderClient,
            ProviderEndpoint, ProviderKind, ProviderMetadata, ProviderPolicy, ProviderRepository,
            ProviderRetryAfter, ProviderScope, ProviderSecretKeyring, ProviderTimeoutStage,
            ProviderTransport, ProviderTransportError, ProviderTransportResponse, UpdateProvider,
        },
        worker::{OfficialAiProcessor, disabled_mcp_provenance_hash},
    },
    db::{
        entities::{entry, plugin_installation},
        migrate,
    },
    plugins::{
        PluginRegistryRepository,
        runtime::{
            AiBrokerError, AiBrokerErrorKind, AiBrokerRequest, AiBrokerResponse,
            AiCapabilityBroker, BrokerInvocationContext, PluginRuntime,
        },
    },
};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, sea_query::Expr};
use secrecy::SecretString;
use serde_json::{Value, json};
use support::{
    database::{
        ENTRY_A_ID, HASH_A, HASH_D, SUBSCRIPTION_A_ID, USER_A_ID, connect_for_contract,
        insert_entry, insert_feed, insert_subscription, insert_user,
    },
    official_ai_component::{compiled_official_ai_plugin, official_ai_component},
    plugin::signed_bundle,
};
use time::{OffsetDateTime, macros::datetime};

const PLUGIN_KEY: &str = "raindrop.ai-content";
const SUMMARY_SCHEMA: &str = "raindrop://schemas/artifacts/ai-summary/v1";
const TRANSLATION_SCHEMA: &str = "raindrop://schemas/artifacts/ai-translation/v1";

#[derive(Clone, Copy)]
enum TransportMode {
    Success,
    RateLimited,
    Timeout,
}

#[derive(Default)]
struct TransportState {
    calls: AtomicUsize,
}

struct RecordingTransport {
    mode: TransportMode,
    state: Arc<TransportState>,
}

#[derive(Default)]
struct BrokerObservation {
    responses: Mutex<Vec<AiBrokerResponse>>,
    errors: Mutex<Vec<AiBrokerErrorKind>>,
}

struct ObservedBroker<B> {
    inner: B,
    observation: Arc<BrokerObservation>,
}

#[async_trait]
impl<B> AiCapabilityBroker for ObservedBroker<B>
where
    B: AiCapabilityBroker,
{
    async fn generate_structured(
        &self,
        context: &BrokerInvocationContext,
        request: AiBrokerRequest,
    ) -> Result<AiBrokerResponse, AiBrokerError> {
        let result = self.inner.generate_structured(context, request).await;
        match &result {
            Ok(response) => self
                .observation
                .responses
                .lock()
                .expect("broker response lock")
                .push(response.clone()),
            Err(error) => self
                .observation
                .errors
                .lock()
                .expect("broker error lock")
                .push(error.kind()),
        }
        result
    }
}

#[async_trait]
impl ProviderTransport for RecordingTransport {
    async fn execute(
        &self,
        provider_id: &str,
        _endpoint: &ProviderEndpoint,
        request: EncodedProviderRequest,
    ) -> Result<ProviderTransportResponse, ProviderTransportError> {
        self.state.calls.fetch_add(1, Ordering::SeqCst);
        match self.mode {
            TransportMode::Success => {
                let request_json: Value =
                    serde_json::from_slice(request.body()).expect("provider request JSON");
                let output = match request_json
                    .pointer("/text/format/name")
                    .and_then(Value::as_str)
                {
                    Some("raindrop_ai_summary_v1") => summary_output(),
                    Some("raindrop_ai_translation_v1") => translation_output(),
                    other => panic!("unexpected provider schema name: {other:?}"),
                };
                Ok(ProviderTransportResponse::new(
                    StatusCode::OK,
                    openai_response(output),
                    None,
                ))
            }
            TransportMode::RateLimited => Ok(ProviderTransportResponse::new(
                StatusCode::TOO_MANY_REQUESTS,
                Vec::new(),
                Some(ProviderRetryAfter::from_deadline(
                    OffsetDateTime::now_utc() + time::Duration::seconds(70),
                )),
            )),
            TransportMode::Timeout => Err(ProviderTransportError::timeout(
                provider_id,
                ProviderTimeoutStage::FirstByte,
            )),
        }
    }
}

#[tokio::test]
async fn processor_executes_real_summary_and_translation_and_commits_artifacts() {
    let fixture = Fixture::new("success", TransportMode::Success, false).await;

    for operation in [
        ContentJobOperation::Summarize,
        ContentJobOperation::Translate,
    ] {
        let claim = fixture
            .enqueue_claim(operation, IdentityOverrides::default())
            .await;
        let remaining = fixture
            .content_repository
            .heartbeat(&claim)
            .await
            .expect("fresh claim heartbeat")
            .remaining_attempt();
        let processed = fixture
            .processor
            .process(&claim, remaining)
            .await
            .unwrap_or_else(|failure| {
                panic!(
                    "official processor should succeed: {failure:?}, usage={:?}, broker_errors={:?}, broker_responses={:?}",
                    failure.attempt_failure().usage(),
                    fixture
                        .broker_observation
                        .errors
                        .lock()
                        .expect("broker error lock"),
                    fixture
                        .broker_observation
                        .responses
                        .lock()
                        .expect("broker response lock"),
                )
            });
        assert_eq!(processed.usage().provider_request_count(), 1);
        assert_eq!(processed.usage().mcp_call_count(), 0);
        assert_eq!(processed.usage().input_tokens(), 12);
        assert_eq!(processed.usage().output_tokens(), 7);
        assert_eq!(processed.usage().estimated_cost_micros(), 2);
        assert_eq!(
            processed.usage().execution_metadata_json(),
            r#"{"estimatedCostComplete":true,"inputTokensComplete":true,"outputTokensComplete":true,"schemaVersion":1}"#,
        );
        assert_eq!(processed.artifact().provider_label(), "model-v1");
        match operation {
            ContentJobOperation::Summarize => {
                assert_eq!(
                    processed.artifact().identity().kind(),
                    ArtifactKind::AiSummary
                );
                assert!(
                    processed
                        .artifact()
                        .payload_json()
                        .contains("Fixture summary.")
                );
            }
            ContentJobOperation::Translate => {
                assert_eq!(
                    processed.artifact().identity().kind(),
                    ArtifactKind::AiTranslation
                );
                assert!(processed.artifact().payload_json().contains("翻译正文"));
            }
        }

        let job_id = claim.job_id().to_owned();
        let (artifact, usage) = processed.into_parts();
        fixture
            .content_repository
            .complete_success(&claim, artifact, usage)
            .await
            .expect("processor result should commit atomically");
        assert_eq!(
            fixture
                .content_repository
                .get_job(USER_A_ID, &job_id)
                .await
                .expect("committed job")
                .status(),
            JobStatus::Succeeded
        );
        let stored = fixture
            .content_repository
            .get_result(USER_A_ID, &job_id)
            .await
            .expect("committed artifact result");
        assert_eq!(stored.artifact().provider_label(), "model-v1");
    }

    assert_eq!(fixture.transport_state.calls.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn processor_rejects_snapshot_drift_before_provider_io() {
    let entry_fixture = Fixture::new("entry-drift", TransportMode::Success, false).await;
    let entry_claim = entry_fixture
        .enqueue_claim(ContentJobOperation::Summarize, IdentityOverrides::default())
        .await;
    entry::Entity::update_many()
        .col_expr(entry::Column::ContentHash, Expr::value(HASH_A))
        .filter(entry::Column::Id.eq(ENTRY_A_ID))
        .exec(&entry_fixture.database)
        .await
        .expect("entry drift update");
    assert_failure(&entry_fixture, &entry_claim, "EXECUTION_SNAPSHOT_STALE").await;

    let config_fixture = Fixture::new("config-drift", TransportMode::Success, false).await;
    let config_claim = config_fixture
        .enqueue_claim(ContentJobOperation::Summarize, IdentityOverrides::default())
        .await;
    config_fixture
        .plugin_repository
        .replace_ai_config(
            PLUGIN_KEY,
            USER_A_ID,
            Some(0),
            true,
            &config_document(config_fixture.provider.id(), false, 2048),
        )
        .await
        .expect("config drift update");
    assert_failure(&config_fixture, &config_claim, "EXECUTION_SNAPSHOT_STALE").await;

    let provider_fixture = Fixture::new("provider-drift", TransportMode::Success, false).await;
    let provider_claim = provider_fixture
        .enqueue_claim(ContentJobOperation::Summarize, IdentityOverrides::default())
        .await;
    provider_fixture
        .provider_repository
        .update(
            provider_fixture.provider.id(),
            &ProviderScope::Instance,
            UpdateProvider {
                expected_revision: 0,
                model: Some("model-v2".to_owned()),
                ..UpdateProvider::default()
            },
        )
        .await
        .expect("provider drift update");
    assert_failure(&provider_fixture, &provider_claim, "PROVIDER_BINDING_STALE").await;

    let plugin_fixture = Fixture::new("plugin-drift", TransportMode::Success, false).await;
    let plugin_claim = plugin_fixture
        .enqueue_claim(ContentJobOperation::Summarize, IdentityOverrides::default())
        .await;
    plugin_installation::Entity::update_many()
        .col_expr(
            plugin_installation::Column::SystemState,
            Expr::value("DISABLED"),
        )
        .filter(plugin_installation::Column::PluginKey.eq(PLUGIN_KEY))
        .exec(&plugin_fixture.database)
        .await
        .expect("plugin state drift update");
    assert_failure(&plugin_fixture, &plugin_claim, "PLUGIN_UNAVAILABLE").await;

    for (name, overrides) in [
        (
            "prompt-drift",
            IdentityOverrides {
                prompt_version: Some("raindrop-summary-v2"),
                ..IdentityOverrides::default()
            },
        ),
        (
            "schema-drift",
            IdentityOverrides {
                schema_id: Some("raindrop://schemas/artifacts/ai-summary/v2"),
                ..IdentityOverrides::default()
            },
        ),
        (
            "component-drift",
            IdentityOverrides {
                component_digest: Some(HASH_A),
                ..IdentityOverrides::default()
            },
        ),
    ] {
        let fixture = Fixture::new(name, TransportMode::Success, false).await;
        let claim = fixture
            .enqueue_claim(ContentJobOperation::Summarize, overrides)
            .await;
        let expected = if name == "component-drift" {
            "PLUGIN_UNAVAILABLE"
        } else {
            "EXECUTION_SNAPSHOT_STALE"
        };
        assert_failure(&fixture, &claim, expected).await;
    }

    let mcp_fixture = Fixture::new("mcp", TransportMode::Success, true).await;
    let mcp_claim = mcp_fixture
        .enqueue_claim(ContentJobOperation::Summarize, IdentityOverrides::default())
        .await;
    assert_failure(&mcp_fixture, &mcp_claim, "MCP_UNAVAILABLE").await;
}

#[tokio::test]
async fn processor_maps_rate_limit_and_timeout_into_bounded_failures() {
    let rate_fixture = Fixture::new("rate", TransportMode::RateLimited, false).await;
    let rate_claim = rate_fixture
        .enqueue_claim(ContentJobOperation::Summarize, IdentityOverrides::default())
        .await;
    let rate = rate_fixture
        .processor
        .process(&rate_claim, Duration::from_secs(30))
        .await
        .expect_err("rate limit should fail");
    let rate = rate.attempt_failure();
    assert_eq!(rate.error_code(), "PROVIDER_RATE_LIMITED");
    assert!(rate.retryable());
    assert!(!rate.outcome_unknown());
    assert!(rate.retry_after().is_some_and(|value| {
        value >= Duration::from_secs(60) && value <= Duration::from_secs(70)
    }));
    assert_eq!(rate.usage().provider_request_count(), 1);
    assert_eq!(rate.usage().input_tokens(), 0);

    let timeout_fixture = Fixture::new("timeout", TransportMode::Timeout, false).await;
    let timeout_claim = timeout_fixture
        .enqueue_claim(ContentJobOperation::Summarize, IdentityOverrides::default())
        .await;
    let timeout = timeout_fixture
        .processor
        .process(&timeout_claim, Duration::from_secs(30))
        .await
        .expect_err("timeout should fail");
    let timeout = timeout.attempt_failure();
    assert_eq!(timeout.error_code(), "PROVIDER_TIMEOUT");
    assert!(timeout.retryable());
    assert!(timeout.outcome_unknown());
    assert_eq!(timeout.retry_after(), None);
    assert_eq!(timeout.usage().provider_request_count(), 1);
}

#[derive(Clone, Copy, Default)]
struct IdentityOverrides {
    prompt_version: Option<&'static str>,
    schema_id: Option<&'static str>,
    component_digest: Option<&'static str>,
}

struct Fixture {
    _data: tempfile::TempDir,
    database: sea_orm::DatabaseConnection,
    content_repository: Arc<ContentRepository>,
    plugin_repository: Arc<PluginRegistryRepository>,
    provider_repository: Arc<ProviderRepository>,
    processor: OfficialAiProcessor,
    provider: ProviderMetadata,
    config_hash: String,
    component_digest: String,
    transport_state: Arc<TransportState>,
    broker_observation: Arc<BrokerObservation>,
}

impl Fixture {
    async fn new(name: &str, mode: TransportMode, mcp_enabled: bool) -> Self {
        let data = tempfile::tempdir().expect("temporary worker processor directory");
        let url = format!(
            "sqlite://{}?mode=rwc",
            data.path()
                .join(format!("content-worker-{name}.db"))
                .display()
        );
        let database = connect_for_contract(SecretString::from(url)).await;
        migrate(&database).await.expect("database migration");
        let now = datetime!(2026-07-19 12:00:00 UTC);
        insert_user(&database, USER_A_ID, &format!("content-worker-{name}")).await;
        insert_feed(&database, now).await;
        insert_subscription(&database, SUBSCRIPTION_A_ID, USER_A_ID, now).await;
        insert_entry(
            &database,
            ENTRY_A_ID,
            1,
            "content-worker-entry",
            HASH_A,
            Some(1_752_926_400_000_000),
            now,
        )
        .await;

        let key = SecretString::from(format!("primary:{}", URL_SAFE_NO_PAD.encode([0x41_u8; 32])));
        let keyring = ProviderSecretKeyring::from_entries(&[key]).expect("provider keyring");
        let provider_repository = Arc::new(ProviderRepository::new(
            database.clone(),
            Some(Arc::new(keyring)),
        ));
        let provider = provider_repository
            .create(CreateProvider {
                scope: ProviderScope::Instance,
                display_name: "Worker provider".to_owned(),
                kind: ProviderKind::OpenAiResponses,
                endpoint: None,
                model: "model-v1".to_owned(),
                credential: SecretString::from("provider-key".to_owned()),
                capabilities: ProviderCapabilities {
                    supports_usage: true,
                    supports_idempotency: true,
                    supports_streaming: false,
                },
                policy: ProviderPolicy {
                    max_concurrency: 2,
                    requests_per_minute: None,
                    max_input_tokens_per_request: 64 * 1024,
                    max_output_tokens_per_request: 16_384,
                    input_cost_micros_per_million_tokens: Some(1_000),
                    output_cost_micros_per_million_tokens: Some(2_000),
                    max_cost_micros_per_request: Some(250_000),
                },
                is_enabled: true,
            })
            .await
            .expect("provider creation");

        let runtime = PluginRuntime::new().expect("plugin runtime");
        let component = official_ai_component();
        let bundle = signed_bundle("1.0.0", component);
        let plugin_repository = Arc::new(PluginRegistryRepository::new(database.clone()));
        plugin_repository
            .sync_bundled(&bundle)
            .await
            .expect("official plugin sync");
        let config = plugin_repository
            .replace_ai_config(
                PLUGIN_KEY,
                USER_A_ID,
                None,
                true,
                &config_document(provider.id(), mcp_enabled, 1024),
            )
            .await
            .expect("official plugin config");
        let compiled = Arc::new(compiled_official_ai_plugin(&runtime));
        let component_digest = compiled.component_digest().to_owned();
        let transport_state = Arc::new(TransportState::default());
        let broker_observation = Arc::new(BrokerObservation::default());
        let broker = Arc::new(ObservedBroker {
            inner: ProviderAiBroker::new(
                Arc::clone(&provider_repository),
                Arc::new(ProviderClient::new(RecordingTransport {
                    mode,
                    state: Arc::clone(&transport_state),
                })),
            ),
            observation: Arc::clone(&broker_observation),
        });
        let content_repository = Arc::new(ContentRepository::new(database.clone()));
        let processor = OfficialAiProcessor::new(
            Arc::clone(&content_repository),
            Arc::clone(&plugin_repository),
            Arc::clone(&provider_repository),
            runtime,
            compiled,
            broker,
        )
        .expect("official processor construction");

        Self {
            _data: data,
            database,
            content_repository,
            plugin_repository,
            provider_repository,
            processor,
            provider,
            config_hash: config.config_hash().to_owned(),
            component_digest,
            transport_state,
            broker_observation,
        }
    }

    async fn enqueue_claim(
        &self,
        operation: ContentJobOperation,
        overrides: IdentityOverrides,
    ) -> ContentJobClaim {
        let target_locale =
            (operation == ContentJobOperation::Translate).then(|| "zh-CN".to_owned());
        let prompt_version = overrides.prompt_version.unwrap_or(match operation {
            ContentJobOperation::Summarize => "raindrop-summary-v1",
            ContentJobOperation::Translate => "raindrop-translation-v1",
        });
        let schema_id = overrides.schema_id.unwrap_or(match operation {
            ContentJobOperation::Summarize => SUMMARY_SCHEMA,
            ContentJobOperation::Translate => TRANSLATION_SCHEMA,
        });
        let identity = ArtifactIdentity::new(ArtifactIdentityInput {
            user_id: USER_A_ID.to_owned(),
            entry_id: ENTRY_A_ID.to_owned(),
            kind: operation.artifact_kind(),
            target_locale: target_locale.clone(),
            entry_content_hash: HASH_D.to_owned(),
            input_hash: invocation_input_hash(operation, target_locale.as_deref()),
            config_hash: self.config_hash.clone(),
            plugin_key: PLUGIN_KEY.to_owned(),
            plugin_version: "1.0.0".to_owned(),
            component_digest: overrides
                .component_digest
                .unwrap_or(&self.component_digest)
                .to_owned(),
            provider_binding_id: self.provider.id().to_owned(),
            provider_kind: self.provider.kind(),
            provider_model: self.provider.model().to_owned(),
            provider_revision: self.provider.revision(),
            prompt_version: prompt_version.to_owned(),
            schema_id: schema_id.to_owned(),
            mcp_provenance_hash: disabled_mcp_provenance_hash(),
        })
        .expect("artifact identity");
        let operation_name = operation.as_storage().to_ascii_lowercase();
        let enqueue = EnqueueContentJob::new(EnqueueContentJobInput {
            operation,
            trigger: ContentJobTrigger::ManualApi,
            identity,
            idempotency_key: format!("processor-{operation_name}"),
            call_chain_id: format!("chain-{operation_name}"),
            remaining_depth: 2,
        })
        .expect("content enqueue request");
        match self
            .content_repository
            .enqueue(enqueue)
            .await
            .expect("content enqueue")
        {
            EnqueueResult::Queued(_) => {}
            other => panic!("expected queued job, got {other:?}"),
        }
        match self
            .content_repository
            .claim_next(ClaimContentJob::new(format!("processor-{operation_name}")).unwrap())
            .await
            .expect("content claim")
        {
            ClaimOutcome::Claimed(claim) => claim,
            other => panic!("expected claimed job, got {other:?}"),
        }
    }
}

async fn assert_failure(fixture: &Fixture, claim: &ContentJobClaim, expected_code: &str) {
    let failure = fixture
        .processor
        .process(claim, Duration::from_secs(30))
        .await
        .expect_err("snapshot drift should fail");
    assert_eq!(failure.attempt_failure().error_code(), expected_code);
    assert_eq!(fixture.transport_state.calls.load(Ordering::SeqCst), 0);
    assert!(!format!("{failure:?} {failure}").contains("Safe content"));
}

fn invocation_input_hash(operation: ContentJobOperation, target_locale: Option<&str>) -> String {
    let canonical = serde_json::to_string(&json!({
        "entry": {
            "canonicalUrl": "https://example.com/articles/1",
            "contentHash": HASH_D,
            "entryId": ENTRY_A_ID,
            "feedId": support::database::FEED_ID,
            "sourceLocale": null,
            "text": "Safe content",
            "title": "Entry 1",
        },
        "operation": operation.as_storage(),
        "schemaVersion": 1,
        "targetLocale": target_locale,
    }))
    .expect("canonical invocation JSON");
    contextual_hash("raindrop.content-invocation-input.v1", canonical.as_bytes())
}

fn contextual_hash(context: &str, value: &[u8]) -> String {
    let mut hasher = blake3::Hasher::new_derive_key(context);
    hasher.update(&(value.len() as u64).to_be_bytes());
    hasher.update(value);
    hasher.finalize().to_hex().to_string()
}

fn config_document(provider_id: &str, mcp_enabled: bool, summary_tokens: u32) -> Vec<u8> {
    let mcp = if mcp_enabled {
        json!({
            "mode": "CONTEXT_ENRICHMENT",
            "failurePolicy": "FAIL_OPEN",
            "maxToolCalls": 1,
            "tools": [{
                "connectionId": "00000000-0000-4000-8000-000000000777",
                "toolName": "search.read"
            }]
        })
    } else {
        json!({
            "mode": "DISABLED",
            "failurePolicy": "FAIL_OPEN",
            "maxToolCalls": 0,
            "tools": []
        })
    };
    serde_json::to_vec(&json!({
        "schemaVersion": 1,
        "operations": {
            "summarize": {
                "enabled": true,
                "providerId": provider_id,
                "style": "BALANCED",
                "maxOutputTokens": summary_tokens,
                "mcp": mcp,
            },
            "translate": {
                "enabled": true,
                "providerId": provider_id,
                "defaultTargetLocale": "zh-CN",
                "maxOutputTokens": 2048,
                "mcp": {
                    "mode": "DISABLED",
                    "failurePolicy": "FAIL_CLOSED",
                    "maxToolCalls": 0,
                    "tools": []
                }
            }
        },
        "automatic": {
            "enabled": false,
            "operations": ["SUMMARIZE"],
            "allSubscribedFeeds": false,
            "feedIds": [],
            "categoryIds": []
        }
    }))
    .expect("AI config JSON")
}

fn summary_output() -> Value {
    json!({
        "schemaVersion": 1,
        "sourceLanguage": "en",
        "summary": "Fixture summary.",
        "bullets": [],
        "conclusion": null,
    })
}

fn translation_output() -> Value {
    json!({
        "schemaVersion": 1,
        "targetLocale": "zh-CN",
        "detectedSourceLanguage": "en",
        "title": "翻译标题",
        "bodyMarkdown": "翻译正文",
    })
}

fn openai_response(output: Value) -> Vec<u8> {
    serde_json::to_vec(&json!({
        "id": "resp_worker",
        "model": "model-v1",
        "status": "completed",
        "output": [{
            "type": "message",
            "status": "completed",
            "content": [{
                "type": "output_text",
                "text": serde_json::to_string(&output).expect("canonical output"),
                "annotations": []
            }]
        }],
        "usage": {"input_tokens": 12, "output_tokens": 7, "total_tokens": 19}
    }))
    .expect("OpenAI fixture response")
}
