#[allow(dead_code)]
mod support;

use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use async_trait::async_trait;
use raindrop::plugins::{
    AiContentConfig,
    runtime::{
        AiBrokerError, AiBrokerRequest, AiBrokerResponse, AiCapabilityBroker, AiFinishReason,
        BrokerInvocationContext, CapabilitySession, CapabilitySessionConfig, CapabilityToolBinding,
        CapabilityToolBindingInput, CompiledPlugin, DenyMcpBroker, McpBrokerError,
        McpBrokerErrorKind, McpBrokerRequest, McpBrokerResponse, McpCapabilityBroker,
        PluginFailureCode, PluginRuntime, PluginRuntimeErrorKind, bindings::types,
    },
};
use serde_json::{Value, json};
use support::{official_ai_component::official_ai_component, plugin::signed_bundle};
use tokio::time::Instant;

const PROVIDER_ID: &str = "00000000-0000-4000-8000-000000000101";
const CONNECTION_ID: &str = "00000000-0000-4000-8000-000000000201";

#[tokio::test]
async fn real_component_executes_direct_summary_and_translation() {
    let runtime = PluginRuntime::new().expect("runtime");
    let compiled = compile(&runtime);

    let summary_broker = Arc::new(RecordingAiBroker::new([Ok(ai_response(summary_output()))]));
    let summary_config = ai_config(types::Operation::Summarize, false, "FAIL_OPEN", false);
    let (summary_session, summary_request) = operation_inputs(
        &compiled,
        types::Operation::Summarize,
        &summary_config,
        Vec::new(),
        summary_broker.clone(),
        Arc::new(DenyMcpBroker),
    );
    let summary = runtime
        .execute(&compiled, summary_session, summary_request)
        .await
        .expect("real summary component should execute");
    assert_eq!(
        summary.schema_id,
        "raindrop://schemas/artifacts/ai-summary/v1"
    );
    assert_eq!(summary.locale, None);
    assert_eq!(summary.payload_json, summary_output());
    assert!(summary.provenance_json.contains("raindrop-summary-v1"));
    let requests = summary_broker.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].provider_request_ordinal, 1);
    assert!(!requests[0].system_instruction.contains("rd-secret-entry"));
    assert!(requests[0].untrusted_input_json.contains("rd-secret-entry"));

    let translation_broker = Arc::new(RecordingAiBroker::new([Ok(ai_response(
        translation_output(),
    ))]));
    let translation_config = ai_config(types::Operation::Translate, false, "FAIL_OPEN", false);
    let (translation_session, translation_request) = operation_inputs(
        &compiled,
        types::Operation::Translate,
        &translation_config,
        Vec::new(),
        translation_broker.clone(),
        Arc::new(DenyMcpBroker),
    );
    let translation = runtime
        .execute(&compiled, translation_session, translation_request)
        .await
        .expect("real translation component should execute");
    assert_eq!(
        translation.schema_id,
        "raindrop://schemas/artifacts/ai-translation/v1"
    );
    assert_eq!(translation.locale.as_deref(), Some("zh-CN"));
    assert_eq!(translation.payload_json, translation_output());
    assert_eq!(translation_broker.requests().len(), 1);
}

#[tokio::test]
async fn real_component_executes_mcp_applied_fail_open_and_fail_closed() {
    let runtime = PluginRuntime::new().expect("runtime");
    let compiled = compile(&runtime);
    let binding = tool_binding();
    let plan = canonical(json!({
        "schemaVersion": 1,
        "calls": [{"toolBindingId":"binding-a","arguments":{"query":"rust"}}],
    }));

    let applied_ai = Arc::new(RecordingAiBroker::new([
        Ok(ai_response(plan.clone())),
        Ok(ai_response(summary_output())),
    ]));
    let applied_mcp = Arc::new(RecordingMcpBroker::new([Ok(McpBrokerResponse {
        result_json: r#"{"facts":["safe"]}"#.to_owned(),
        connection_label: "Search".to_owned(),
        tool_label: "Read".to_owned(),
    })]));
    let applied_config = ai_config(types::Operation::Summarize, true, "FAIL_OPEN", false);
    let (applied_session, applied_request) = operation_inputs(
        &compiled,
        types::Operation::Summarize,
        &applied_config,
        vec![binding.clone()],
        applied_ai.clone(),
        applied_mcp.clone(),
    );
    let applied = runtime
        .execute(&compiled, applied_session, applied_request)
        .await
        .expect("MCP applied flow should execute");
    assert!(applied.provenance_json.contains(r#""status":"APPLIED""#));
    assert_eq!(applied_ai.requests().len(), 2);
    assert_eq!(applied_mcp.requests().len(), 1);
    assert!(
        applied_ai.requests()[1]
            .untrusted_input_json
            .contains("mcpContext")
    );

    let fail_open_ai = Arc::new(RecordingAiBroker::new([
        Ok(ai_response(plan.clone())),
        Ok(ai_response(summary_output())),
    ]));
    let fail_open_mcp = Arc::new(RecordingMcpBroker::new([Err(McpBrokerError::new(
        McpBrokerErrorKind::Timeout,
        true,
    ))]));
    let fail_open_config = ai_config(types::Operation::Summarize, true, "FAIL_OPEN", false);
    let (fail_open_session, fail_open_request) = operation_inputs(
        &compiled,
        types::Operation::Summarize,
        &fail_open_config,
        vec![binding.clone()],
        fail_open_ai.clone(),
        fail_open_mcp,
    );
    let fail_open = runtime
        .execute(&compiled, fail_open_session, fail_open_request)
        .await
        .expect("MCP fail-open should still generate");
    assert!(fail_open.provenance_json.contains(r#""status":"DEGRADED""#));
    assert_eq!(fail_open_ai.requests().len(), 2);
    assert!(
        !fail_open_ai.requests()[1]
            .untrusted_input_json
            .contains("mcpContext")
    );

    let fail_closed_ai = Arc::new(RecordingAiBroker::new([Ok(ai_response(plan))]));
    let fail_closed_mcp = Arc::new(RecordingMcpBroker::new([Err(McpBrokerError::new(
        McpBrokerErrorKind::Timeout,
        true,
    ))]));
    let fail_closed_config = ai_config(types::Operation::Summarize, true, "FAIL_CLOSED", false);
    let (fail_closed_session, fail_closed_request) = operation_inputs(
        &compiled,
        types::Operation::Summarize,
        &fail_closed_config,
        vec![binding],
        fail_closed_ai.clone(),
        fail_closed_mcp,
    );
    let error = runtime
        .execute(&compiled, fail_closed_session, fail_closed_request)
        .await
        .expect_err("MCP fail-closed should stop before final generation");
    assert_eq!(error.kind(), PluginRuntimeErrorKind::GuestTrap);
    assert_eq!(error.failure_code(), Some(PluginFailureCode::McpTimeout));
    assert_eq!(fail_closed_ai.requests().len(), 1);
}

#[tokio::test]
async fn real_component_emits_lifecycle_intents_without_capability_calls() {
    let runtime = PluginRuntime::new().expect("runtime");
    let compiled = compile(&runtime);
    let config = ai_config(types::Operation::Summarize, false, "FAIL_OPEN", true);
    let ai = Arc::new(RecordingAiBroker::new([]));
    let mcp = Arc::new(RecordingMcpBroker::new([]));
    let outcome = runtime
        .on_event(
            &compiled,
            lifecycle_session(ai.clone(), mcp.clone()),
            lifecycle_request(&compiled, &config),
        )
        .await
        .expect("real lifecycle component should emit intents");
    assert_eq!(outcome.job_intents.len(), 1);
    assert_eq!(
        outcome.job_intents[0].operation,
        types::Operation::Summarize
    );
    assert_eq!(
        outcome.job_intents[0].entry_id,
        "00000000-0000-4000-8000-000000000401"
    );
    assert!(
        outcome.job_intents[0]
            .idempotency_key
            .contains("plugin:raindrop.ai-content")
    );
    assert!(ai.requests().is_empty());
    assert!(mcp.requests().is_empty());
}

fn compile(runtime: &PluginRuntime) -> CompiledPlugin {
    let component = official_ai_component();
    let bundle = signed_bundle("1.0.0", component);
    CompiledPlugin::compile(runtime, &bundle, component).expect("real component should compile")
}

fn operation_inputs(
    compiled: &CompiledPlugin,
    operation: types::Operation,
    config: &AiContentConfig,
    bindings: Vec<CapabilityToolBinding>,
    ai: Arc<dyn AiCapabilityBroker>,
    mcp: Arc<dyn McpCapabilityBroker>,
) -> (CapabilitySession, types::OperationRequest) {
    let (deadline_unix_ms, deadline) = invocation_deadline(Duration::from_secs(30));
    let remaining_mcp_calls = u32::try_from(bindings.len()).expect("binding count");
    let wit_bindings = bindings.iter().map(CapabilityToolBinding::to_wit).collect();
    (
        session(
            operation,
            types::Trigger::ManualApi,
            bindings,
            3,
            remaining_mcp_calls,
            64 * 1024,
            max_output_budget(operation),
            deadline_unix_ms,
            deadline,
            ai,
            mcp,
        ),
        operation_request(
            compiled,
            operation,
            config,
            wit_bindings,
            remaining_mcp_calls,
            deadline_unix_ms,
        ),
    )
}

fn lifecycle_session(
    ai: Arc<dyn AiCapabilityBroker>,
    mcp: Arc<dyn McpCapabilityBroker>,
) -> CapabilitySession {
    let (deadline_unix_ms, deadline) = invocation_deadline(Duration::from_secs(30));
    session(
        types::Operation::Summarize,
        types::Trigger::FeedRefreshPersisted,
        Vec::new(),
        0,
        0,
        0,
        0,
        deadline_unix_ms,
        deadline,
        ai,
        mcp,
    )
}

#[allow(clippy::too_many_arguments)]
fn session(
    operation: types::Operation,
    trigger: types::Trigger,
    bindings: Vec<CapabilityToolBinding>,
    remaining_provider_requests: u32,
    remaining_mcp_calls: u32,
    remaining_input_tokens: u32,
    remaining_output_tokens: u32,
    deadline_unix_ms: u64,
    deadline: Instant,
    ai: Arc<dyn AiCapabilityBroker>,
    mcp: Arc<dyn McpCapabilityBroker>,
) -> CapabilitySession {
    CapabilitySession::new(
        CapabilitySessionConfig {
            invocation: BrokerInvocationContext {
                invocation_id: "invocation-1".to_owned(),
                job_id: "job-1".to_owned(),
                user_subject: "user-1".to_owned(),
                call_chain_id: "call-chain-1".to_owned(),
                operation,
                trigger,
                remaining_depth: 2,
            },
            provider_binding_id: PROVIDER_ID.to_owned(),
            tool_bindings: bindings,
            remaining_provider_requests,
            remaining_mcp_calls,
            remaining_input_tokens,
            remaining_output_tokens,
            remaining_cost_micros: 250_000,
            deadline_unix_ms,
            deadline,
        },
        ai,
        mcp,
    )
    .expect("component session")
}

fn operation_request(
    compiled: &CompiledPlugin,
    operation: types::Operation,
    config: &AiContentConfig,
    tool_bindings: Vec<types::ToolBinding>,
    remaining_mcp_calls: u32,
    deadline_unix_ms: u64,
) -> types::OperationRequest {
    types::OperationRequest {
        invocation_id: "invocation-1".to_owned(),
        job_id: "job-1".to_owned(),
        idempotency_key: "idempotency-1".to_owned(),
        plugin_key: compiled.plugin_key().to_owned(),
        plugin_version: compiled.version().to_owned(),
        component_digest: compiled.component_digest().to_owned(),
        user_scope: types::UserScope {
            subject: "user-1".to_owned(),
        },
        trigger: types::Trigger::ManualApi,
        operation,
        target_locale: (operation == types::Operation::Translate).then(|| "zh-CN".to_owned()),
        entry: types::EntryReference {
            entry_id: "entry-1".to_owned(),
            feed_id: "feed-1".to_owned(),
            content_hash: "a".repeat(64),
            title: "Fixture title".to_owned(),
            text: "rd-secret-entry".to_owned(),
            canonical_url: Some("https://example.test/articles/1".to_owned()),
            source_locale: Some("en".to_owned()),
        },
        config_json: config.canonical_json().to_owned(),
        config_hash: config.config_hash().to_owned(),
        provider_binding_id: PROVIDER_ID.to_owned(),
        tool_bindings,
        call_chain_id: "call-chain-1".to_owned(),
        budget: types::InvocationBudget {
            remaining_depth: 2,
            deadline_unix_ms,
            remaining_provider_requests: 3,
            remaining_mcp_calls,
            remaining_input_tokens: 64 * 1024,
            remaining_output_tokens: max_output_budget(operation),
            remaining_cost_micros: 250_000,
        },
    }
}

fn lifecycle_request(
    compiled: &CompiledPlugin,
    config: &AiContentConfig,
) -> types::LifecycleRequest {
    types::LifecycleRequest {
        invocation_id: "invocation-1".to_owned(),
        plugin_key: compiled.plugin_key().to_owned(),
        plugin_version: compiled.version().to_owned(),
        component_digest: compiled.component_digest().to_owned(),
        config_json: config.canonical_json().to_owned(),
        config_hash: config.config_hash().to_owned(),
        event: types::LifecycleEvent {
            event_id: "00000000-0000-4000-8000-000000000501".to_owned(),
            event_type: "feed.refresh.persisted".to_owned(),
            schema_version: 1,
            refresh_id: "00000000-0000-4000-8000-000000000601".to_owned(),
            sequence: 10,
            occurred_at: "2026-07-19T12:00:03Z".to_owned(),
            idempotency_key: "refresh:00000000-0000-4000-8000-000000000601:persisted:v1".to_owned(),
            user_scope: types::UserScope {
                subject: "user-1".to_owned(),
            },
            context_json: canonical(json!({
                "feedId": "00000000-0000-4000-8000-000000000301",
                "commitGeneration": 42,
                "newCount": 1,
                "updatedCount": 0,
                "droppedCount": 0,
                "newEntries": [{
                    "entryId": "00000000-0000-4000-8000-000000000401",
                    "contentHash": "b".repeat(64),
                }],
                "updatedEntries": [],
            })),
        },
    }
}

fn ai_config(
    operation: types::Operation,
    mcp_enabled: bool,
    failure_policy: &str,
    automatic: bool,
) -> AiContentConfig {
    let summarize_enabled = operation == types::Operation::Summarize || automatic;
    let translate_enabled = operation == types::Operation::Translate;
    let value = json!({
        "schemaVersion": 1,
        "operations": {
            "summarize": {
                "enabled": summarize_enabled,
                "providerId": PROVIDER_ID,
                "style": "BALANCED",
                "maxOutputTokens": 512,
                "mcp": if mcp_enabled {
                    json!({
                        "mode": "CONTEXT_ENRICHMENT",
                        "failurePolicy": failure_policy,
                        "maxToolCalls": 1,
                        "tools": [{"connectionId": CONNECTION_ID, "toolName": "search.read"}],
                    })
                } else {
                    json!({
                        "mode": "DISABLED",
                        "failurePolicy": failure_policy,
                        "maxToolCalls": 0,
                        "tools": [],
                    })
                },
            },
            "translate": {
                "enabled": translate_enabled,
                "providerId": PROVIDER_ID,
                "defaultTargetLocale": "zh-CN",
                "maxOutputTokens": 1024,
                "mcp": {
                    "mode": "DISABLED",
                    "failurePolicy": "FAIL_CLOSED",
                    "maxToolCalls": 0,
                    "tools": [],
                },
            },
        },
        "automatic": {
            "enabled": automatic,
            "operations": ["SUMMARIZE"],
            "allSubscribedFeeds": automatic,
            "feedIds": [],
            "categoryIds": [],
        },
    });
    AiContentConfig::parse(canonical(value).as_bytes()).expect("component config")
}

fn tool_binding() -> CapabilityToolBinding {
    let input_schema_json =
        r#"{"additionalProperties":false,"properties":{"query":{"type":"string"}},"type":"object"}"#
            .to_owned();
    CapabilityToolBinding::new(CapabilityToolBindingInput {
        binding_id: "binding-a".to_owned(),
        connection_id: CONNECTION_ID.to_owned(),
        tool_name: "search.read".to_owned(),
        display_label: "Search".to_owned(),
        description: "rd-secret-tool-description".to_owned(),
        input_schema_digest: tool_schema_digest(&input_schema_json),
        input_schema_json,
    })
    .expect("tool binding")
}

fn summary_output() -> String {
    canonical(json!({
        "schemaVersion": 1,
        "sourceLanguage": "en",
        "summary": "Fixture summary.",
        "bullets": [],
        "conclusion": null,
    }))
}

fn translation_output() -> String {
    canonical(json!({
        "schemaVersion": 1,
        "detectedSourceLanguage": "en",
        "targetLocale": "zh-CN",
        "title": "标题",
        "bodyMarkdown": "正文",
    }))
}

fn ai_response(output_json: String) -> AiBrokerResponse {
    AiBrokerResponse {
        output_json,
        finish_reason: AiFinishReason::Completed,
        input_tokens: None,
        output_tokens: None,
        model_label: "fixture-model".to_owned(),
        estimated_cost_micros: None,
    }
}

fn max_output_budget(operation: types::Operation) -> u32 {
    match operation {
        types::Operation::Summarize => 4_096,
        types::Operation::Translate => 8_192,
    }
}

fn invocation_deadline(duration: Duration) -> (u64, Instant) {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time");
    (
        u64::try_from((now + duration).as_millis()).expect("deadline"),
        Instant::now() + duration,
    )
}

fn tool_schema_digest(input_schema_json: &str) -> String {
    let mut hasher = blake3::Hasher::new_derive_key("raindrop.mcp-tool-input-schema.v1");
    hasher.update(&(input_schema_json.len() as u64).to_be_bytes());
    hasher.update(input_schema_json.as_bytes());
    hasher.finalize().to_hex().to_string()
}

fn canonical(value: Value) -> String {
    serde_json::to_string(&value).expect("canonical JSON")
}

struct RecordingAiBroker {
    responses: Mutex<VecDeque<Result<AiBrokerResponse, AiBrokerError>>>,
    requests: Mutex<Vec<AiBrokerRequest>>,
}

impl RecordingAiBroker {
    fn new(responses: impl IntoIterator<Item = Result<AiBrokerResponse, AiBrokerError>>) -> Self {
        Self {
            responses: Mutex::new(responses.into_iter().collect()),
            requests: Mutex::new(Vec::new()),
        }
    }

    fn requests(&self) -> Vec<AiBrokerRequest> {
        self.requests.lock().expect("AI request lock").clone()
    }
}

#[async_trait]
impl AiCapabilityBroker for RecordingAiBroker {
    async fn generate_structured(
        &self,
        _context: &BrokerInvocationContext,
        request: AiBrokerRequest,
    ) -> Result<AiBrokerResponse, AiBrokerError> {
        self.requests.lock().expect("AI request lock").push(request);
        self.responses
            .lock()
            .expect("AI response lock")
            .pop_front()
            .expect("AI fixture response")
    }
}

struct RecordingMcpBroker {
    responses: Mutex<VecDeque<Result<McpBrokerResponse, McpBrokerError>>>,
    requests: Mutex<Vec<McpBrokerRequest>>,
}

impl RecordingMcpBroker {
    fn new(responses: impl IntoIterator<Item = Result<McpBrokerResponse, McpBrokerError>>) -> Self {
        Self {
            responses: Mutex::new(responses.into_iter().collect()),
            requests: Mutex::new(Vec::new()),
        }
    }

    fn requests(&self) -> Vec<McpBrokerRequest> {
        self.requests.lock().expect("MCP request lock").clone()
    }
}

#[async_trait]
impl McpCapabilityBroker for RecordingMcpBroker {
    async fn call_tool(
        &self,
        _context: &BrokerInvocationContext,
        request: McpBrokerRequest,
    ) -> Result<McpBrokerResponse, McpBrokerError> {
        self.requests
            .lock()
            .expect("MCP request lock")
            .push(request);
        self.responses
            .lock()
            .expect("MCP response lock")
            .pop_front()
            .expect("MCP fixture response")
    }
}
