#[allow(dead_code)]
mod support;

use std::{
    fs,
    path::PathBuf,
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use raindrop::plugins::{
    AiContentConfig,
    runtime::{
        BrokerInvocationContext, CapabilitySession, CapabilitySessionConfig, CapabilityToolBinding,
        CapabilityToolBindingInput, CompiledPlugin, DenyAiBroker, DenyMcpBroker, PluginFailureCode,
        PluginRuntime, PluginRuntimeError, PluginRuntimeErrorKind, bindings::types,
    },
};
use serde_json::json;
use support::{
    plugin::signed_bundle,
    plugin_component::{
        ComponentBehavior, component_with_behavior, component_with_unexpected_import,
    },
};
use tokio::time::Instant;

const PROVIDER_BINDING_ID: &str = "provider-binding-1";

#[tokio::test]
async fn sandbox_executes_valid_component_and_returns_validated_artifact() {
    let runtime = PluginRuntime::new().expect("runtime should construct");
    let compiled = compile_behavior(&runtime, ComponentBehavior::Success);

    let (session, request) = execution_inputs(&compiled);
    let artifact = runtime
        .execute(&compiled, session, request)
        .await
        .expect("valid component should execute");

    assert_eq!(
        artifact.schema_id,
        "raindrop://schemas/artifacts/ai-summary/v1"
    );
    assert_eq!(artifact.locale, None);
    assert!(artifact.payload_json.contains("Fixture summary."));
    assert_eq!(artifact.provenance_json, r#"{"fixture":true}"#);

    let (session, request) = execution_inputs(&compiled);
    let replay = runtime
        .execute(&compiled, session, request)
        .await
        .expect("a fresh Store should permit an identical second invocation");
    assert_eq!(replay.payload_json, artifact.payload_json);
}

#[tokio::test]
async fn sandbox_requires_matching_descriptor_before_business_calls() {
    let runtime = PluginRuntime::new().expect("runtime should construct");
    let mismatch = compile_behavior(&runtime, ComponentBehavior::DescriptorMismatch);
    let (session, request) = execution_inputs(&mismatch);
    assert_error(
        runtime.execute(&mismatch, session, request).await,
        PluginRuntimeErrorKind::DescriptorMismatch,
    );

    let descriptor_trap = compile_behavior(&runtime, ComponentBehavior::DescriptorTrap);
    let (session, request) = execution_inputs(&descriptor_trap);
    assert_error(
        runtime.execute(&descriptor_trap, session, request).await,
        PluginRuntimeErrorKind::GuestTrap,
    );

    let execute_trap = compile_behavior(&runtime, ComponentBehavior::ExecuteTrap);
    let (session, request) = execution_inputs(&execute_trap);
    assert_error(
        runtime.execute(&execute_trap, session, request).await,
        PluginRuntimeErrorKind::GuestTrap,
    );
}

#[tokio::test]
async fn sandbox_preserves_only_allowlisted_guest_failure_codes() {
    let runtime = PluginRuntime::new().expect("runtime should construct");
    let known = compile_behavior(&runtime, ComponentBehavior::KnownPluginError);
    let (session, request) = execution_inputs(&known);
    let known = runtime
        .execute(&known, session, request)
        .await
        .expect_err("known guest failure should return a runtime error");
    assert_eq!(known.kind(), PluginRuntimeErrorKind::InvalidInvocation);
    assert_eq!(known.failure_code(), Some(PluginFailureCode::ConfigInvalid));
    assert!(
        !format!("{known:?} {known}").contains("raindrop.ai-content.config-invalid"),
        "fixed message keys should not be rendered directly",
    );

    let unknown = compile_behavior(&runtime, ComponentBehavior::UnknownPluginError);
    let (session, request) = execution_inputs(&unknown);
    let unknown = runtime
        .execute(&unknown, session, request)
        .await
        .expect_err("unknown guest failure should return a runtime error");
    assert_eq!(unknown.kind(), PluginRuntimeErrorKind::InvalidInvocation);
    assert_eq!(unknown.failure_code(), None);
    assert!(!format!("{unknown:?} {unknown}").contains("rd-secret"));
}

#[tokio::test]
async fn sandbox_enforces_memory_and_distinct_execute_lifecycle_fuel() {
    let runtime = PluginRuntime::new().expect("runtime should construct");
    let memory = compile_behavior(&runtime, ComponentBehavior::MemoryLimit);
    let (session, request) = execution_inputs(&memory);
    assert_error(
        runtime.execute(&memory, session, request).await,
        PluginRuntimeErrorKind::MemoryLimit,
    );

    let fuel = compile_behavior(&runtime, ComponentBehavior::FuelSplit);
    let (session, request) = execution_inputs(&fuel);
    runtime
        .execute(&fuel, session, request)
        .await
        .expect("execute should have enough 50M fuel for the fixed workload");
    assert_error(
        runtime
            .on_event(&fuel, lifecycle_session(), lifecycle_request(&fuel))
            .await,
        PluginRuntimeErrorKind::FuelExhausted,
    );
}

#[tokio::test]
async fn sandbox_denies_unknown_imports_and_untrusted_request_or_output_shapes() {
    let runtime = PluginRuntime::new().expect("runtime should construct");
    let ambient_component = component_with_unexpected_import();
    let ambient_bundle = signed_bundle("1.0.0", &ambient_component);
    let ambient = CompiledPlugin::compile(&runtime, &ambient_bundle, &ambient_component)
        .expect("hostile component should compile before linking");
    let (session, request) = execution_inputs(&ambient);
    assert_error(
        runtime.execute(&ambient, session, request).await,
        PluginRuntimeErrorKind::LinkDenied,
    );

    let success = compile_behavior(&runtime, ComponentBehavior::Success);
    let (session, mut digest_mismatch) = execution_inputs(&success);
    digest_mismatch.component_digest = "b".repeat(64);
    assert_error(
        runtime.execute(&success, session, digest_mismatch).await,
        PluginRuntimeErrorKind::InvalidInvocation,
    );
    let (session, mut job_mismatch) = execution_inputs(&success);
    job_mismatch.job_id = "attacker-job".to_owned();
    assert_error(
        runtime.execute(&success, session, job_mismatch).await,
        PluginRuntimeErrorKind::InvalidInvocation,
    );
    let (session, mut oversized_text) = execution_inputs(&success);
    oversized_text.entry.text = "x".repeat(512 * 1024 + 1);
    assert_error(
        runtime.execute(&success, session, oversized_text).await,
        PluginRuntimeErrorKind::InvalidInvocation,
    );
    let (session, mut budget_mismatch) = execution_inputs(&success);
    budget_mismatch.budget.remaining_provider_requests = 2;
    assert_error(
        runtime.execute(&success, session, budget_mismatch).await,
        PluginRuntimeErrorKind::InvalidInvocation,
    );

    let oversized = compile_behavior(&runtime, ComponentBehavior::OutputTooLarge);
    let (session, request) = execution_inputs(&oversized);
    assert_error(
        runtime.execute(&oversized, session, request).await,
        PluginRuntimeErrorKind::OutputTooLarge,
    );
    let invalid = compile_behavior(&runtime, ComponentBehavior::InvalidArtifact);
    let (session, request) = execution_inputs(&invalid);
    assert_error(
        runtime.execute(&invalid, session, request).await,
        PluginRuntimeErrorKind::InvalidInvocation,
    );

    let (session, request) = execution_inputs_with_tool(&success);
    runtime
        .execute(&success, session, request)
        .await
        .expect("exact host-issued tool descriptor should execute");
    assert_tool_drift(&runtime, &success, |binding| {
        binding.connection_id = "00000000-0000-4000-8000-000000000099".to_owned();
    })
    .await;
    assert_tool_drift(&runtime, &success, |binding| {
        binding.tool_name = "other.read".to_owned();
    })
    .await;
    assert_tool_drift(&runtime, &success, |binding| {
        binding.description = "substituted description".to_owned();
    })
    .await;
    assert_tool_drift(&runtime, &success, |binding| {
        binding.input_schema_json = r#"{"type":"array"}"#.to_owned();
    })
    .await;
    assert_tool_drift(&runtime, &success, |binding| {
        binding.input_schema_digest = "f".repeat(64);
    })
    .await;
}

#[tokio::test]
async fn sandbox_lifecycle_requires_verified_config_and_identity() {
    let runtime = PluginRuntime::new().expect("runtime should construct");
    let compiled = compile_behavior(&runtime, ComponentBehavior::Success);
    let outcome = runtime
        .on_event(&compiled, lifecycle_session(), lifecycle_request(&compiled))
        .await
        .expect("valid lifecycle request should execute");
    assert!(outcome.job_intents.is_empty());
    assert!(outcome.diagnostics.is_empty());

    for mutate in [
        (|request: &mut types::LifecycleRequest| {
            request.invocation_id = "other-invocation".to_owned();
        }) as fn(&mut types::LifecycleRequest),
        |request| request.plugin_key = "attacker.plugin".to_owned(),
        |request| request.component_digest = "f".repeat(64),
        |request| request.config_hash = "f".repeat(64),
        |request| {
            request.config_json = serde_json::to_string_pretty(
                &serde_json::from_str::<serde_json::Value>(&request.config_json)
                    .expect("config value"),
            )
            .expect("pretty config");
        },
        |request| request.event.user_scope.subject = "other-user".to_owned(),
        |request| request.event.event_type = "feed.refresh.completed".to_owned(),
    ] {
        let mut request = lifecycle_request(&compiled);
        mutate(&mut request);
        assert_error(
            runtime
                .on_event(&compiled, lifecycle_session(), request)
                .await,
            PluginRuntimeErrorKind::InvalidInvocation,
        );
    }
}

#[test]
fn runtime_source_has_no_ambient_capability_or_persistence_shortcut() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/plugins/runtime");
    let mut source = String::new();
    for file in [
        "bindings.rs",
        "capability.rs",
        "component.rs",
        "engine.rs",
        "error.rs",
        "execute.rs",
        "host.rs",
        "mod.rs",
    ] {
        source.push_str(&fs::read_to_string(root.join(file)).expect("runtime source should read"));
    }
    for forbidden in [
        "wasmtime_wasi",
        "WasiCtx",
        "ProviderClient",
        "DatabaseConnection",
        "PluginRegistryRepository",
        "reqwest::",
        "TcpStream",
        "UnixStream",
        "Command::new",
    ] {
        assert!(
            !source.contains(forbidden),
            "forbidden runtime path: {forbidden}"
        );
    }
    let component = fs::read_to_string(root.join("component.rs")).expect("component source");
    assert!(component.contains("Component::from_binary"));
    assert!(!component.contains("Component::new"));
    assert!(!component.contains("deserialize"));
    let execute = fs::read_to_string(root.join("execute.rs")).expect("execute source");
    let lifecycle_start = execute
        .find("pub async fn on_event")
        .expect("lifecycle method");
    let lifecycle_end = execute[lifecycle_start..]
        .find("async fn instantiate")
        .map(|offset| lifecycle_start + offset)
        .expect("lifecycle method end");
    assert!(!execute[lifecycle_start..lifecycle_end].contains("activate()"));
}

fn compile_behavior(runtime: &PluginRuntime, behavior: ComponentBehavior) -> CompiledPlugin {
    let component = component_with_behavior(behavior);
    let bundle = signed_bundle("1.0.0", &component);
    CompiledPlugin::compile(runtime, &bundle, &component).expect("fixture component should compile")
}

fn execution_inputs(compiled: &CompiledPlugin) -> (CapabilitySession, types::OperationRequest) {
    let (deadline_unix_ms, deadline) = invocation_deadline(Duration::from_secs(30));
    (
        session(
            types::Trigger::ManualApi,
            0,
            deadline_unix_ms,
            deadline,
            Vec::new(),
        ),
        operation_request(compiled, deadline_unix_ms),
    )
}

fn execution_inputs_with_tool(
    compiled: &CompiledPlugin,
) -> (CapabilitySession, types::OperationRequest) {
    let (deadline_unix_ms, deadline) = invocation_deadline(Duration::from_secs(30));
    let binding = tool_binding();
    let mut request = operation_request(compiled, deadline_unix_ms);
    request.tool_bindings = vec![binding.to_wit()];
    request.budget.remaining_mcp_calls = 1;
    (
        session(
            types::Trigger::ManualApi,
            1,
            deadline_unix_ms,
            deadline,
            vec![binding],
        ),
        request,
    )
}

fn lifecycle_session() -> CapabilitySession {
    let (deadline_unix_ms, deadline) = invocation_deadline(Duration::from_secs(30));
    session(
        types::Trigger::FeedRefreshPersisted,
        0,
        deadline_unix_ms,
        deadline,
        Vec::new(),
    )
}

fn session(
    trigger: types::Trigger,
    remaining_mcp_calls: u32,
    deadline_unix_ms: u64,
    deadline: Instant,
    tool_bindings: Vec<CapabilityToolBinding>,
) -> CapabilitySession {
    CapabilitySession::new(
        CapabilitySessionConfig {
            invocation: BrokerInvocationContext {
                invocation_id: "invocation-1".to_owned(),
                job_id: "job-1".to_owned(),
                user_subject: "user-1".to_owned(),
                call_chain_id: "call-chain-1".to_owned(),
                operation: types::Operation::Summarize,
                trigger,
                remaining_depth: 2,
            },
            provider_binding_id: PROVIDER_BINDING_ID.to_owned(),
            tool_bindings,
            remaining_provider_requests: 3,
            remaining_mcp_calls,
            remaining_input_tokens: 8_192,
            remaining_output_tokens: 4_096,
            remaining_cost_micros: 250_000,
            deadline_unix_ms,
            deadline,
        },
        Arc::new(DenyAiBroker),
        Arc::new(DenyMcpBroker),
    )
    .expect("sandbox session should construct")
}

fn tool_binding() -> CapabilityToolBinding {
    let input_schema_json = r#"{"additionalProperties":false,"properties":{"query":{"type":"string"}},"type":"object"}"#.to_owned();
    let input_schema_digest = {
        let mut hasher = blake3::Hasher::new_derive_key("raindrop.mcp-tool-input-schema.v1");
        hasher.update(&(input_schema_json.len() as u64).to_be_bytes());
        hasher.update(input_schema_json.as_bytes());
        hasher.finalize().to_hex().to_string()
    };
    CapabilityToolBinding::new(CapabilityToolBindingInput {
        binding_id: "tool-binding-1".to_owned(),
        connection_id: "00000000-0000-4000-8000-000000000001".to_owned(),
        tool_name: "search.read".to_owned(),
        display_label: "Search".to_owned(),
        description: "Untrusted search tool description".to_owned(),
        input_schema_json,
        input_schema_digest,
    })
    .expect("sandbox tool binding")
}

async fn assert_tool_drift(
    runtime: &PluginRuntime,
    compiled: &CompiledPlugin,
    mutate: fn(&mut types::ToolBinding),
) {
    let (session, mut request) = execution_inputs_with_tool(compiled);
    mutate(&mut request.tool_bindings[0]);
    assert_error(
        runtime.execute(compiled, session, request).await,
        PluginRuntimeErrorKind::InvalidInvocation,
    );
}

fn operation_request(compiled: &CompiledPlugin, deadline_unix_ms: u64) -> types::OperationRequest {
    let config = ai_config();
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
        operation: types::Operation::Summarize,
        target_locale: None,
        entry: types::EntryReference {
            entry_id: "entry-1".to_owned(),
            feed_id: "feed-1".to_owned(),
            content_hash: "a".repeat(64),
            title: "Fixture title".to_owned(),
            text: "Fixture entry text".to_owned(),
            canonical_url: Some("https://example.com/articles/1".to_owned()),
            source_locale: Some("en".to_owned()),
        },
        config_json: config.canonical_json().to_owned(),
        config_hash: config.config_hash().to_owned(),
        provider_binding_id: PROVIDER_BINDING_ID.to_owned(),
        tool_bindings: Vec::new(),
        call_chain_id: "call-chain-1".to_owned(),
        budget: types::InvocationBudget {
            remaining_depth: 2,
            deadline_unix_ms,
            remaining_provider_requests: 3,
            remaining_mcp_calls: 0,
            remaining_input_tokens: 8_192,
            remaining_output_tokens: 4_096,
            remaining_cost_micros: 250_000,
        },
    }
}

fn invocation_deadline(duration: Duration) -> (u64, Instant) {
    let unix_now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock should follow Unix epoch");
    let deadline_unix_ms =
        u64::try_from((unix_now + duration).as_millis()).expect("deadline should fit u64");
    (deadline_unix_ms, Instant::now() + duration)
}

fn lifecycle_event() -> types::LifecycleEvent {
    types::LifecycleEvent {
        event_id: "00000000-0000-4000-8000-000000010004".to_owned(),
        event_type: "feed.refresh.persisted".to_owned(),
        schema_version: 1,
        refresh_id: "00000000-0000-4000-8000-000000020001".to_owned(),
        sequence: 10,
        occurred_at: "2026-07-19T12:00:03Z".to_owned(),
        idempotency_key:
            "refresh:00000000-0000-4000-8000-000000020001:persisted:v1".to_owned(),
        user_scope: types::UserScope {
            subject: "user-1".to_owned(),
        },
        context_json: r#"{"commitGeneration":42,"droppedCount":0,"feedId":"00000000-0000-4000-8000-000000030001","newCount":1,"newEntries":[{"contentHash":"b3788c7661d79a104f4dfa15f2c284f5f7ee35f9b3dfbd520c4e1ef3d068cf65","entryId":"00000000-0000-4000-8000-000000040001"}],"updatedCount":0,"updatedEntries":[]}"#.to_owned(),
    }
}

fn lifecycle_request(compiled: &CompiledPlugin) -> types::LifecycleRequest {
    let config = ai_config();
    types::LifecycleRequest {
        invocation_id: "invocation-1".to_owned(),
        plugin_key: compiled.plugin_key().to_owned(),
        plugin_version: compiled.version().to_owned(),
        component_digest: compiled.component_digest().to_owned(),
        config_json: config.canonical_json().to_owned(),
        config_hash: config.config_hash().to_owned(),
        event: lifecycle_event(),
    }
}

fn ai_config() -> AiContentConfig {
    let config = json!({
        "schemaVersion": 1,
        "operations": {
            "summarize": {
                "enabled": true,
                "providerId": "00000000-0000-4000-8000-000000000901",
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
                "providerId": "00000000-0000-4000-8000-000000000902",
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
    });
    AiContentConfig::parse(&serde_json::to_vec(&config).expect("config JSON"))
        .expect("fixture config should parse")
}

fn assert_error<T>(result: Result<T, PluginRuntimeError>, expected: PluginRuntimeErrorKind) {
    let error = match result {
        Ok(_) => panic!("expected {expected:?}"),
        Err(error) => error,
    };
    assert_eq!(error.kind(), expected);
    assert!(!format!("{error:?} {error}").contains("Fixture entry text"));
}
