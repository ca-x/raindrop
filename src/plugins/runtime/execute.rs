use std::time::{Duration, Instant};

use serde_json::json;
use url::Url;
use wasmtime::{
    CallHook, ResourceLimiter, Store, StoreLimits, StoreLimitsBuilder, Trap,
    component::{HasSelf, Linker},
};

use crate::plugins::{
    AiContentConfig, LifecycleEvent, SummaryArtifact, TranslationArtifact,
    json::{
        canonical_json, normalize_locale, parse_unique_json, validate_lower_hex_hash,
        validate_text, validate_visible_ascii,
    },
};

use super::{
    CapabilitySession, CompiledPlugin, PluginRuntime, PluginRuntimeError, PluginRuntimeErrorKind,
    bindings::{ContentPluginV1, ContentPluginV1Pre, types},
};

const MAX_LINEAR_MEMORY_BYTES: usize = 64 * 1024 * 1024;
const MAX_MEMORIES: usize = 2;
const MAX_TABLES: usize = 4;
const MAX_INSTANCES: usize = 4;
const MAX_TABLE_ELEMENTS: usize = 10_000;
const EXECUTE_FUEL: u64 = 50_000_000;
const LIFECYCLE_FUEL: u64 = 5_000_000;
const MAX_GUEST_CPU: Duration = Duration::from_secs(2);
const EPOCH_TICK: Duration = Duration::from_millis(10);
const MAX_REQUEST_BYTES: usize = 1024 * 1024;
const MAX_ENTRY_TEXT_BYTES: usize = 512 * 1024;
const MAX_CONFIG_BYTES: usize = 256 * 1024;
const MAX_EVENT_CONTEXT_BYTES: usize = 64 * 1024;
const MAX_OUTPUT_BYTES: usize = 512 * 1024;
const SUMMARY_SCHEMA: &str = "raindrop://schemas/artifacts/ai-summary/v1";
const TRANSLATION_SCHEMA: &str = "raindrop://schemas/artifacts/ai-translation/v1";

impl PluginRuntime {
    pub async fn execute(
        &self,
        compiled: &CompiledPlugin,
        session: CapabilitySession,
        request: types::OperationRequest,
    ) -> Result<types::ArtifactCandidate, PluginRuntimeError> {
        validate_operation_request(compiled, &session, &request)?;
        let (mut store, guest) = self.instantiate(compiled, session, EXECUTE_FUEL).await?;
        validate_descriptor(compiled, call_descriptor(&guest, &mut store).await?)?;
        store.data_mut().capability.activate();
        let result = guest
            .raindrop_content_plugin_content_plugin()
            .call_execute(&mut store, &request)
            .await
            .map_err(classify_guest_error)?
            .map_err(map_plugin_error)?;
        validate_artifact(&request, result)
    }

    pub async fn on_event(
        &self,
        compiled: &CompiledPlugin,
        session: CapabilitySession,
        event: types::LifecycleEvent,
    ) -> Result<types::EventOutcome, PluginRuntimeError> {
        validate_lifecycle_event(&session, &event)?;
        let (mut store, guest) = self.instantiate(compiled, session, LIFECYCLE_FUEL).await?;
        validate_descriptor(compiled, call_descriptor(&guest, &mut store).await?)?;
        store.data_mut().capability.activate();
        let result = guest
            .raindrop_content_plugin_content_plugin()
            .call_on_event(&mut store, &event)
            .await
            .map_err(classify_guest_error)?
            .map_err(map_plugin_error)?;
        validate_event_outcome(result)
    }

    async fn instantiate(
        &self,
        compiled: &CompiledPlugin,
        mut capability: CapabilitySession,
        fuel: u64,
    ) -> Result<(Store<StoreState>, ContentPluginV1), PluginRuntimeError> {
        capability.suspend();
        let mut linker = Linker::new(self.engine());
        ContentPluginV1::add_to_linker::<StoreState, HasSelf<CapabilitySession>>(
            &mut linker,
            |state| &mut state.capability,
        )
        .map_err(|_| PluginRuntimeError::new(PluginRuntimeErrorKind::LinkDenied))?;
        let pre = linker
            .instantiate_pre(compiled.component())
            .map_err(|_| PluginRuntimeError::new(PluginRuntimeErrorKind::LinkDenied))?;
        let pre = ContentPluginV1Pre::new(pre)
            .map_err(|_| PluginRuntimeError::new(PluginRuntimeErrorKind::LinkDenied))?;
        let mut store = new_store(self, capability, fuel, MAX_GUEST_CPU)?;
        let guest = pre
            .instantiate_async(&mut store)
            .await
            .map_err(classify_instantiation_error)?;
        Ok((store, guest))
    }
}

struct StoreState {
    capability: CapabilitySession,
    limits: RuntimeLimits,
    guest_cpu: GuestCpuBudget,
}

fn new_store(
    runtime: &PluginRuntime,
    capability: CapabilitySession,
    fuel: u64,
    guest_cpu: Duration,
) -> Result<Store<StoreState>, PluginRuntimeError> {
    let mut store = Store::new(
        runtime.engine(),
        StoreState {
            capability,
            limits: RuntimeLimits::new(),
            guest_cpu: GuestCpuBudget::new(guest_cpu),
        },
    );
    store.limiter(|state| &mut state.limits);
    store
        .set_fuel(fuel)
        .map_err(|_| PluginRuntimeError::new(PluginRuntimeErrorKind::RuntimeUnavailable))?;
    store.epoch_deadline_trap();
    store.set_epoch_deadline(GuestCpuBudget::ticks(guest_cpu));
    store.call_hook(|mut context, hook| {
        let deadline = context
            .data_mut()
            .guest_cpu
            .transition(hook)
            .map_err(wasmtime::Error::new)?;
        if let Some(deadline) = deadline {
            context.set_epoch_deadline(deadline);
        }
        Ok(())
    });
    Ok(store)
}

struct RuntimeLimits {
    inner: StoreLimits,
}

impl RuntimeLimits {
    fn new() -> Self {
        Self {
            inner: StoreLimitsBuilder::new()
                .memory_size(MAX_LINEAR_MEMORY_BYTES)
                .memories(MAX_MEMORIES)
                .tables(MAX_TABLES)
                .instances(MAX_INSTANCES)
                .table_elements(MAX_TABLE_ELEMENTS)
                .trap_on_grow_failure(true)
                .build(),
        }
    }

    fn denied<T>() -> wasmtime::Result<T> {
        Err(wasmtime::Error::new(PluginRuntimeError::new(
            PluginRuntimeErrorKind::MemoryLimit,
        )))
    }
}

impl ResourceLimiter for RuntimeLimits {
    fn memory_growing(
        &mut self,
        current: usize,
        desired: usize,
        maximum: Option<usize>,
    ) -> wasmtime::Result<bool> {
        match self.inner.memory_growing(current, desired, maximum) {
            Ok(true) => Ok(true),
            Ok(false) | Err(_) => Self::denied(),
        }
    }

    fn memory_grow_failed(&mut self, _error: wasmtime::Error) -> wasmtime::Result<()> {
        Self::denied()
    }

    fn table_growing(
        &mut self,
        current: usize,
        desired: usize,
        maximum: Option<usize>,
    ) -> wasmtime::Result<bool> {
        match self.inner.table_growing(current, desired, maximum) {
            Ok(true) => Ok(true),
            Ok(false) | Err(_) => Self::denied(),
        }
    }

    fn table_grow_failed(&mut self, _error: wasmtime::Error) -> wasmtime::Result<()> {
        Self::denied()
    }

    fn instances(&self) -> usize {
        self.inner.instances()
    }

    fn tables(&self) -> usize {
        self.inner.tables()
    }

    fn memories(&self) -> usize {
        self.inner.memories()
    }
}

struct GuestCpuBudget {
    remaining: Duration,
    entered_at: Option<Instant>,
}

impl GuestCpuBudget {
    const fn new(remaining: Duration) -> Self {
        Self {
            remaining,
            entered_at: None,
        }
    }

    fn transition(&mut self, hook: CallHook) -> Result<Option<u64>, PluginRuntimeError> {
        self.transition_at(hook, Instant::now())
    }

    fn transition_at(
        &mut self,
        hook: CallHook,
        now: Instant,
    ) -> Result<Option<u64>, PluginRuntimeError> {
        if hook.exiting_host() {
            if self.remaining.is_zero() || self.entered_at.is_some() {
                return Err(PluginRuntimeError::new(
                    PluginRuntimeErrorKind::GuestTimeout,
                ));
            }
            self.entered_at = Some(now);
            Ok(Some(Self::ticks(self.remaining)))
        } else {
            let entered_at = self
                .entered_at
                .take()
                .ok_or_else(|| PluginRuntimeError::new(PluginRuntimeErrorKind::GuestTrap))?;
            self.remaining = self
                .remaining
                .saturating_sub(now.duration_since(entered_at));
            if self.remaining.is_zero() {
                Err(PluginRuntimeError::new(
                    PluginRuntimeErrorKind::GuestTimeout,
                ))
            } else {
                Ok(None)
            }
        }
    }

    fn ticks(remaining: Duration) -> u64 {
        let tick_nanos = EPOCH_TICK.as_nanos();
        let ticks = remaining.as_nanos().div_ceil(tick_nanos);
        u64::try_from(ticks.max(1)).unwrap_or(u64::MAX)
    }
}

async fn call_descriptor(
    guest: &ContentPluginV1,
    store: &mut Store<StoreState>,
) -> Result<types::PluginDescriptor, PluginRuntimeError> {
    guest
        .raindrop_content_plugin_content_plugin()
        .call_descriptor(store)
        .await
        .map_err(classify_guest_error)
}

fn validate_descriptor(
    compiled: &CompiledPlugin,
    descriptor: types::PluginDescriptor,
) -> Result<(), PluginRuntimeError> {
    let valid = descriptor.plugin_key == compiled.plugin_key()
        && descriptor.version == compiled.version()
        && descriptor.abi == compiled.abi_version()
        && descriptor.operations == [types::Operation::Summarize, types::Operation::Translate]
        && descriptor.lifecycle_subscriptions.len() == 1
        && descriptor.lifecycle_subscriptions[0].event == "feed.refresh.persisted"
        && descriptor.lifecycle_subscriptions[0].schema_version == 1
        && descriptor.required_capabilities == ["ai.generate_structured"]
        && descriptor.optional_capabilities == ["mcp.call_tool"];
    if valid {
        Ok(())
    } else {
        Err(PluginRuntimeError::new(
            PluginRuntimeErrorKind::DescriptorMismatch,
        ))
    }
}

fn validate_operation_request(
    compiled: &CompiledPlugin,
    session: &CapabilitySession,
    request: &types::OperationRequest,
) -> Result<(), PluginRuntimeError> {
    if request.entry.text.len() > MAX_ENTRY_TEXT_BYTES
        || request.config_json.len() > MAX_CONFIG_BYTES
        || operation_request_size(request) > MAX_REQUEST_BYTES
    {
        return Err(PluginRuntimeError::new(
            PluginRuntimeErrorKind::InvalidInvocation,
        ));
    }
    session.validate_operation_request(request)?;
    let identity_matches = request.plugin_key == compiled.plugin_key()
        && request.plugin_version == compiled.version()
        && request.component_digest == compiled.component_digest();
    let identifiers_valid = [
        request.invocation_id.as_str(),
        request.job_id.as_str(),
        request.idempotency_key.as_str(),
        request.entry.entry_id.as_str(),
        request.entry.feed_id.as_str(),
        request.call_chain_id.as_str(),
    ]
    .into_iter()
    .all(|value| {
        validate_visible_ascii(
            value,
            255,
            crate::plugins::PluginRegistryErrorKind::InvalidInput,
        )
        .is_ok()
    });
    let content_valid = validate_text(
        &request.entry.title,
        64 * 1024,
        crate::plugins::PluginRegistryErrorKind::InvalidInput,
    )
    .is_ok()
        && validate_text(
            &request.entry.text,
            MAX_ENTRY_TEXT_BYTES,
            crate::plugins::PluginRegistryErrorKind::InvalidInput,
        )
        .is_ok()
        && validate_lower_hex_hash(
            &request.entry.content_hash,
            crate::plugins::PluginRegistryErrorKind::InvalidInput,
        )
        .is_ok()
        && valid_optional_locale(request.entry.source_locale.as_deref())
        && valid_optional_url(request.entry.canonical_url.as_deref());
    let target_valid = match request.operation {
        types::Operation::Summarize => request.target_locale.is_none(),
        types::Operation::Translate => request
            .target_locale
            .as_deref()
            .is_some_and(normalize_locale_exact),
    };
    let config = AiContentConfig::parse(request.config_json.as_bytes())
        .map_err(|_| PluginRuntimeError::new(PluginRuntimeErrorKind::InvalidInvocation))?;
    let config_valid = request.config_json == config.canonical_json()
        && request.config_hash == config.config_hash();
    if identity_matches && identifiers_valid && content_valid && target_valid && config_valid {
        Ok(())
    } else {
        Err(PluginRuntimeError::new(
            PluginRuntimeErrorKind::InvalidInvocation,
        ))
    }
}

fn validate_lifecycle_event(
    session: &CapabilitySession,
    event: &types::LifecycleEvent,
) -> Result<(), PluginRuntimeError> {
    session.validate_lifecycle_context(&event.user_scope.subject)?;
    if event.context_json.len() > MAX_EVENT_CONTEXT_BYTES {
        return Err(PluginRuntimeError::new(
            PluginRuntimeErrorKind::InvalidInvocation,
        ));
    }
    let context = parse_unique_json(event.context_json.as_bytes(), MAX_EVENT_CONTEXT_BYTES)
        .map_err(|_| PluginRuntimeError::new(PluginRuntimeErrorKind::InvalidInvocation))?;
    let canonical_context = canonical_json(context.clone(), MAX_EVENT_CONTEXT_BYTES)
        .map_err(|_| PluginRuntimeError::new(PluginRuntimeErrorKind::InvalidInvocation))?;
    if canonical_context != event.context_json {
        return Err(PluginRuntimeError::new(
            PluginRuntimeErrorKind::InvalidInvocation,
        ));
    }
    let envelope = json!({
        "schemaVersion": event.schema_version,
        "eventId": event.event_id,
        "eventType": event.event_type,
        "refreshId": event.refresh_id,
        "sequence": event.sequence,
        "occurredAt": event.occurred_at,
        "idempotencyKey": event.idempotency_key,
        "context": context,
    });
    let encoded = serde_json::to_vec(&envelope)
        .map_err(|_| PluginRuntimeError::new(PluginRuntimeErrorKind::InvalidInvocation))?;
    if encoded.len() > MAX_REQUEST_BYTES || LifecycleEvent::parse(&encoded).is_err() {
        return Err(PluginRuntimeError::new(
            PluginRuntimeErrorKind::InvalidInvocation,
        ));
    }
    Ok(())
}

fn validate_artifact(
    request: &types::OperationRequest,
    artifact: types::ArtifactCandidate,
) -> Result<types::ArtifactCandidate, PluginRuntimeError> {
    let total_size = artifact.schema_id.len()
        + artifact.locale.as_ref().map_or(0, String::len)
        + artifact.payload_json.len()
        + artifact.provenance_json.len();
    if total_size > MAX_OUTPUT_BYTES {
        return Err(PluginRuntimeError::new(
            PluginRuntimeErrorKind::OutputTooLarge,
        ));
    }
    let provenance = parse_unique_json(artifact.provenance_json.as_bytes(), MAX_OUTPUT_BYTES)
        .map_err(|_| PluginRuntimeError::new(PluginRuntimeErrorKind::InvalidInvocation))?;
    let provenance_is_canonical = canonical_json(provenance.clone(), MAX_OUTPUT_BYTES)
        .is_ok_and(|canonical| canonical == artifact.provenance_json);
    if !provenance.is_object() || !provenance_is_canonical {
        return Err(PluginRuntimeError::new(
            PluginRuntimeErrorKind::InvalidInvocation,
        ));
    }
    let valid = match request.operation {
        types::Operation::Summarize => {
            artifact.schema_id == SUMMARY_SCHEMA
                && artifact.locale.is_none()
                && SummaryArtifact::parse(artifact.payload_json.as_bytes()).is_ok()
        }
        types::Operation::Translate => {
            let Ok(translation) = TranslationArtifact::parse(artifact.payload_json.as_bytes())
            else {
                return Err(PluginRuntimeError::new(
                    PluginRuntimeErrorKind::InvalidInvocation,
                ));
            };
            artifact.schema_id == TRANSLATION_SCHEMA
                && artifact.locale == request.target_locale
                && translation.target_locale() == request.target_locale.as_deref().unwrap_or("")
        }
    };
    if valid {
        Ok(artifact)
    } else {
        Err(PluginRuntimeError::new(
            PluginRuntimeErrorKind::InvalidInvocation,
        ))
    }
}

fn validate_event_outcome(
    outcome: types::EventOutcome,
) -> Result<types::EventOutcome, PluginRuntimeError> {
    let size = outcome
        .job_intents
        .iter()
        .map(|intent| {
            intent.entry_id.len()
                + intent.target_locale.as_ref().map_or(0, String::len)
                + intent.idempotency_key.len()
        })
        .sum::<usize>()
        + outcome.diagnostics.iter().map(String::len).sum::<usize>();
    let valid = size <= MAX_OUTPUT_BYTES
        && outcome.job_intents.iter().all(|intent| {
            validate_visible_ascii(
                &intent.entry_id,
                255,
                crate::plugins::PluginRegistryErrorKind::InvalidInput,
            )
            .is_ok()
                && validate_visible_ascii(
                    &intent.idempotency_key,
                    255,
                    crate::plugins::PluginRegistryErrorKind::InvalidInput,
                )
                .is_ok()
                && match intent.operation {
                    types::Operation::Summarize => intent.target_locale.is_none(),
                    types::Operation::Translate => intent
                        .target_locale
                        .as_deref()
                        .is_some_and(normalize_locale_exact),
                }
        })
        && outcome.diagnostics.iter().all(|diagnostic| {
            validate_text(
                diagnostic,
                1024,
                crate::plugins::PluginRegistryErrorKind::InvalidInput,
            )
            .is_ok()
        });
    if size > MAX_OUTPUT_BYTES {
        Err(PluginRuntimeError::new(
            PluginRuntimeErrorKind::OutputTooLarge,
        ))
    } else if valid {
        Ok(outcome)
    } else {
        Err(PluginRuntimeError::new(
            PluginRuntimeErrorKind::InvalidInvocation,
        ))
    }
}

fn operation_request_size(request: &types::OperationRequest) -> usize {
    216 + request.invocation_id.len()
        + request.job_id.len()
        + request.idempotency_key.len()
        + request.plugin_key.len()
        + request.plugin_version.len()
        + request.component_digest.len()
        + request.user_scope.subject.len()
        + request.target_locale.as_ref().map_or(0, String::len)
        + request.entry.entry_id.len()
        + request.entry.feed_id.len()
        + request.entry.content_hash.len()
        + request.entry.title.len()
        + request.entry.text.len()
        + request.entry.canonical_url.as_ref().map_or(0, String::len)
        + request.entry.source_locale.as_ref().map_or(0, String::len)
        + request.config_json.len()
        + request.config_hash.len()
        + request.provider_binding_id.len()
        + request.call_chain_id.len()
        + request
            .tool_bindings
            .iter()
            .map(|binding| binding.binding_id.len() + binding.display_label.len() + 16)
            .sum::<usize>()
}

fn valid_optional_locale(locale: Option<&str>) -> bool {
    locale.is_none_or(normalize_locale_exact)
}

fn normalize_locale_exact(locale: &str) -> bool {
    normalize_locale(
        locale,
        crate::plugins::PluginRegistryErrorKind::InvalidInput,
    )
    .is_ok_and(|normalized| normalized == locale)
}

fn valid_optional_url(value: Option<&str>) -> bool {
    value.is_none_or(|value| {
        Url::parse(value).is_ok_and(|url| {
            matches!(url.scheme(), "http" | "https")
                && url.username().is_empty()
                && url.password().is_none()
                && url.host_str().is_some()
        })
    })
}

fn classify_instantiation_error(error: wasmtime::Error) -> PluginRuntimeError {
    classify_error(error, true)
}

fn classify_guest_error(error: wasmtime::Error) -> PluginRuntimeError {
    classify_error(error, false)
}

fn classify_error(error: wasmtime::Error, instantiation: bool) -> PluginRuntimeError {
    if let Some(error) = error.downcast_ref::<PluginRuntimeError>() {
        return *error;
    }
    if let Some(trap) = error.downcast_ref::<Trap>() {
        return PluginRuntimeError::new(match trap {
            Trap::OutOfFuel => PluginRuntimeErrorKind::FuelExhausted,
            Trap::Interrupt => PluginRuntimeErrorKind::GuestTimeout,
            Trap::AllocationTooLarge => PluginRuntimeErrorKind::MemoryLimit,
            _ => PluginRuntimeErrorKind::GuestTrap,
        });
    }
    PluginRuntimeError::new(if instantiation {
        PluginRuntimeErrorKind::MemoryLimit
    } else {
        PluginRuntimeErrorKind::GuestTrap
    })
}

fn map_plugin_error(error: types::PluginError) -> PluginRuntimeError {
    PluginRuntimeError::new(match error.code {
        types::PluginErrorCode::Disabled | types::PluginErrorCode::CapabilityDenied => {
            PluginRuntimeErrorKind::CapabilityDenied
        }
        types::PluginErrorCode::InvalidRequest
        | types::PluginErrorCode::ConfigInvalid
        | types::PluginErrorCode::OutputInvalid => PluginRuntimeErrorKind::InvalidInvocation,
        types::PluginErrorCode::OutputTooLarge => PluginRuntimeErrorKind::OutputTooLarge,
        types::PluginErrorCode::DeadlineExceeded => PluginRuntimeErrorKind::GuestTimeout,
        types::PluginErrorCode::BudgetExhausted | types::PluginErrorCode::HostFailure => {
            PluginRuntimeErrorKind::GuestTrap
        }
    })
}

#[cfg(test)]
mod tests {
    use std::{
        sync::Arc,
        time::{SystemTime, UNIX_EPOCH},
    };

    use tokio::time as tokio_time;
    use wasmtime::{Instance, Module};

    use super::*;
    use crate::plugins::runtime::{
        BrokerInvocationContext, CapabilitySessionConfig, DenyAiBroker, DenyMcpBroker,
    };

    #[test]
    fn translation_artifact_target_locale_must_match_the_request() {
        let request = types::OperationRequest {
            invocation_id: "invocation-1".to_owned(),
            job_id: "job-1".to_owned(),
            idempotency_key: "idempotency-1".to_owned(),
            plugin_key: "raindrop.ai-content".to_owned(),
            plugin_version: "1.0.0".to_owned(),
            component_digest: "a".repeat(64),
            user_scope: types::UserScope {
                subject: "user-1".to_owned(),
            },
            trigger: types::Trigger::ManualApi,
            operation: types::Operation::Translate,
            target_locale: Some("zh-CN".to_owned()),
            entry: types::EntryReference {
                entry_id: "entry-1".to_owned(),
                feed_id: "feed-1".to_owned(),
                content_hash: "b".repeat(64),
                title: "Title".to_owned(),
                text: "Text".to_owned(),
                canonical_url: None,
                source_locale: Some("en".to_owned()),
            },
            config_json: "{}".to_owned(),
            config_hash: "c".repeat(64),
            provider_binding_id: "provider-1".to_owned(),
            tool_bindings: Vec::new(),
            call_chain_id: "chain-1".to_owned(),
            budget: types::InvocationBudget {
                remaining_depth: 0,
                deadline_unix_ms: 1,
                remaining_provider_requests: 1,
                remaining_mcp_calls: 0,
                remaining_input_tokens: 1,
                remaining_output_tokens: 1,
                remaining_cost_micros: 1,
            },
        };
        let artifact = types::ArtifactCandidate {
            schema_id: TRANSLATION_SCHEMA.to_owned(),
            locale: Some("zh-CN".to_owned()),
            payload_json: r#"{"bodyMarkdown":"正文","detectedSourceLanguage":"en","schemaVersion":1,"targetLocale":"ja","title":"标题"}"#.to_owned(),
            provenance_json: "{}".to_owned(),
        };

        assert_eq!(
            validate_artifact(&request, artifact)
                .expect_err("payload locale drift must fail")
                .kind(),
            PluginRuntimeErrorKind::InvalidInvocation,
        );
    }

    #[test]
    fn guest_cpu_budget_excludes_time_spent_in_host_code() {
        let start = Instant::now();
        let mut budget = GuestCpuBudget::new(MAX_GUEST_CPU);
        assert_eq!(
            budget
                .transition_at(CallHook::CallingWasm, start)
                .expect("guest entry"),
            Some(200)
        );
        assert_eq!(
            budget
                .transition_at(CallHook::CallingHost, start + Duration::from_millis(100))
                .expect("host entry"),
            None
        );
        assert_eq!(
            budget
                .transition_at(CallHook::ReturningFromHost, start + Duration::from_secs(5))
                .expect("guest re-entry"),
            Some(190)
        );
        budget
            .transition_at(
                CallHook::ReturningFromWasm,
                start + Duration::from_secs(5) + Duration::from_millis(50),
            )
            .expect("guest return");
        assert_eq!(budget.remaining, Duration::from_millis(1850));
    }

    #[tokio::test]
    async fn epoch_ticker_interrupts_guest_before_fuel_when_policy_is_lowered() {
        let runtime = PluginRuntime::new().expect("runtime should construct");
        let wasm = wat::parse_str("(module (func (export \"run\") (loop $spin br $spin)))")
            .expect("loop module should parse");
        let module = Module::from_binary(runtime.engine(), &wasm).expect("module should compile");
        let mut store = new_store(&runtime, capability_session(), EXECUTE_FUEL, EPOCH_TICK)
            .expect("store should construct");
        let instance = Instance::new_async(&mut store, &module, &[])
            .await
            .expect("module should instantiate");
        let run = instance
            .get_typed_func::<(), ()>(&mut store, "run")
            .expect("run export should type-check");
        let error = run
            .call_async(&mut store, ())
            .await
            .expect_err("epoch should interrupt the loop");
        assert_eq!(
            classify_guest_error(error).kind(),
            PluginRuntimeErrorKind::GuestTimeout
        );
    }

    fn capability_session() -> CapabilitySession {
        let duration = Duration::from_secs(30);
        let unix_now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should follow Unix epoch");
        let deadline_unix_ms =
            u64::try_from((unix_now + duration).as_millis()).expect("deadline should fit u64");
        CapabilitySession::new(
            CapabilitySessionConfig {
                invocation: BrokerInvocationContext {
                    invocation_id: "test-invocation".to_owned(),
                    job_id: "test-job".to_owned(),
                    user_subject: "test-user".to_owned(),
                    call_chain_id: "test-chain".to_owned(),
                    operation: types::Operation::Summarize,
                    trigger: types::Trigger::ManualApi,
                    remaining_depth: 2,
                },
                provider_binding_id: "test-provider".to_owned(),
                tool_binding_ids: Vec::new(),
                remaining_provider_requests: 3,
                remaining_mcp_calls: 0,
                remaining_input_tokens: 1024,
                remaining_output_tokens: 1024,
                remaining_cost_micros: 1,
                deadline_unix_ms,
                deadline: tokio_time::Instant::now() + duration,
            },
            Arc::new(DenyAiBroker),
            Arc::new(DenyMcpBroker),
        )
        .expect("test capability session")
    }
}
