pub mod types {
    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub enum Operation {
        Summarize,
        Translate,
    }

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub enum Trigger {
        ManualApi,
        ReaderSidecar,
        FeedRefreshPersisted,
        McpServer,
    }

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub enum PluginErrorCode {
        Disabled,
        InvalidRequest,
        CapabilityDenied,
        ConfigInvalid,
        OutputInvalid,
        OutputTooLarge,
        BudgetExhausted,
        DeadlineExceeded,
        HostFailure,
    }

    #[derive(Clone, Debug, Eq, PartialEq)]
    pub struct LifecycleSubscription {
        pub event: String,
        pub schema_version: u32,
    }

    #[derive(Clone, Debug, Eq, PartialEq)]
    pub struct PluginDescriptor {
        pub plugin_key: String,
        pub version: String,
        pub abi: String,
        pub operations: Vec<Operation>,
        pub lifecycle_subscriptions: Vec<LifecycleSubscription>,
        pub required_capabilities: Vec<String>,
        pub optional_capabilities: Vec<String>,
    }

    #[derive(Clone, Debug, Eq, PartialEq)]
    pub struct UserScope {
        pub subject: String,
    }

    #[derive(Clone, Debug, Eq, PartialEq)]
    pub struct EntryReference {
        pub entry_id: String,
        pub feed_id: String,
        pub content_hash: String,
        pub title: String,
        pub text: String,
        pub canonical_url: Option<String>,
        pub source_locale: Option<String>,
    }

    #[derive(Clone, Debug, Eq, PartialEq)]
    pub struct ToolBinding {
        pub binding_id: String,
        pub connection_id: String,
        pub tool_name: String,
        pub display_label: String,
        pub description: String,
        pub input_schema_json: String,
        pub input_schema_digest: String,
    }

    #[derive(Clone, Debug, Eq, PartialEq)]
    pub struct InvocationBudget {
        pub remaining_depth: u32,
        pub deadline_unix_ms: u64,
        pub remaining_provider_requests: u32,
        pub remaining_mcp_calls: u32,
        pub remaining_input_tokens: u32,
        pub remaining_output_tokens: u32,
        pub remaining_cost_micros: u64,
    }

    #[derive(Clone, Debug, Eq, PartialEq)]
    pub struct OperationRequest {
        pub invocation_id: String,
        pub job_id: String,
        pub idempotency_key: String,
        pub plugin_key: String,
        pub plugin_version: String,
        pub component_digest: String,
        pub user_scope: UserScope,
        pub trigger: Trigger,
        pub operation: Operation,
        pub target_locale: Option<String>,
        pub entry: EntryReference,
        pub config_json: String,
        pub config_hash: String,
        pub provider_binding_id: String,
        pub tool_bindings: Vec<ToolBinding>,
        pub call_chain_id: String,
        pub budget: InvocationBudget,
    }

    #[derive(Clone, Debug, Eq, PartialEq)]
    pub struct ArtifactCandidate {
        pub schema_id: String,
        pub locale: Option<String>,
        pub payload_json: String,
        pub provenance_json: String,
    }

    #[derive(Clone, Debug, Eq, PartialEq)]
    pub struct LifecycleEvent {
        pub event_id: String,
        pub event_type: String,
        pub schema_version: u32,
        pub refresh_id: String,
        pub sequence: u32,
        pub occurred_at: String,
        pub idempotency_key: String,
        pub user_scope: UserScope,
        pub context_json: String,
    }

    #[derive(Clone, Debug, Eq, PartialEq)]
    pub struct LifecycleRequest {
        pub invocation_id: String,
        pub plugin_key: String,
        pub plugin_version: String,
        pub component_digest: String,
        pub config_json: String,
        pub config_hash: String,
        pub event: LifecycleEvent,
    }

    #[derive(Clone, Debug, Eq, PartialEq)]
    pub struct ContentJobIntent {
        pub operation: Operation,
        pub entry_id: String,
        pub target_locale: Option<String>,
        pub idempotency_key: String,
    }

    #[derive(Clone, Debug, Eq, PartialEq)]
    pub struct EventOutcome {
        pub job_intents: Vec<ContentJobIntent>,
        pub diagnostics: Vec<String>,
    }

    #[derive(Clone, Debug, Eq, PartialEq)]
    pub struct PluginError {
        pub code: PluginErrorCode,
        pub retryable: bool,
        pub message_key: String,
    }
}

pub mod host_ai {
    use super::types::Operation;
    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub enum FinishReason {
        Completed,
        Length,
        ContentFilter,
        ToolPlan,
        Unknown,
    }

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub enum GenerateErrorCode {
        CapabilityDenied,
        ProviderUnavailable,
        QuotaExceeded,
        RateLimited,
        Timeout,
        OutputSchemaInvalid,
        CostLimitExceeded,
        InvalidRequest,
    }

    #[derive(Clone, Debug, Eq, PartialEq)]
    pub struct GenerateRequest {
        pub provider_binding_id: String,
        pub operation: Operation,
        pub system_instruction: String,
        pub untrusted_input_json: String,
        pub output_schema_id: String,
        pub output_schema_json: String,
        pub provider_request_ordinal: u32,
        pub max_input_tokens: u32,
        pub max_output_tokens: u32,
    }

    #[derive(Clone, Debug, Eq, PartialEq)]
    pub struct TokenUsage {
        pub input_tokens: Option<u64>,
        pub output_tokens: Option<u64>,
    }

    #[derive(Clone, Debug, Eq, PartialEq)]
    pub struct GenerateResponse {
        pub output_json: String,
        pub finish_reason: FinishReason,
        pub usage: TokenUsage,
        pub model_label: String,
        pub estimated_cost_micros: Option<u64>,
    }

    #[derive(Clone, Debug, Eq, PartialEq)]
    pub struct GenerateError {
        pub code: GenerateErrorCode,
        pub retryable: bool,
        pub retry_at_unix_ms: Option<u64>,
    }
}

pub mod host_mcp {
    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub enum CallErrorCode {
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
    }

    #[derive(Clone, Debug, Eq, PartialEq)]
    pub struct CallRequest {
        pub tool_binding_id: String,
        pub arguments_json: String,
        pub requested_timeout_ms: u32,
    }

    #[derive(Clone, Debug, Eq, PartialEq)]
    pub struct CallResponse {
        pub result_json: String,
        pub connection_label: String,
        pub tool_label: String,
    }

    #[derive(Clone, Debug, Eq, PartialEq)]
    pub struct CallError {
        pub code: CallErrorCode,
        pub retryable: bool,
    }
}

#[async_trait::async_trait]
pub trait ContentHost: Send {
    async fn generate_structured(
        &mut self,
        request: host_ai::GenerateRequest,
    ) -> Result<Result<host_ai::GenerateResponse, host_ai::GenerateError>, ()>;
    async fn call_tool(
        &mut self,
        request: host_mcp::CallRequest,
    ) -> Result<Result<host_mcp::CallResponse, host_mcp::CallError>, ()>;
}
