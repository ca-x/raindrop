use std::{
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use async_trait::async_trait;
use serde_json::json;
use tokio::time::Instant;

use crate::{
    content::{
        jobs::{
            ArtifactCandidate, AttemptUsage, ContentJobClaim, ContentJobOperation,
            ContentJobTrigger, ContentRepository, ContentRepositoryErrorKind,
        },
        provider::{ProviderCoreErrorKind, ProviderRepository},
    },
    plugins::{
        AiContentConfig, PluginRegistryErrorKind, PluginRegistryRepository, PluginSystemState,
        json::parse_unique_json,
        runtime::{
            AiCapabilityBroker, BrokerInvocationContext, CapabilityFailureHint, CapabilitySession,
            CapabilitySessionConfig, CapabilityUsage, CompiledPlugin, DenyMcpBroker,
            PluginExecutionFailure, PluginFailureCode, PluginRuntime, PluginRuntimeErrorKind,
            bindings::types,
        },
    },
};

use super::{
    ContentInvocationInput, ContentProcessFailure, ContentProcessSuccess, ContentWorkerError,
    ContentWorkerErrorKind, disabled_mcp_provenance_hash,
};

const OFFICIAL_PLUGIN_KEY: &str = "raindrop.ai-content";
const OFFICIAL_ABI: &str = "raindrop:content-plugin@1.0.0";
const SUMMARY_PROMPT_VERSION: &str = "raindrop-summary-v1";
const TRANSLATION_PROMPT_VERSION: &str = "raindrop-translation-v1";
const SUMMARY_SCHEMA_ID: &str = "raindrop://schemas/artifacts/ai-summary/v1";
const TRANSLATION_SCHEMA_ID: &str = "raindrop://schemas/artifacts/ai-translation/v1";
const MAX_PROVIDER_REQUESTS: u32 = 2;
const MAX_COST_MICROS: u64 = 250_000;
const MAX_ARTIFACT_BYTES: usize = 512 * 1024;
const MAX_PROVENANCE_BYTES: usize = 32 * 1024;
const MAX_RETRY_AFTER: Duration = Duration::from_secs(60 * 60);

#[async_trait]
pub trait ContentProcessor: Send + Sync {
    async fn process(
        &self,
        claim: &ContentJobClaim,
        remaining_attempt: Duration,
    ) -> Result<ContentProcessSuccess, ContentProcessFailure>;
}

#[derive(Clone)]
pub struct OfficialAiProcessor {
    content_repository: Arc<ContentRepository>,
    plugin_repository: Arc<PluginRegistryRepository>,
    provider_repository: Arc<ProviderRepository>,
    runtime: PluginRuntime,
    compiled: Arc<CompiledPlugin>,
    ai_broker: Arc<dyn AiCapabilityBroker>,
}

impl OfficialAiProcessor {
    pub fn new(
        content_repository: Arc<ContentRepository>,
        plugin_repository: Arc<PluginRegistryRepository>,
        provider_repository: Arc<ProviderRepository>,
        runtime: PluginRuntime,
        compiled: Arc<CompiledPlugin>,
        ai_broker: Arc<dyn AiCapabilityBroker>,
    ) -> Result<Self, ContentWorkerError> {
        if compiled.plugin_key() != OFFICIAL_PLUGIN_KEY || compiled.abi_version() != OFFICIAL_ABI {
            return Err(ContentWorkerError::new(
                ContentWorkerErrorKind::InvalidConfiguration,
            ));
        }
        Ok(Self {
            content_repository,
            plugin_repository,
            provider_repository,
            runtime,
            compiled,
            ai_broker,
        })
    }

    pub async fn process(
        &self,
        claim: &ContentJobClaim,
        remaining_attempt: Duration,
    ) -> Result<ContentProcessSuccess, ContentProcessFailure> {
        <Self as ContentProcessor>::process(self, claim, remaining_attempt).await
    }

    async fn process_claim(
        &self,
        claim: &ContentJobClaim,
        remaining_attempt: Duration,
    ) -> Result<ContentProcessSuccess, ContentProcessFailure> {
        let identity = claim.identity();
        let entry = self
            .content_repository
            .load_execution_entry(claim)
            .await
            .map_err(|error| map_entry_error(error.kind()))?;
        let input =
            ContentInvocationInput::new(&entry, claim.operation(), identity.target_locale())
                .map_err(|_| snapshot_stale())?;
        if input.hash() != identity.input_hash()
            || input.operation() != claim.operation()
            || identity.kind() != claim.operation().artifact_kind()
        {
            return Err(snapshot_stale());
        }

        let installation = self
            .plugin_repository
            .get_installation(identity.plugin_key())
            .await
            .map_err(|error| map_plugin_repository_error(error.kind()))?;
        let installation_matches = installation.plugin_key() == OFFICIAL_PLUGIN_KEY
            && installation.plugin_key() == self.compiled.plugin_key()
            && installation.version() == identity.plugin_version()
            && installation.version() == self.compiled.version()
            && installation.abi_version() == self.compiled.abi_version()
            && installation.component_digest() == identity.component_digest()
            && installation.component_digest() == self.compiled.component_digest()
            && installation.system_state() == PluginSystemState::Enabled;
        if !installation_matches {
            return Err(plugin_unavailable());
        }

        let stored_config = self
            .plugin_repository
            .get_ai_config(OFFICIAL_PLUGIN_KEY, claim.user_id())
            .await
            .map_err(|error| map_plugin_repository_error(error.kind()))?
            .ok_or_else(snapshot_stale)?;
        if stored_config.plugin_id() != installation.id()
            || stored_config.owner_user_id() != claim.user_id()
            || !stored_config.is_enabled()
            || stored_config.config_hash() != identity.config_hash()
        {
            return Err(snapshot_stale());
        }
        let operation_config =
            OperationConfig::from_config(stored_config.config(), claim.operation());
        if !operation_config.enabled
            || operation_config.provider_id != identity.provider_binding_id()
        {
            return Err(snapshot_stale());
        }
        if operation_config.mcp_enabled {
            return Err(fixed_failure(
                "MCP_UNAVAILABLE",
                false,
                false,
                None,
                AttemptUsage::empty(),
            ));
        }
        if identity.mcp_provenance_hash() != disabled_mcp_provenance_hash() {
            return Err(snapshot_stale());
        }
        let contract = OperationContract::for_operation(claim.operation());
        if identity.prompt_version() != contract.prompt_version
            || identity.schema_id() != contract.schema_id
        {
            return Err(snapshot_stale());
        }

        let binding = self
            .provider_repository
            .load_enabled_binding(identity.provider_binding_id(), claim.user_id())
            .await
            .map_err(|error| map_provider_error(error.kind()))?;
        let metadata = binding.metadata();
        let provider_matches = metadata.id() == identity.provider_binding_id()
            && metadata.kind() == identity.provider_kind()
            && metadata.model() == identity.provider_model()
            && metadata.revision() == identity.provider_revision()
            && metadata.is_enabled();
        let provider_policy = metadata.policy();
        if !provider_matches {
            return Err(provider_binding_stale());
        }
        drop(binding);

        let (deadline_unix_ms, deadline) = invocation_deadlines(claim, remaining_attempt)?;
        let remaining_input_tokens = provider_policy
            .max_input_tokens_per_request
            .saturating_mul(MAX_PROVIDER_REQUESTS);
        let operation = wit_operation(claim.operation());
        let trigger = wit_trigger(claim.trigger());
        let invocation = BrokerInvocationContext {
            invocation_id: format!(
                "{}:{}:{}",
                claim.job_id(),
                claim.attempt(),
                claim.lease_token()
            ),
            job_id: claim.job_id().to_owned(),
            user_subject: claim.user_id().to_owned(),
            call_chain_id: claim.call_chain_id().to_owned(),
            operation,
            trigger,
            remaining_depth: u32::from(claim.remaining_depth()),
            expected_provider_kind: identity.provider_kind().as_storage().to_owned(),
            expected_provider_model: identity.provider_model().to_owned(),
            expected_provider_revision: identity.provider_revision(),
        };
        let session = CapabilitySession::new(
            CapabilitySessionConfig {
                invocation,
                provider_binding_id: identity.provider_binding_id().to_owned(),
                tool_bindings: Vec::new(),
                remaining_provider_requests: MAX_PROVIDER_REQUESTS,
                remaining_mcp_calls: 0,
                remaining_input_tokens,
                remaining_output_tokens: operation_config.max_output_tokens,
                remaining_cost_micros: MAX_COST_MICROS,
                deadline_unix_ms,
                deadline,
            },
            Arc::clone(&self.ai_broker),
            Arc::new(DenyMcpBroker),
        )
        .map_err(|_| runtime_unavailable(AttemptUsage::empty()))?;
        let request = types::OperationRequest {
            invocation_id: format!(
                "{}:{}:{}",
                claim.job_id(),
                claim.attempt(),
                claim.lease_token()
            ),
            job_id: claim.job_id().to_owned(),
            idempotency_key: claim.idempotency_key().to_owned(),
            plugin_key: identity.plugin_key().to_owned(),
            plugin_version: identity.plugin_version().to_owned(),
            component_digest: identity.component_digest().to_owned(),
            user_scope: types::UserScope {
                subject: claim.user_id().to_owned(),
            },
            trigger,
            operation,
            target_locale: identity.target_locale().map(str::to_owned),
            entry: input.to_wit_entry(),
            config_json: stored_config.canonical_json().to_owned(),
            config_hash: stored_config.config_hash().to_owned(),
            provider_binding_id: identity.provider_binding_id().to_owned(),
            tool_bindings: Vec::new(),
            call_chain_id: claim.call_chain_id().to_owned(),
            budget: types::InvocationBudget {
                remaining_depth: u32::from(claim.remaining_depth()),
                deadline_unix_ms,
                remaining_provider_requests: MAX_PROVIDER_REQUESTS,
                remaining_mcp_calls: 0,
                remaining_input_tokens,
                remaining_output_tokens: operation_config.max_output_tokens,
                remaining_cost_micros: MAX_COST_MICROS,
            },
        };

        match self
            .runtime
            .execute_detailed(&self.compiled, session, request)
            .await
        {
            Ok(execution) => build_success(claim, execution),
            Err(failure) => Err(map_execution_failure(failure)),
        }
    }
}

#[async_trait]
impl ContentProcessor for OfficialAiProcessor {
    async fn process(
        &self,
        claim: &ContentJobClaim,
        remaining_attempt: Duration,
    ) -> Result<ContentProcessSuccess, ContentProcessFailure> {
        self.process_claim(claim, remaining_attempt).await
    }
}

struct OperationConfig<'a> {
    enabled: bool,
    provider_id: &'a str,
    max_output_tokens: u32,
    mcp_enabled: bool,
}

impl<'a> OperationConfig<'a> {
    fn from_config(config: &'a AiContentConfig, operation: ContentJobOperation) -> Self {
        match operation {
            ContentJobOperation::Summarize => Self {
                enabled: config.summarize_enabled(),
                provider_id: config.summarize_provider_id(),
                max_output_tokens: config.summarize_max_output_tokens(),
                mcp_enabled: config.summarize_mcp_enabled(),
            },
            ContentJobOperation::Translate => Self {
                enabled: config.translate_enabled(),
                provider_id: config.translate_provider_id(),
                max_output_tokens: config.translate_max_output_tokens(),
                mcp_enabled: config.translate_mcp_enabled(),
            },
        }
    }
}

struct OperationContract {
    prompt_version: &'static str,
    schema_id: &'static str,
}

impl OperationContract {
    const fn for_operation(operation: ContentJobOperation) -> Self {
        match operation {
            ContentJobOperation::Summarize => Self {
                prompt_version: SUMMARY_PROMPT_VERSION,
                schema_id: SUMMARY_SCHEMA_ID,
            },
            ContentJobOperation::Translate => Self {
                prompt_version: TRANSLATION_PROMPT_VERSION,
                schema_id: TRANSLATION_SCHEMA_ID,
            },
        }
    }
}

fn invocation_deadlines(
    claim: &ContentJobClaim,
    remaining_attempt: Duration,
) -> Result<(u64, Instant), ContentProcessFailure> {
    if remaining_attempt.is_zero()
        || remaining_attempt > Duration::from_secs(u64::from(claim.trigger().timeout_seconds()))
    {
        return Err(runtime_unavailable(AttemptUsage::empty()));
    }
    let wall_now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| runtime_unavailable(AttemptUsage::empty()))?;
    let wall_deadline = wall_now
        .checked_add(remaining_attempt)
        .and_then(|value| u64::try_from(value.as_millis()).ok())
        .ok_or_else(|| runtime_unavailable(AttemptUsage::empty()))?;
    let deadline = Instant::now()
        .checked_add(remaining_attempt)
        .ok_or_else(|| runtime_unavailable(AttemptUsage::empty()))?;
    Ok((wall_deadline, deadline))
}

fn build_success(
    claim: &ContentJobClaim,
    execution: crate::plugins::runtime::PluginExecutionSuccess,
) -> Result<ContentProcessSuccess, ContentProcessFailure> {
    let usage = build_attempt_usage(execution.usage())
        .map_err(|_| runtime_unavailable(AttemptUsage::empty()))?;
    let artifact = execution.artifact();
    if artifact.schema_id != claim.identity().schema_id()
        || artifact.locale.as_deref() != claim.identity().target_locale()
    {
        return Err(fixed_failure(
            "PLUGIN_OUTPUT_INVALID",
            false,
            false,
            None,
            usage,
        ));
    }
    let provider_label = execution
        .usage()
        .final_model_label()
        .ok_or_else(|| fixed_failure("PLUGIN_OUTPUT_INVALID", false, false, None, usage.clone()))?;
    let payload = parse_unique_json(artifact.payload_json.as_bytes(), MAX_ARTIFACT_BYTES)
        .map_err(|_| fixed_failure("PLUGIN_OUTPUT_INVALID", false, false, None, usage.clone()))?;
    let provenance = parse_unique_json(artifact.provenance_json.as_bytes(), MAX_PROVENANCE_BYTES)
        .map_err(|_| {
        fixed_failure("PLUGIN_OUTPUT_INVALID", false, false, None, usage.clone())
    })?;
    let candidate = ArtifactCandidate::new(
        claim.identity().clone(),
        provider_label.to_owned(),
        payload,
        provenance,
    )
    .map_err(|_| fixed_failure("PLUGIN_OUTPUT_INVALID", false, false, None, usage.clone()))?;
    Ok(ContentProcessSuccess::new(candidate, usage))
}

fn build_attempt_usage(
    usage: &CapabilityUsage,
) -> Result<AttemptUsage, crate::content::jobs::ContentRepositoryError> {
    AttemptUsage::new(
        usage.provider_request_count(),
        usage.mcp_call_count(),
        usage.input_tokens(),
        usage.output_tokens(),
        usage.estimated_cost_micros(),
        json!({
            "schemaVersion": 1,
            "inputTokensComplete": usage.input_tokens_complete(),
            "outputTokensComplete": usage.output_tokens_complete(),
            "estimatedCostComplete": usage.estimated_cost_complete(),
        }),
    )
}

fn map_execution_failure(failure: PluginExecutionFailure) -> ContentProcessFailure {
    let usage = build_attempt_usage(failure.usage()).unwrap_or_else(|_| AttemptUsage::empty());
    let (code, retryable, outcome_unknown) = if let Some(code) = failure.error().failure_code() {
        match code {
            PluginFailureCode::Disabled => ("PLUGIN_UNAVAILABLE", false, false),
            PluginFailureCode::ConfigInvalid => ("EXECUTION_SNAPSHOT_STALE", false, false),
            PluginFailureCode::ProviderUnavailable => ("PROVIDER_UNAVAILABLE", true, false),
            PluginFailureCode::ProviderRateLimited => ("PROVIDER_RATE_LIMITED", true, false),
            PluginFailureCode::ProviderTimeout => ("PROVIDER_TIMEOUT", true, true),
            PluginFailureCode::ProviderOutputInvalid => ("PROVIDER_OUTPUT_INVALID", false, false),
            PluginFailureCode::McpSchemaInvalid => ("MCP_SCHEMA_INVALID", false, false),
            PluginFailureCode::McpTimeout => ("MCP_TIMEOUT", true, true),
            PluginFailureCode::McpBudgetExhausted => ("MCP_BUDGET_EXHAUSTED", false, false),
            PluginFailureCode::McpRecursionBlocked => ("MCP_RECURSION_BLOCKED", false, false),
            PluginFailureCode::BudgetExhausted => ("PLUGIN_BUDGET_EXHAUSTED", false, false),
            PluginFailureCode::OutputInvalid => ("PLUGIN_OUTPUT_INVALID", false, false),
        }
    } else {
        match failure.error().kind() {
            PluginRuntimeErrorKind::InvalidComponent
            | PluginRuntimeErrorKind::ComponentDigestMismatch
            | PluginRuntimeErrorKind::LinkDenied
            | PluginRuntimeErrorKind::DescriptorMismatch
            | PluginRuntimeErrorKind::CapabilityDenied => ("PLUGIN_UNAVAILABLE", false, false),
            PluginRuntimeErrorKind::InvalidInvocation => ("PLUGIN_OUTPUT_INVALID", false, false),
            PluginRuntimeErrorKind::BrokerTimeout => ("PROVIDER_TIMEOUT", true, true),
            PluginRuntimeErrorKind::BrokerFailure => ("PROVIDER_UNAVAILABLE", true, false),
            PluginRuntimeErrorKind::FuelExhausted => ("PLUGIN_FUEL_EXHAUSTED", false, false),
            PluginRuntimeErrorKind::MemoryLimit => ("PLUGIN_MEMORY_LIMIT", false, false),
            PluginRuntimeErrorKind::GuestTimeout => ("PLUGIN_GUEST_TIMEOUT", false, false),
            PluginRuntimeErrorKind::GuestTrap => ("PLUGIN_GUEST_TRAP", false, false),
            PluginRuntimeErrorKind::OutputTooLarge => ("PLUGIN_OUTPUT_TOO_LARGE", false, false),
            PluginRuntimeErrorKind::RuntimeUnavailable => {
                ("PLUGIN_RUNTIME_UNAVAILABLE", true, false)
            }
        }
    };
    let hint = failure.failure_hint().filter(|_| allows_failure_hint(code));
    let retryable = hint.map_or(retryable, |hint| hint.retryable());
    let outcome_unknown = hint.map_or(outcome_unknown, |hint| hint.outcome_unknown());
    let retry_after = hint.and_then(retry_after);
    fixed_failure(code, retryable, outcome_unknown, retry_after, usage)
}

fn allows_failure_hint(code: &str) -> bool {
    matches!(
        code,
        "PROVIDER_UNAVAILABLE" | "PROVIDER_RATE_LIMITED" | "PROVIDER_TIMEOUT" | "MCP_TIMEOUT"
    )
}

fn retry_after(hint: CapabilityFailureHint) -> Option<Duration> {
    let retry_at = hint.retry_at_unix_ms()?;
    let now = SystemTime::now().duration_since(UNIX_EPOCH).ok()?;
    let now_ms = u64::try_from(now.as_millis()).ok()?;
    Some(Duration::from_millis(retry_at.saturating_sub(now_ms)).min(MAX_RETRY_AFTER))
}

fn wit_operation(operation: ContentJobOperation) -> types::Operation {
    match operation {
        ContentJobOperation::Summarize => types::Operation::Summarize,
        ContentJobOperation::Translate => types::Operation::Translate,
    }
}

fn wit_trigger(trigger: ContentJobTrigger) -> types::Trigger {
    match trigger {
        ContentJobTrigger::ManualApi => types::Trigger::ManualApi,
        ContentJobTrigger::ReaderSidecar => types::Trigger::ReaderSidecar,
        ContentJobTrigger::FeedRefreshPersisted => types::Trigger::FeedRefreshPersisted,
        ContentJobTrigger::McpServer => types::Trigger::McpServer,
    }
}

fn map_entry_error(kind: ContentRepositoryErrorKind) -> ContentProcessFailure {
    if kind == ContentRepositoryErrorKind::Database {
        fixed_failure(
            "CONTENT_REPOSITORY_UNAVAILABLE",
            true,
            false,
            None,
            AttemptUsage::empty(),
        )
    } else {
        snapshot_stale()
    }
}

fn map_plugin_repository_error(kind: PluginRegistryErrorKind) -> ContentProcessFailure {
    if kind == PluginRegistryErrorKind::Database {
        runtime_unavailable(AttemptUsage::empty())
    } else {
        plugin_unavailable()
    }
}

fn map_provider_error(kind: ProviderCoreErrorKind) -> ContentProcessFailure {
    match kind {
        ProviderCoreErrorKind::Database => fixed_failure(
            "PROVIDER_UNAVAILABLE",
            true,
            false,
            None,
            AttemptUsage::empty(),
        ),
        ProviderCoreErrorKind::SecretUnavailable => fixed_failure(
            "PROVIDER_UNAVAILABLE",
            false,
            false,
            None,
            AttemptUsage::empty(),
        ),
        ProviderCoreErrorKind::InvalidProviderId
        | ProviderCoreErrorKind::InvalidUserId
        | ProviderCoreErrorKind::InvalidDisplayName
        | ProviderCoreErrorKind::InvalidEndpoint
        | ProviderCoreErrorKind::InvalidModel
        | ProviderCoreErrorKind::InvalidCredential
        | ProviderCoreErrorKind::UnsupportedCapability
        | ProviderCoreErrorKind::InvalidPolicy
        | ProviderCoreErrorKind::InvalidPatch
        | ProviderCoreErrorKind::NotFound
        | ProviderCoreErrorKind::ProviderDisabled
        | ProviderCoreErrorKind::RevisionConflict
        | ProviderCoreErrorKind::CorruptData => provider_binding_stale(),
    }
}

fn snapshot_stale() -> ContentProcessFailure {
    fixed_failure(
        "EXECUTION_SNAPSHOT_STALE",
        false,
        false,
        None,
        AttemptUsage::empty(),
    )
}

fn plugin_unavailable() -> ContentProcessFailure {
    fixed_failure(
        "PLUGIN_UNAVAILABLE",
        false,
        false,
        None,
        AttemptUsage::empty(),
    )
}

fn provider_binding_stale() -> ContentProcessFailure {
    fixed_failure(
        "PROVIDER_BINDING_STALE",
        false,
        false,
        None,
        AttemptUsage::empty(),
    )
}

fn runtime_unavailable(usage: AttemptUsage) -> ContentProcessFailure {
    fixed_failure("PLUGIN_RUNTIME_UNAVAILABLE", true, false, None, usage)
}

fn fixed_failure(
    code: &'static str,
    retryable: bool,
    outcome_unknown: bool,
    retry_after: Option<Duration>,
    usage: AttemptUsage,
) -> ContentProcessFailure {
    ContentProcessFailure::fixed(code, retryable, outcome_unknown, retry_after, usage)
}
