use crate::{
    Failure,
    config::{Config, OperationKind},
    lifecycle::{LifecycleInput, build_intents},
    operation::{
        AiError, AiRequest, AiResponse, Artifact, EntryInput, ExecuteInput, FinishReason,
        HostCapabilities, InvocationBudget, McpError, McpRequest, McpResponse, ToolBinding,
        execute,
    },
};

wit_bindgen::generate!({
    path: "../../../contracts/wit/raindrop-content-plugin-v1",
    world: "content-plugin-v1",
});

use exports::raindrop::content_plugin::content_plugin::Guest;
use raindrop::content_plugin::{host_ai, host_mcp, types};

struct Component;

impl Guest for Component {
    fn descriptor() -> types::PluginDescriptor {
        types::PluginDescriptor {
            plugin_key: "raindrop.ai-content".to_owned(),
            version: "1.0.0".to_owned(),
            abi: "raindrop:content-plugin@1.0.0".to_owned(),
            operations: vec![types::Operation::Summarize, types::Operation::Translate],
            lifecycle_subscriptions: vec![types::LifecycleSubscription {
                event: "feed.refresh.persisted".to_owned(),
                schema_version: 1,
            }],
            required_capabilities: vec!["ai.generate_structured".to_owned()],
            optional_capabilities: vec!["mcp.call_tool".to_owned()],
        }
    }

    fn execute(
        request: types::OperationRequest,
    ) -> Result<types::ArtifactCandidate, types::PluginError> {
        let input = map_execute_input(request)?;
        let mut host = WitHost;
        execute(&mut host, input)
            .map(map_artifact)
            .map_err(map_failure)
    }

    fn on_event(
        request: types::LifecycleRequest,
    ) -> Result<types::EventOutcome, types::PluginError> {
        if request.event.event_type != "feed.refresh.persisted" || request.event.schema_version != 1
        {
            return Err(map_failure(Failure::ConfigInvalid));
        }
        let config = Config::parse(&request.config_json).map_err(map_failure)?;
        let intents = build_intents(
            &config,
            LifecycleInput {
                event_id: &request.event.event_id,
                subject: &request.event.user_scope.subject,
                config_hash: &request.config_hash,
                context_json: &request.event.context_json,
            },
        )
        .map_err(map_failure)?;
        Ok(types::EventOutcome {
            job_intents: intents
                .into_iter()
                .map(|intent| types::ContentJobIntent {
                    operation: map_operation_to_wit(intent.operation),
                    entry_id: intent.entry_id,
                    target_locale: intent.target_locale,
                    idempotency_key: intent.idempotency_key,
                })
                .collect(),
            diagnostics: Vec::new(),
        })
    }
}

struct WitHost;

impl HostCapabilities for WitHost {
    fn generate(&mut self, request: AiRequest) -> Result<AiResponse, AiError> {
        host_ai::generate_structured(&host_ai::GenerateRequest {
            provider_binding_id: request.provider_binding_id,
            operation: map_operation_to_wit(request.operation),
            system_instruction: request.system_instruction.to_owned(),
            untrusted_input_json: request.untrusted_input_json,
            output_schema_id: request.output_schema_id.to_owned(),
            output_schema_json: request.output_schema_json,
            provider_request_ordinal: request.provider_request_ordinal,
            max_input_tokens: request.max_input_tokens,
            max_output_tokens: request.max_output_tokens,
        })
        .map(|response| AiResponse {
            output_json: response.output_json,
            finish_reason: match response.finish_reason {
                host_ai::FinishReason::Completed => FinishReason::Completed,
                host_ai::FinishReason::Length => FinishReason::Length,
                host_ai::FinishReason::ContentFilter => FinishReason::ContentFilter,
                host_ai::FinishReason::ToolPlan => FinishReason::ToolPlan,
                host_ai::FinishReason::Unknown => FinishReason::Unknown,
            },
        })
        .map_err(|error| match error.code {
            host_ai::GenerateErrorCode::CapabilityDenied => AiError::CapabilityDenied,
            host_ai::GenerateErrorCode::ProviderUnavailable => AiError::ProviderUnavailable,
            host_ai::GenerateErrorCode::QuotaExceeded => AiError::QuotaExceeded,
            host_ai::GenerateErrorCode::RateLimited => AiError::RateLimited,
            host_ai::GenerateErrorCode::Timeout => AiError::Timeout,
            host_ai::GenerateErrorCode::OutputSchemaInvalid => AiError::OutputSchemaInvalid,
            host_ai::GenerateErrorCode::CostLimitExceeded => AiError::CostLimitExceeded,
            host_ai::GenerateErrorCode::InvalidRequest => AiError::InvalidRequest,
        })
    }

    fn call_tool(&mut self, request: McpRequest) -> Result<McpResponse, McpError> {
        host_mcp::call_tool(&host_mcp::CallRequest {
            tool_binding_id: request.binding_id,
            arguments_json: request.arguments_json,
            requested_timeout_ms: request.requested_timeout_ms,
        })
        .map(|response| McpResponse {
            result_json: response.result_json,
            connection_label: response.connection_label,
            tool_label: response.tool_label,
        })
        .map_err(|error| match error.code {
            host_mcp::CallErrorCode::Disabled => McpError::Disabled,
            host_mcp::CallErrorCode::CapabilityDenied => McpError::CapabilityDenied,
            host_mcp::CallErrorCode::ConnectionDenied => McpError::ConnectionDenied,
            host_mcp::CallErrorCode::ToolDenied => McpError::ToolDenied,
            host_mcp::CallErrorCode::SideEffectConfirmationRequired => {
                McpError::SideEffectConfirmationRequired
            }
            host_mcp::CallErrorCode::SchemaInvalid => McpError::SchemaInvalid,
            host_mcp::CallErrorCode::Timeout => McpError::Timeout,
            host_mcp::CallErrorCode::ResultTooLarge => McpError::ResultTooLarge,
            host_mcp::CallErrorCode::BudgetExhausted => McpError::BudgetExhausted,
            host_mcp::CallErrorCode::RecursionBlocked => McpError::RecursionBlocked,
        })
    }
}

fn map_execute_input(request: types::OperationRequest) -> Result<ExecuteInput, types::PluginError> {
    Ok(ExecuteInput {
        operation: map_operation(request.operation),
        target_locale: request.target_locale,
        entry: EntryInput {
            entry_id: request.entry.entry_id,
            feed_id: request.entry.feed_id,
            content_hash: request.entry.content_hash,
            title: request.entry.title,
            text: request.entry.text,
            canonical_url: request.entry.canonical_url,
            source_locale: request.entry.source_locale,
        },
        config_json: request.config_json,
        provider_binding_id: request.provider_binding_id,
        tool_bindings: request
            .tool_bindings
            .into_iter()
            .map(|binding| ToolBinding {
                binding_id: binding.binding_id,
                connection_id: binding.connection_id,
                tool_name: binding.tool_name,
                display_label: binding.display_label,
                description: binding.description,
                input_schema_json: binding.input_schema_json,
                input_schema_digest: binding.input_schema_digest,
            })
            .collect(),
        budget: InvocationBudget {
            remaining_mcp_calls: request.budget.remaining_mcp_calls,
            remaining_input_tokens: request.budget.remaining_input_tokens,
            remaining_output_tokens: request.budget.remaining_output_tokens,
        },
    })
}

const fn map_operation(operation: types::Operation) -> OperationKind {
    match operation {
        types::Operation::Summarize => OperationKind::Summarize,
        types::Operation::Translate => OperationKind::Translate,
    }
}

const fn map_operation_to_wit(operation: OperationKind) -> types::Operation {
    match operation {
        OperationKind::Summarize => types::Operation::Summarize,
        OperationKind::Translate => types::Operation::Translate,
    }
}

fn map_artifact(artifact: Artifact) -> types::ArtifactCandidate {
    types::ArtifactCandidate {
        schema_id: artifact.schema_id.to_owned(),
        locale: artifact.locale,
        payload_json: artifact.payload_json,
        provenance_json: artifact.provenance_json,
    }
}

fn map_failure(failure: Failure) -> types::PluginError {
    let code = match failure {
        Failure::Disabled => types::PluginErrorCode::Disabled,
        Failure::ConfigInvalid => types::PluginErrorCode::ConfigInvalid,
        Failure::ProviderOutputInvalid | Failure::OutputInvalid => {
            types::PluginErrorCode::OutputInvalid
        }
        Failure::BudgetExhausted | Failure::McpBudgetExhausted => {
            types::PluginErrorCode::BudgetExhausted
        }
        Failure::ProviderUnavailable
        | Failure::ProviderRateLimited
        | Failure::ProviderTimeout
        | Failure::McpSchemaInvalid
        | Failure::McpTimeout
        | Failure::McpRecursionBlocked => types::PluginErrorCode::HostFailure,
    };
    types::PluginError {
        code,
        retryable: matches!(
            failure,
            Failure::ProviderUnavailable
                | Failure::ProviderRateLimited
                | Failure::ProviderTimeout
                | Failure::McpTimeout
        ),
        message_key: failure.message_key().to_owned(),
    }
}

export!(Component);
