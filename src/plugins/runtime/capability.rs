use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use async_trait::async_trait;
use tokio::time::{Instant, timeout};

use crate::plugins::json::{
    canonical_json, contextual_hash, parse_unique_json, validate_text, validate_uuid,
    validate_visible_ascii,
};

use super::{
    PluginRuntimeError, PluginRuntimeErrorKind,
    bindings::{host_ai, host_mcp, types},
};

const MAX_AI_CALLS: u32 = 3;
const MAX_AI_INPUT_BYTES: usize = 512 * 1024;
const MAX_AI_SCHEMA_BYTES: usize = 64 * 1024;
const MAX_AI_SYSTEM_INSTRUCTION_BYTES: usize = 64 * 1024;
const MAX_AI_OUTPUT_BYTES: usize = 512 * 1024;
const MAX_AI_TIMEOUT: Duration = Duration::from_secs(90);
const MAX_MODEL_LABEL_BYTES: usize = 200;
const MAX_BINDING_ID_BYTES: usize = 128;
const MAX_SCHEMA_ID_BYTES: usize = 256;
const MAX_REMAINING_DEPTH: u32 = 2;
const MAX_COST_MICROS: u64 = 250_000;
const MAX_MCP_ARGUMENT_BYTES: usize = 64 * 1024;
const MAX_MCP_RESULT_BYTES: usize = 256 * 1024;
const MAX_MCP_TIMEOUT: Duration = Duration::from_secs(15);
const MAX_MCP_LABEL_BYTES: usize = 128;
const MAX_TOOL_BINDINGS: usize = 16;
const MAX_TOOL_DESCRIPTION_BYTES: usize = 8 * 1024;
const MAX_TOOL_SCHEMA_BYTES: usize = 64 * 1024;
const MAX_DEADLINE_SKEW: Duration = Duration::from_secs(2);
const TOOL_SCHEMA_HASH_CONTEXT: &str = "raindrop.mcp-tool-input-schema.v1";

#[derive(Clone)]
pub struct BrokerInvocationContext {
    pub invocation_id: String,
    pub job_id: String,
    pub user_subject: String,
    pub call_chain_id: String,
    pub operation: types::Operation,
    pub trigger: types::Trigger,
    pub remaining_depth: u32,
}

impl fmt::Debug for BrokerInvocationContext {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("BrokerInvocationContext")
            .field("operation", &self.operation)
            .field("trigger", &self.trigger)
            .field("remaining_depth", &self.remaining_depth)
            .finish_non_exhaustive()
    }
}

pub struct CapabilitySessionConfig {
    pub invocation: BrokerInvocationContext,
    pub provider_binding_id: String,
    pub tool_bindings: Vec<CapabilityToolBinding>,
    pub remaining_provider_requests: u32,
    pub remaining_mcp_calls: u32,
    pub remaining_input_tokens: u32,
    pub remaining_output_tokens: u32,
    pub remaining_cost_micros: u64,
    pub deadline_unix_ms: u64,
    pub deadline: Instant,
}

pub struct CapabilityToolBindingInput {
    pub binding_id: String,
    pub connection_id: String,
    pub tool_name: String,
    pub display_label: String,
    pub description: String,
    pub input_schema_json: String,
    pub input_schema_digest: String,
}

#[derive(Clone, Eq, PartialEq)]
pub struct CapabilityToolBinding {
    binding_id: String,
    connection_id: String,
    tool_name: String,
    display_label: String,
    description: String,
    input_schema_json: String,
    input_schema_digest: String,
}

impl CapabilityToolBinding {
    pub fn new(input: CapabilityToolBindingInput) -> Result<Self, PluginRuntimeError> {
        let schema = canonical_object_json(&input.input_schema_json, MAX_TOOL_SCHEMA_BYTES)
            .ok_or_else(invalid_invocation)?;
        let valid = validate_visible_ascii(
            &input.binding_id,
            MAX_BINDING_ID_BYTES,
            crate::plugins::PluginRegistryErrorKind::InvalidInput,
        )
        .is_ok()
            && validate_uuid(
                &input.connection_id,
                crate::plugins::PluginRegistryErrorKind::InvalidInput,
            )
            .is_ok()
            && valid_tool_name(&input.tool_name)
            && valid_public_label(&input.display_label, MAX_MCP_LABEL_BYTES)
            && valid_tool_description(&input.description)
            && input.input_schema_digest
                == contextual_hash(TOOL_SCHEMA_HASH_CONTEXT, schema.as_bytes());
        if !valid {
            return Err(invalid_invocation());
        }
        Ok(Self {
            binding_id: input.binding_id,
            connection_id: input.connection_id,
            tool_name: input.tool_name,
            display_label: input.display_label,
            description: input.description,
            input_schema_json: schema,
            input_schema_digest: input.input_schema_digest,
        })
    }

    #[must_use]
    pub fn binding_id(&self) -> &str {
        &self.binding_id
    }

    #[must_use]
    pub fn to_wit(&self) -> types::ToolBinding {
        types::ToolBinding {
            binding_id: self.binding_id.clone(),
            connection_id: self.connection_id.clone(),
            tool_name: self.tool_name.clone(),
            display_label: self.display_label.clone(),
            description: self.description.clone(),
            input_schema_json: self.input_schema_json.clone(),
            input_schema_digest: self.input_schema_digest.clone(),
        }
    }

    fn matches_wit(&self, binding: &types::ToolBinding) -> bool {
        binding.binding_id == self.binding_id
            && binding.connection_id == self.connection_id
            && binding.tool_name == self.tool_name
            && binding.display_label == self.display_label
            && binding.description == self.description
            && binding.input_schema_json == self.input_schema_json
            && binding.input_schema_digest == self.input_schema_digest
    }
}

impl fmt::Debug for CapabilityToolBinding {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CapabilityToolBinding")
            .field("description_bytes", &self.description.len())
            .field("input_schema_bytes", &self.input_schema_json.len())
            .finish_non_exhaustive()
    }
}

impl fmt::Debug for CapabilitySessionConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CapabilitySessionConfig")
            .field("invocation", &self.invocation)
            .field(
                "remaining_provider_requests",
                &self.remaining_provider_requests,
            )
            .field("remaining_mcp_calls", &self.remaining_mcp_calls)
            .field("remaining_input_tokens", &self.remaining_input_tokens)
            .field("remaining_output_tokens", &self.remaining_output_tokens)
            .field("remaining_cost_micros", &self.remaining_cost_micros)
            .field("tool_binding_count", &self.tool_bindings.len())
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AiFinishReason {
    Completed,
    Length,
    ContentFilter,
    ToolPlan,
    Unknown,
}

#[derive(Clone)]
pub struct AiBrokerRequest {
    pub provider_binding_id: String,
    pub operation: types::Operation,
    pub system_instruction: String,
    pub untrusted_input_json: String,
    pub output_schema_id: String,
    pub output_schema_json: String,
    pub provider_request_ordinal: u32,
    pub max_input_tokens: u32,
    pub max_output_tokens: u32,
    pub max_cost_micros: u64,
    pub timeout: Duration,
}

impl fmt::Debug for AiBrokerRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AiBrokerRequest")
            .field("operation", &self.operation)
            .field("provider_request_ordinal", &self.provider_request_ordinal)
            .field("max_input_tokens", &self.max_input_tokens)
            .field("max_output_tokens", &self.max_output_tokens)
            .field("max_cost_micros", &self.max_cost_micros)
            .field("timeout", &self.timeout)
            .finish_non_exhaustive()
    }
}

#[derive(Clone)]
pub struct AiBrokerResponse {
    pub output_json: String,
    pub finish_reason: AiFinishReason,
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub model_label: String,
    pub estimated_cost_micros: Option<u64>,
}

impl fmt::Debug for AiBrokerResponse {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AiBrokerResponse")
            .field("finish_reason", &self.finish_reason)
            .field("input_tokens", &self.input_tokens)
            .field("output_tokens", &self.output_tokens)
            .field("estimated_cost_micros", &self.estimated_cost_micros)
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AiBrokerErrorKind {
    CapabilityDenied,
    ProviderUnavailable,
    QuotaExceeded,
    RateLimited,
    Timeout,
    OutputSchemaInvalid,
    CostLimitExceeded,
    InvalidRequest,
    Failure,
}

#[derive(Clone, Copy, Eq, PartialEq)]
pub struct AiBrokerError {
    kind: AiBrokerErrorKind,
    retryable: bool,
    retry_at_unix_ms: Option<u64>,
}

impl AiBrokerError {
    #[must_use]
    pub const fn new(
        kind: AiBrokerErrorKind,
        retryable: bool,
        retry_at_unix_ms: Option<u64>,
    ) -> Self {
        Self {
            kind,
            retryable,
            retry_at_unix_ms,
        }
    }

    #[must_use]
    pub const fn kind(&self) -> AiBrokerErrorKind {
        self.kind
    }

    #[must_use]
    pub const fn retryable(&self) -> bool {
        self.retryable
    }

    #[must_use]
    pub const fn retry_at_unix_ms(&self) -> Option<u64> {
        self.retry_at_unix_ms
    }
}

impl fmt::Debug for AiBrokerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AiBrokerError")
            .field("kind", &self.kind)
            .field("retryable", &self.retryable)
            .field("retry_at_unix_ms", &self.retry_at_unix_ms)
            .finish()
    }
}

#[async_trait]
pub trait AiCapabilityBroker: Send + Sync {
    async fn generate_structured(
        &self,
        context: &BrokerInvocationContext,
        request: AiBrokerRequest,
    ) -> Result<AiBrokerResponse, AiBrokerError>;
}

#[derive(Clone)]
pub struct McpBrokerRequest {
    pub tool_binding_id: String,
    pub arguments_json: String,
    pub timeout: Duration,
    pub remaining_depth: u32,
}

impl fmt::Debug for McpBrokerRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("McpBrokerRequest")
            .field("timeout", &self.timeout)
            .field("remaining_depth", &self.remaining_depth)
            .finish_non_exhaustive()
    }
}

#[derive(Clone)]
pub struct McpBrokerResponse {
    pub result_json: String,
    pub connection_label: String,
    pub tool_label: String,
}

impl fmt::Debug for McpBrokerResponse {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("McpBrokerResponse")
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum McpBrokerErrorKind {
    Disabled,
    CapabilityDenied,
    ConnectionDenied,
    ToolDenied,
    SideEffectConfirmationRequired,
    SchemaInvalid,
    Timeout,
    ResultTooLarge,
    BudgetExhausted,
    RecursionBlocked,
    Failure,
}

#[derive(Clone, Copy, Eq, PartialEq)]
pub struct McpBrokerError {
    kind: McpBrokerErrorKind,
    retryable: bool,
}

impl McpBrokerError {
    #[must_use]
    pub const fn new(kind: McpBrokerErrorKind, retryable: bool) -> Self {
        Self { kind, retryable }
    }

    #[must_use]
    pub const fn kind(&self) -> McpBrokerErrorKind {
        self.kind
    }
}

impl fmt::Debug for McpBrokerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("McpBrokerError")
            .field("kind", &self.kind)
            .field("retryable", &self.retryable)
            .finish()
    }
}

#[async_trait]
pub trait McpCapabilityBroker: Send + Sync {
    async fn call_tool(
        &self,
        context: &BrokerInvocationContext,
        request: McpBrokerRequest,
    ) -> Result<McpBrokerResponse, McpBrokerError>;
}

pub struct DenyAiBroker;

#[async_trait]
impl AiCapabilityBroker for DenyAiBroker {
    async fn generate_structured(
        &self,
        _context: &BrokerInvocationContext,
        _request: AiBrokerRequest,
    ) -> Result<AiBrokerResponse, AiBrokerError> {
        Err(AiBrokerError::new(
            AiBrokerErrorKind::CapabilityDenied,
            false,
            None,
        ))
    }
}

pub struct DenyMcpBroker;

#[async_trait]
impl McpCapabilityBroker for DenyMcpBroker {
    async fn call_tool(
        &self,
        _context: &BrokerInvocationContext,
        _request: McpBrokerRequest,
    ) -> Result<McpBrokerResponse, McpBrokerError> {
        Err(McpBrokerError::new(McpBrokerErrorKind::Disabled, false))
    }
}

pub struct CapabilitySession {
    invocation: BrokerInvocationContext,
    provider_binding_id: String,
    tool_bindings: BTreeMap<String, CapabilityToolBinding>,
    remaining_provider_requests: u32,
    remaining_mcp_calls: u32,
    remaining_input_tokens: u32,
    remaining_output_tokens: u32,
    remaining_cost_micros: u64,
    next_provider_ordinal: u32,
    deadline_unix_ms: u64,
    deadline: Instant,
    enabled: bool,
    ai_broker: Arc<dyn AiCapabilityBroker>,
    mcp_broker: Arc<dyn McpCapabilityBroker>,
}

impl CapabilitySession {
    pub fn new(
        config: CapabilitySessionConfig,
        ai_broker: Arc<dyn AiCapabilityBroker>,
        mcp_broker: Arc<dyn McpCapabilityBroker>,
    ) -> Result<Self, PluginRuntimeError> {
        validate_session_config(&config)?;
        Ok(Self {
            invocation: config.invocation,
            provider_binding_id: config.provider_binding_id,
            tool_bindings: config
                .tool_bindings
                .into_iter()
                .map(|binding| (binding.binding_id.clone(), binding))
                .collect(),
            remaining_provider_requests: config.remaining_provider_requests,
            remaining_mcp_calls: config.remaining_mcp_calls,
            remaining_input_tokens: config.remaining_input_tokens,
            remaining_output_tokens: config.remaining_output_tokens,
            remaining_cost_micros: config.remaining_cost_micros,
            next_provider_ordinal: 1,
            deadline_unix_ms: config.deadline_unix_ms,
            deadline: config.deadline,
            enabled: true,
            ai_broker,
            mcp_broker,
        })
    }

    pub async fn generate_structured(
        &mut self,
        request: host_ai::GenerateRequest,
    ) -> Result<Result<host_ai::GenerateResponse, host_ai::GenerateError>, PluginRuntimeError> {
        let request = match self.validate_ai_request(request) {
            Ok(request) => request,
            Err(error) => return Ok(Err(error)),
        };
        self.remaining_provider_requests -= 1;
        self.remaining_input_tokens -= request.max_input_tokens;
        self.remaining_output_tokens -= request.max_output_tokens;
        self.next_provider_ordinal += 1;

        let broker_result = timeout(
            request.timeout,
            self.ai_broker
                .generate_structured(&self.invocation, request.clone()),
        )
        .await;
        let response = match broker_result {
            Err(_) => return Ok(Err(ai_error(host_ai::GenerateErrorCode::Timeout))),
            Ok(Err(error)) => return map_ai_broker_error(error),
            Ok(Ok(response)) => response,
        };
        let response = match self.validate_ai_response(&request, response) {
            Ok(response) => response,
            Err(error) => return Ok(Err(error)),
        };
        Ok(Ok(response))
    }

    pub async fn call_tool(
        &mut self,
        request: host_mcp::CallRequest,
    ) -> Result<Result<host_mcp::CallResponse, host_mcp::CallError>, PluginRuntimeError> {
        let request = match self.validate_mcp_request(request) {
            Ok(request) => request,
            Err(error) => return Ok(Err(error)),
        };
        self.remaining_mcp_calls -= 1;

        let broker_result = timeout(
            request.timeout,
            self.mcp_broker.call_tool(&self.invocation, request.clone()),
        )
        .await;
        let response = match broker_result {
            Err(_) => return Ok(Err(mcp_error(host_mcp::CallErrorCode::Timeout))),
            Ok(Err(error)) => return map_mcp_broker_error(error),
            Ok(Ok(response)) => response,
        };
        let response = match validate_mcp_response(response) {
            Ok(response) => response,
            Err(error) => return Ok(Err(error)),
        };
        Ok(Ok(response))
    }

    fn validate_ai_request(
        &self,
        request: host_ai::GenerateRequest,
    ) -> Result<AiBrokerRequest, host_ai::GenerateError> {
        if !self.enabled
            || request.provider_binding_id != self.provider_binding_id
            || request.operation != self.invocation.operation
        {
            return Err(ai_error(host_ai::GenerateErrorCode::CapabilityDenied));
        }
        if self.remaining_provider_requests == 0 {
            return Err(ai_error(host_ai::GenerateErrorCode::QuotaExceeded));
        }
        if request.provider_request_ordinal != self.next_provider_ordinal {
            return Err(ai_error(host_ai::GenerateErrorCode::InvalidRequest));
        }
        if validate_text(
            &request.system_instruction,
            MAX_AI_SYSTEM_INSTRUCTION_BYTES,
            crate::plugins::PluginRegistryErrorKind::InvalidInput,
        )
        .is_err()
            || validate_visible_ascii(
                &request.output_schema_id,
                MAX_SCHEMA_ID_BYTES,
                crate::plugins::PluginRegistryErrorKind::InvalidInput,
            )
            .is_err()
        {
            return Err(ai_error(host_ai::GenerateErrorCode::InvalidRequest));
        }
        let untrusted_input_json =
            canonical_object_json(&request.untrusted_input_json, MAX_AI_INPUT_BYTES)
                .ok_or_else(|| ai_error(host_ai::GenerateErrorCode::InvalidRequest))?;
        let output_schema_json =
            canonical_object_json(&request.output_schema_json, MAX_AI_SCHEMA_BYTES)
                .ok_or_else(|| ai_error(host_ai::GenerateErrorCode::InvalidRequest))?;
        let operation_output_limit = max_output_tokens(self.invocation.operation);
        if request.max_input_tokens == 0
            || request.max_input_tokens > self.remaining_input_tokens
            || request.max_output_tokens == 0
            || request.max_output_tokens > self.remaining_output_tokens
            || request.max_output_tokens > operation_output_limit
        {
            return Err(ai_error(host_ai::GenerateErrorCode::QuotaExceeded));
        }
        let remaining = self.deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(ai_error(host_ai::GenerateErrorCode::Timeout));
        }

        Ok(AiBrokerRequest {
            provider_binding_id: request.provider_binding_id,
            operation: request.operation,
            system_instruction: request.system_instruction,
            untrusted_input_json,
            output_schema_id: request.output_schema_id,
            output_schema_json,
            provider_request_ordinal: request.provider_request_ordinal,
            max_input_tokens: request.max_input_tokens,
            max_output_tokens: request.max_output_tokens,
            max_cost_micros: self.remaining_cost_micros,
            timeout: remaining.min(MAX_AI_TIMEOUT),
        })
    }

    fn validate_ai_response(
        &mut self,
        request: &AiBrokerRequest,
        response: AiBrokerResponse,
    ) -> Result<host_ai::GenerateResponse, host_ai::GenerateError> {
        let output_json = canonical_object_json(&response.output_json, MAX_AI_OUTPUT_BYTES)
            .ok_or_else(|| ai_error(host_ai::GenerateErrorCode::OutputSchemaInvalid))?;
        if !valid_public_label(&response.model_label, MAX_MODEL_LABEL_BYTES)
            || response
                .input_tokens
                .is_some_and(|value| value > u64::from(request.max_input_tokens))
            || response
                .output_tokens
                .is_some_and(|value| value > u64::from(request.max_output_tokens))
        {
            return Err(ai_error(host_ai::GenerateErrorCode::OutputSchemaInvalid));
        }
        if response
            .estimated_cost_micros
            .is_some_and(|value| value > self.remaining_cost_micros)
        {
            return Err(ai_error(host_ai::GenerateErrorCode::CostLimitExceeded));
        }
        if let Some(cost) = response.estimated_cost_micros {
            self.remaining_cost_micros -= cost;
        }
        Ok(host_ai::GenerateResponse {
            output_json,
            finish_reason: match response.finish_reason {
                AiFinishReason::Completed => host_ai::FinishReason::Completed,
                AiFinishReason::Length => host_ai::FinishReason::Length,
                AiFinishReason::ContentFilter => host_ai::FinishReason::ContentFilter,
                AiFinishReason::ToolPlan => host_ai::FinishReason::ToolPlan,
                AiFinishReason::Unknown => host_ai::FinishReason::Unknown,
            },
            usage: host_ai::TokenUsage {
                input_tokens: response.input_tokens,
                output_tokens: response.output_tokens,
            },
            model_label: response.model_label,
            estimated_cost_micros: response.estimated_cost_micros,
        })
    }

    fn validate_mcp_request(
        &self,
        request: host_mcp::CallRequest,
    ) -> Result<McpBrokerRequest, host_mcp::CallError> {
        if !self.enabled {
            return Err(mcp_error(host_mcp::CallErrorCode::CapabilityDenied));
        }
        if !self.tool_bindings.contains_key(&request.tool_binding_id) {
            return Err(mcp_error(host_mcp::CallErrorCode::ToolDenied));
        }
        if self.invocation.remaining_depth == 0 {
            return Err(mcp_error(host_mcp::CallErrorCode::RecursionBlocked));
        }
        if self.remaining_mcp_calls == 0 {
            return Err(mcp_error(host_mcp::CallErrorCode::BudgetExhausted));
        }
        if request.requested_timeout_ms == 0
            || request.requested_timeout_ms > MAX_MCP_TIMEOUT.as_millis() as u32
        {
            return Err(mcp_error(host_mcp::CallErrorCode::Timeout));
        }
        let arguments_json = canonical_object_json(&request.arguments_json, MAX_MCP_ARGUMENT_BYTES)
            .ok_or_else(|| mcp_error(host_mcp::CallErrorCode::SchemaInvalid))?;
        let remaining = self.deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(mcp_error(host_mcp::CallErrorCode::Timeout));
        }
        let requested_timeout = Duration::from_millis(u64::from(request.requested_timeout_ms));
        Ok(McpBrokerRequest {
            tool_binding_id: request.tool_binding_id,
            arguments_json,
            timeout: requested_timeout.min(remaining).min(MAX_MCP_TIMEOUT),
            remaining_depth: self.invocation.remaining_depth - 1,
        })
    }

    pub(crate) fn validate_operation_request(
        &self,
        request: &types::OperationRequest,
    ) -> Result<(), PluginRuntimeError> {
        let tool_binding_ids = request
            .tool_bindings
            .iter()
            .map(|binding| binding.binding_id.as_str())
            .collect::<BTreeSet<_>>();
        let valid = request.invocation_id == self.invocation.invocation_id
            && request.job_id == self.invocation.job_id
            && request.user_scope.subject == self.invocation.user_subject
            && request.call_chain_id == self.invocation.call_chain_id
            && request.operation == self.invocation.operation
            && request.trigger == self.invocation.trigger
            && request.provider_binding_id == self.provider_binding_id
            && tool_binding_ids.len() == request.tool_bindings.len()
            && tool_binding_ids
                == self
                    .tool_bindings
                    .keys()
                    .map(String::as_str)
                    .collect::<BTreeSet<_>>()
            && request.tool_bindings.iter().all(|binding| {
                self.tool_bindings
                    .get(&binding.binding_id)
                    .is_some_and(|expected| expected.matches_wit(binding))
            })
            && request.budget.remaining_depth == self.invocation.remaining_depth
            && request.budget.deadline_unix_ms == self.deadline_unix_ms
            && request.budget.remaining_provider_requests == self.remaining_provider_requests
            && request.budget.remaining_mcp_calls == self.remaining_mcp_calls
            && request.budget.remaining_input_tokens == self.remaining_input_tokens
            && request.budget.remaining_output_tokens == self.remaining_output_tokens
            && request.budget.remaining_cost_micros == self.remaining_cost_micros;
        if valid {
            Ok(())
        } else {
            Err(PluginRuntimeError::new(
                PluginRuntimeErrorKind::InvalidInvocation,
            ))
        }
    }

    pub(crate) fn validate_lifecycle_context(
        &self,
        invocation_id: &str,
        subject: &str,
    ) -> Result<(), PluginRuntimeError> {
        if invocation_id == self.invocation.invocation_id
            && subject == self.invocation.user_subject
            && self.invocation.trigger == types::Trigger::FeedRefreshPersisted
        {
            Ok(())
        } else {
            Err(PluginRuntimeError::new(
                PluginRuntimeErrorKind::InvalidInvocation,
            ))
        }
    }

    pub(crate) fn suspend(&mut self) {
        self.enabled = false;
    }

    pub(crate) fn activate(&mut self) {
        self.enabled = true;
    }
}

fn validate_session_config(config: &CapabilitySessionConfig) -> Result<(), PluginRuntimeError> {
    let monotonic_remaining = config.deadline.saturating_duration_since(Instant::now());
    let unix_now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|duration| u64::try_from(duration.as_millis()).ok());
    let wall_remaining = unix_now_ms
        .and_then(|now| config.deadline_unix_ms.checked_sub(now))
        .map(Duration::from_millis);
    let maximum_deadline = match config.invocation.trigger {
        types::Trigger::FeedRefreshPersisted => Duration::from_secs(120),
        types::Trigger::ManualApi | types::Trigger::ReaderSidecar | types::Trigger::McpServer => {
            Duration::from_secs(180)
        }
    };
    let deadlines_valid = wall_remaining.is_some_and(|wall_remaining| {
        !wall_remaining.is_zero()
            && !monotonic_remaining.is_zero()
            && wall_remaining <= maximum_deadline
            && monotonic_remaining <= maximum_deadline
            && wall_remaining.abs_diff(monotonic_remaining) <= MAX_DEADLINE_SKEW
    });
    let invalid = validate_visible_ascii(
        &config.invocation.invocation_id,
        MAX_BINDING_ID_BYTES,
        crate::plugins::PluginRegistryErrorKind::InvalidInput,
    )
    .is_err()
        || validate_visible_ascii(
            &config.invocation.job_id,
            MAX_BINDING_ID_BYTES,
            crate::plugins::PluginRegistryErrorKind::InvalidInput,
        )
        .is_err()
        || validate_visible_ascii(
            &config.invocation.user_subject,
            MAX_BINDING_ID_BYTES,
            crate::plugins::PluginRegistryErrorKind::InvalidInput,
        )
        .is_err()
        || validate_visible_ascii(
            &config.invocation.call_chain_id,
            MAX_BINDING_ID_BYTES,
            crate::plugins::PluginRegistryErrorKind::InvalidInput,
        )
        .is_err()
        || validate_visible_ascii(
            &config.provider_binding_id,
            MAX_BINDING_ID_BYTES,
            crate::plugins::PluginRegistryErrorKind::InvalidInput,
        )
        .is_err()
        || config.invocation.remaining_depth > MAX_REMAINING_DEPTH
        || config.remaining_provider_requests > MAX_AI_CALLS
        || config.remaining_mcp_calls > max_mcp_calls(config.invocation.trigger)
        || config.remaining_output_tokens > max_output_tokens(config.invocation.operation)
        || config.remaining_cost_micros > MAX_COST_MICROS
        || config.tool_bindings.len() > MAX_TOOL_BINDINGS
        || !deadlines_valid;
    if invalid {
        return Err(PluginRuntimeError::new(
            PluginRuntimeErrorKind::InvalidInvocation,
        ));
    }
    let mut unique_ids = BTreeSet::new();
    let mut unique_tools = BTreeSet::new();
    for binding in &config.tool_bindings {
        if !unique_ids.insert(binding.binding_id.as_str())
            || !unique_tools.insert((binding.connection_id.as_str(), binding.tool_name.as_str()))
        {
            return Err(PluginRuntimeError::new(
                PluginRuntimeErrorKind::InvalidInvocation,
            ));
        }
    }
    Ok(())
}

fn valid_tool_name(value: &str) -> bool {
    let Some(first) = value.as_bytes().first() else {
        return false;
    };
    value.len() <= 128
        && first.is_ascii_alphanumeric()
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b':' | b'/' | b'-')
        })
}

fn valid_tool_description(value: &str) -> bool {
    value.len() <= MAX_TOOL_DESCRIPTION_BYTES
        && !value
            .chars()
            .any(|character| character.is_control() && !matches!(character, '\n' | '\r' | '\t'))
}

fn invalid_invocation() -> PluginRuntimeError {
    PluginRuntimeError::new(PluginRuntimeErrorKind::InvalidInvocation)
}

fn canonical_object_json(input: &str, max_bytes: usize) -> Option<String> {
    let value = parse_unique_json(input.as_bytes(), max_bytes).ok()?;
    if !value.is_object() {
        return None;
    }
    let encoded = canonical_json(value, max_bytes).ok()?;
    (encoded == input).then_some(encoded)
}

fn max_output_tokens(operation: types::Operation) -> u32 {
    match operation {
        types::Operation::Summarize => 4_096,
        types::Operation::Translate => 16_384,
    }
}

fn max_mcp_calls(trigger: types::Trigger) -> u32 {
    match trigger {
        types::Trigger::FeedRefreshPersisted => 2,
        types::Trigger::ManualApi | types::Trigger::ReaderSidecar | types::Trigger::McpServer => 4,
    }
}

fn ai_error(code: host_ai::GenerateErrorCode) -> host_ai::GenerateError {
    host_ai::GenerateError {
        code,
        retryable: false,
        retry_at_unix_ms: None,
    }
}

fn mcp_error(code: host_mcp::CallErrorCode) -> host_mcp::CallError {
    host_mcp::CallError {
        code,
        retryable: false,
    }
}

fn map_ai_broker_error(
    error: AiBrokerError,
) -> Result<Result<host_ai::GenerateResponse, host_ai::GenerateError>, PluginRuntimeError> {
    if error.kind == AiBrokerErrorKind::Failure {
        return Err(PluginRuntimeError::new(
            PluginRuntimeErrorKind::BrokerFailure,
        ));
    }
    let code = match error.kind {
        AiBrokerErrorKind::CapabilityDenied => host_ai::GenerateErrorCode::CapabilityDenied,
        AiBrokerErrorKind::ProviderUnavailable => host_ai::GenerateErrorCode::ProviderUnavailable,
        AiBrokerErrorKind::QuotaExceeded => host_ai::GenerateErrorCode::QuotaExceeded,
        AiBrokerErrorKind::RateLimited => host_ai::GenerateErrorCode::RateLimited,
        AiBrokerErrorKind::Timeout => host_ai::GenerateErrorCode::Timeout,
        AiBrokerErrorKind::OutputSchemaInvalid => host_ai::GenerateErrorCode::OutputSchemaInvalid,
        AiBrokerErrorKind::CostLimitExceeded => host_ai::GenerateErrorCode::CostLimitExceeded,
        AiBrokerErrorKind::InvalidRequest => host_ai::GenerateErrorCode::InvalidRequest,
        AiBrokerErrorKind::Failure => unreachable!(),
    };
    Ok(Err(host_ai::GenerateError {
        code,
        retryable: error.retryable,
        retry_at_unix_ms: error.retry_at_unix_ms,
    }))
}

fn canonical_json_value(input: &str, max_bytes: usize) -> Option<String> {
    let value = parse_unique_json(input.as_bytes(), max_bytes).ok()?;
    let encoded = canonical_json(value, max_bytes).ok()?;
    (encoded == input).then_some(encoded)
}

fn validate_mcp_response(
    response: McpBrokerResponse,
) -> Result<host_mcp::CallResponse, host_mcp::CallError> {
    if response.result_json.len() > MAX_MCP_RESULT_BYTES {
        return Err(mcp_error(host_mcp::CallErrorCode::ResultTooLarge));
    }
    let result_json = canonical_json_value(&response.result_json, MAX_MCP_RESULT_BYTES)
        .ok_or_else(|| mcp_error(host_mcp::CallErrorCode::SchemaInvalid))?;
    if !valid_public_label(&response.connection_label, MAX_MCP_LABEL_BYTES)
        || !valid_public_label(&response.tool_label, MAX_MCP_LABEL_BYTES)
    {
        return Err(mcp_error(host_mcp::CallErrorCode::SchemaInvalid));
    }
    Ok(host_mcp::CallResponse {
        result_json,
        connection_label: response.connection_label,
        tool_label: response.tool_label,
    })
}

fn valid_public_label(value: &str, max_bytes: usize) -> bool {
    !value.trim().is_empty() && value.len() <= max_bytes && !value.chars().any(char::is_control)
}

fn map_mcp_broker_error(
    error: McpBrokerError,
) -> Result<Result<host_mcp::CallResponse, host_mcp::CallError>, PluginRuntimeError> {
    if error.kind == McpBrokerErrorKind::Failure {
        return Err(PluginRuntimeError::new(
            PluginRuntimeErrorKind::BrokerFailure,
        ));
    }
    let code = match error.kind {
        McpBrokerErrorKind::Disabled => host_mcp::CallErrorCode::Disabled,
        McpBrokerErrorKind::CapabilityDenied => host_mcp::CallErrorCode::CapabilityDenied,
        McpBrokerErrorKind::ConnectionDenied => host_mcp::CallErrorCode::ConnectionDenied,
        McpBrokerErrorKind::ToolDenied => host_mcp::CallErrorCode::ToolDenied,
        McpBrokerErrorKind::SideEffectConfirmationRequired => {
            host_mcp::CallErrorCode::SideEffectConfirmationRequired
        }
        McpBrokerErrorKind::SchemaInvalid => host_mcp::CallErrorCode::SchemaInvalid,
        McpBrokerErrorKind::Timeout => host_mcp::CallErrorCode::Timeout,
        McpBrokerErrorKind::ResultTooLarge => host_mcp::CallErrorCode::ResultTooLarge,
        McpBrokerErrorKind::BudgetExhausted => host_mcp::CallErrorCode::BudgetExhausted,
        McpBrokerErrorKind::RecursionBlocked => host_mcp::CallErrorCode::RecursionBlocked,
        McpBrokerErrorKind::Failure => unreachable!(),
    };
    Ok(Err(host_mcp::CallError {
        code,
        retryable: error.retryable,
    }))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;

    #[test]
    fn suspended_session_denies_host_capabilities_until_descriptor_is_accepted() {
        let duration = Duration::from_secs(30);
        let unix_now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should follow Unix epoch");
        let deadline_unix_ms =
            u64::try_from((unix_now + duration).as_millis()).expect("deadline should fit u64");
        let mut session = CapabilitySession::new(
            CapabilitySessionConfig {
                invocation: BrokerInvocationContext {
                    invocation_id: "invocation-1".to_owned(),
                    job_id: "job-1".to_owned(),
                    user_subject: "user-1".to_owned(),
                    call_chain_id: "call-chain-1".to_owned(),
                    operation: types::Operation::Summarize,
                    trigger: types::Trigger::ManualApi,
                    remaining_depth: 2,
                },
                provider_binding_id: "provider-1".to_owned(),
                tool_bindings: vec![test_tool_binding()],
                remaining_provider_requests: 3,
                remaining_mcp_calls: 1,
                remaining_input_tokens: 1024,
                remaining_output_tokens: 1024,
                remaining_cost_micros: 1,
                deadline_unix_ms,
                deadline: Instant::now() + duration,
            },
            Arc::new(DenyAiBroker),
            Arc::new(DenyMcpBroker),
        )
        .expect("test session should construct");
        session.suspend();

        let ai_error = session
            .validate_ai_request(host_ai::GenerateRequest {
                provider_binding_id: "provider-1".to_owned(),
                operation: types::Operation::Summarize,
                system_instruction: "Summarize.".to_owned(),
                untrusted_input_json: r#"{"text":"entry"}"#.to_owned(),
                output_schema_id: "raindrop://schemas/test/v1".to_owned(),
                output_schema_json: r#"{"type":"object"}"#.to_owned(),
                provider_request_ordinal: 1,
                max_input_tokens: 1,
                max_output_tokens: 1,
            })
            .expect_err("AI must be denied while descriptor is unverified");
        assert_eq!(ai_error.code, host_ai::GenerateErrorCode::CapabilityDenied);

        let mcp_error = session
            .validate_mcp_request(host_mcp::CallRequest {
                tool_binding_id: "tool-1".to_owned(),
                arguments_json: r#"{"query":"entry"}"#.to_owned(),
                requested_timeout_ms: 1,
            })
            .expect_err("MCP must be denied while descriptor is unverified");
        assert_eq!(mcp_error.code, host_mcp::CallErrorCode::CapabilityDenied);

        session.activate();
        assert!(
            session
                .validate_mcp_request(host_mcp::CallRequest {
                    tool_binding_id: "tool-1".to_owned(),
                    arguments_json: r#"{"query":"entry"}"#.to_owned(),
                    requested_timeout_ms: 1,
                })
                .is_ok()
        );
    }

    fn test_tool_binding() -> CapabilityToolBinding {
        let input_schema_json = r#"{"type":"object"}"#.to_owned();
        CapabilityToolBinding::new(CapabilityToolBindingInput {
            binding_id: "tool-1".to_owned(),
            connection_id: "00000000-0000-4000-8000-000000000001".to_owned(),
            tool_name: "search.read".to_owned(),
            display_label: "Search".to_owned(),
            description: "Untrusted description".to_owned(),
            input_schema_digest: contextual_hash(
                TOOL_SCHEMA_HASH_CONTEXT,
                input_schema_json.as_bytes(),
            ),
            input_schema_json,
        })
        .expect("test tool binding")
    }
}
