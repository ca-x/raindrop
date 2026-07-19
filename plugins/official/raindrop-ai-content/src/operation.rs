use std::collections::{BTreeSet, HashSet};

use serde_json::{Value, json};

use crate::{
    Failure,
    config::{
        Config, McpConfig, McpFailurePolicy, McpMode, OperationKind, SummaryStyle, normalize_locale,
    },
    json::{canonical_json, parse_canonical_object, parse_unique},
    prompt::{TOOL_PLAN_PROMPT_VERSION, final_instruction, prompt_version, tool_plan_instruction},
    tool_plan::{TOOL_PLAN_SCHEMA_ID, ToolDescriptor, build_schema, parse_plan},
};

const SUMMARY_SCHEMA_ID: &str = "raindrop://schemas/artifacts/ai-summary/v1";
const TRANSLATION_SCHEMA_ID: &str = "raindrop://schemas/artifacts/ai-translation/v1";
const SUMMARY_SCHEMA_DOCUMENT: &str =
    include_str!("../../../../contracts/artifacts/ai-summary.v1.schema.json");
const TRANSLATION_SCHEMA_DOCUMENT: &str =
    include_str!("../../../../contracts/artifacts/ai-translation.v1.schema.json");
const MAX_CANONICAL_INPUT_BYTES: usize = 512 * 1024;
const MAX_SCHEMA_BYTES: usize = 64 * 1024;
const MAX_MCP_RESULT_BYTES: usize = 256 * 1024;
const PLAN_OUTPUT_TOKENS: u32 = 1_024;
const MCP_TIMEOUT_MS: u32 = 10_000;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ExecuteInput {
    pub(crate) operation: OperationKind,
    pub(crate) target_locale: Option<String>,
    pub(crate) entry: EntryInput,
    pub(crate) config_json: String,
    pub(crate) provider_binding_id: String,
    pub(crate) tool_bindings: Vec<ToolBinding>,
    pub(crate) budget: InvocationBudget,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct EntryInput {
    pub(crate) entry_id: String,
    pub(crate) feed_id: String,
    pub(crate) content_hash: String,
    pub(crate) title: String,
    pub(crate) text: String,
    pub(crate) canonical_url: Option<String>,
    pub(crate) source_locale: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ToolBinding {
    pub(crate) binding_id: String,
    pub(crate) connection_id: String,
    pub(crate) tool_name: String,
    pub(crate) display_label: String,
    pub(crate) description: String,
    pub(crate) input_schema_json: String,
    pub(crate) input_schema_digest: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct InvocationBudget {
    pub(crate) remaining_mcp_calls: u32,
    pub(crate) remaining_input_tokens: u32,
    pub(crate) remaining_output_tokens: u32,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Artifact {
    pub(crate) schema_id: &'static str,
    pub(crate) locale: Option<String>,
    pub(crate) payload_json: String,
    pub(crate) provenance_json: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct AiRequest {
    pub(crate) provider_binding_id: String,
    pub(crate) operation: OperationKind,
    pub(crate) system_instruction: &'static str,
    pub(crate) untrusted_input_json: String,
    pub(crate) output_schema_id: &'static str,
    pub(crate) output_schema_json: String,
    pub(crate) provider_request_ordinal: u32,
    pub(crate) max_input_tokens: u32,
    pub(crate) max_output_tokens: u32,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct AiResponse {
    pub(crate) output_json: String,
    pub(crate) finish_reason: FinishReason,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum FinishReason {
    Completed,
    Length,
    ContentFilter,
    ToolPlan,
    Unknown,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum AiError {
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
pub(crate) struct McpRequest {
    pub(crate) binding_id: String,
    pub(crate) arguments_json: String,
    pub(crate) requested_timeout_ms: u32,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct McpResponse {
    pub(crate) result_json: String,
    pub(crate) connection_label: String,
    pub(crate) tool_label: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum McpError {
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

pub(crate) trait HostCapabilities {
    fn generate(&mut self, request: AiRequest) -> Result<AiResponse, AiError>;
    fn call_tool(&mut self, request: McpRequest) -> Result<McpResponse, McpError>;
}

pub(crate) fn execute(
    host: &mut impl HostCapabilities,
    input: ExecuteInput,
) -> Result<Artifact, Failure> {
    let config = Config::parse(&input.config_json)?;
    let operation_config = config.operation(input.operation);
    if !operation_config.enabled {
        return Err(Failure::Disabled);
    }
    if operation_config.provider_id != input.provider_binding_id {
        return Err(Failure::ConfigInvalid);
    }
    let target_locale = validate_target_locale(&config, &input)?;
    let base_input = base_untrusted_input(&config, &input, target_locale.as_deref())?;
    let mut budget = BudgetState::from(input.budget);
    let selected_bindings = selected_bindings(operation_config.mcp, &input.tool_bindings)?;
    if operation_config.mcp.mode == McpMode::Disabled || selected_bindings.is_empty() {
        return generate_final(
            host,
            &input,
            operation_config.max_output_tokens,
            target_locale,
            base_input,
            &mut budget,
            1,
            McpStatus::Disabled,
            0,
        );
    }

    let max_calls = usize::try_from(input.budget.remaining_mcp_calls)
        .unwrap_or(usize::MAX)
        .min(usize::from(operation_config.mcp.max_tool_calls))
        .min(selected_bindings.len())
        .min(4);
    if max_calls == 0 {
        return generate_final(
            host,
            &input,
            operation_config.max_output_tokens,
            target_locale,
            base_input,
            &mut budget,
            1,
            McpStatus::Disabled,
            0,
        );
    }

    let plan_schema = build_schema(
        selected_bindings.iter().map(|binding| ToolDescriptor {
            binding_id: &binding.binding_id,
            input_schema_json: &binding.input_schema_json,
        }),
        max_calls,
    )
    .map_err(|_| Failure::McpSchemaInvalid)?;
    let plan_input = tool_plan_input(&input, target_locale.as_deref(), &selected_bindings)?;
    let minimum_final = minimum_output_tokens(input.operation);
    if budget.output_tokens <= minimum_final {
        return Err(Failure::BudgetExhausted);
    }
    let plan_output_tokens = PLAN_OUTPUT_TOKENS.min(budget.output_tokens - minimum_final);
    let plan_request = prepare_ai_request(
        &input,
        tool_plan_instruction(),
        plan_input,
        TOOL_PLAN_SCHEMA_ID,
        plan_schema,
        1,
        plan_output_tokens,
        &mut budget,
    )?;
    let plan_response = match host.generate(plan_request) {
        Ok(response) => response,
        Err(error) => {
            return handle_mcp_failure(
                host,
                &input,
                operation_config.max_output_tokens,
                target_locale,
                base_input,
                &mut budget,
                operation_config.mcp.failure_policy,
                map_ai_error(error),
            );
        }
    };
    if plan_response.finish_reason != FinishReason::Completed {
        return handle_mcp_failure(
            host,
            &input,
            operation_config.max_output_tokens,
            target_locale,
            base_input,
            &mut budget,
            operation_config.mcp.failure_policy,
            Failure::McpSchemaInvalid,
        );
    }
    let allowed_binding_ids = selected_bindings
        .iter()
        .map(|binding| binding.binding_id.clone())
        .collect::<BTreeSet<_>>();
    let calls = match parse_plan(&plan_response.output_json, &allowed_binding_ids, max_calls) {
        Ok(calls) => calls,
        Err(()) => {
            return handle_mcp_failure(
                host,
                &input,
                operation_config.max_output_tokens,
                target_locale,
                base_input,
                &mut budget,
                operation_config.mcp.failure_policy,
                Failure::McpSchemaInvalid,
            );
        }
    };

    let mut contexts = Vec::with_capacity(calls.len());
    for call in calls {
        if budget.mcp_calls == 0 {
            return handle_mcp_failure(
                host,
                &input,
                operation_config.max_output_tokens,
                target_locale,
                base_input,
                &mut budget,
                operation_config.mcp.failure_policy,
                Failure::McpBudgetExhausted,
            );
        }
        budget.mcp_calls -= 1;
        let response = match host.call_tool(McpRequest {
            binding_id: call.binding_id,
            arguments_json: call.arguments_json,
            requested_timeout_ms: MCP_TIMEOUT_MS,
        }) {
            Ok(response) => response,
            Err(error) => {
                return handle_mcp_failure(
                    host,
                    &input,
                    operation_config.max_output_tokens,
                    target_locale,
                    base_input,
                    &mut budget,
                    operation_config.mcp.failure_policy,
                    map_mcp_error(error),
                );
            }
        };
        let result = match parse_canonical_object(&response.result_json, MAX_MCP_RESULT_BYTES) {
            Ok(result) => result,
            Err(_) => {
                return handle_mcp_failure(
                    host,
                    &input,
                    operation_config.max_output_tokens,
                    target_locale,
                    base_input,
                    &mut budget,
                    operation_config.mcp.failure_policy,
                    Failure::McpSchemaInvalid,
                );
            }
        };
        contexts.push(json!({
            "connectionLabel": response.connection_label,
            "result": result,
            "toolLabel": response.tool_label,
        }));
    }
    let successful_call_count = contexts.len();
    let enriched_input = match with_mcp_context(base_input.clone(), contexts) {
        Ok(input) => input,
        Err(error) => {
            return handle_mcp_failure(
                host,
                &input,
                operation_config.max_output_tokens,
                target_locale,
                base_input,
                &mut budget,
                operation_config.mcp.failure_policy,
                error,
            );
        }
    };
    generate_final(
        host,
        &input,
        operation_config.max_output_tokens,
        target_locale,
        enriched_input,
        &mut budget,
        2,
        McpStatus::Applied,
        successful_call_count,
    )
}

#[allow(
    clippy::too_many_arguments,
    reason = "the fail-open handoff keeps every consumed budget and final-generation input explicit"
)]
fn handle_mcp_failure(
    host: &mut impl HostCapabilities,
    input: &ExecuteInput,
    configured_output_tokens: u32,
    target_locale: Option<String>,
    base_input: Value,
    budget: &mut BudgetState,
    policy: McpFailurePolicy,
    failure: Failure,
) -> Result<Artifact, Failure> {
    match policy {
        McpFailurePolicy::FailClosed => Err(failure),
        McpFailurePolicy::FailOpen => generate_final(
            host,
            input,
            configured_output_tokens,
            target_locale,
            base_input,
            budget,
            2,
            McpStatus::Degraded,
            0,
        ),
    }
}

#[allow(
    clippy::too_many_arguments,
    reason = "the final request contract keeps provenance, locale, and budget values visible at each call site"
)]
fn generate_final(
    host: &mut impl HostCapabilities,
    input: &ExecuteInput,
    configured_output_tokens: u32,
    target_locale: Option<String>,
    untrusted_input: Value,
    budget: &mut BudgetState,
    ordinal: u32,
    mcp_status: McpStatus,
    successful_call_count: usize,
) -> Result<Artifact, Failure> {
    let (schema_id, schema_document) = match input.operation {
        OperationKind::Summarize => (SUMMARY_SCHEMA_ID, SUMMARY_SCHEMA_DOCUMENT),
        OperationKind::Translate => (TRANSLATION_SCHEMA_ID, TRANSLATION_SCHEMA_DOCUMENT),
    };
    let output_schema_json = canonical_contract(schema_document)?;
    let untrusted_input_json = canonical_json(untrusted_input, MAX_CANONICAL_INPUT_BYTES)
        .map_err(|_| Failure::OutputInvalid)?;
    let maximum_output = configured_output_tokens.min(budget.output_tokens);
    if maximum_output < minimum_output_tokens(input.operation) {
        return Err(Failure::BudgetExhausted);
    }
    let style = (input.operation == OperationKind::Summarize).then_some(
        Config::parse(&input.config_json)?
            .operations
            .summarize
            .style,
    );
    let request = prepare_ai_request(
        input,
        final_instruction(input.operation, style),
        untrusted_input_json,
        schema_id,
        output_schema_json,
        ordinal,
        maximum_output,
        budget,
    )?;
    let response = host.generate(request).map_err(map_ai_error)?;
    if response.finish_reason != FinishReason::Completed {
        return Err(Failure::ProviderOutputInvalid);
    }
    parse_canonical_object(&response.output_json, MAX_CANONICAL_INPUT_BYTES)
        .map_err(|_| Failure::ProviderOutputInvalid)?;
    Ok(Artifact {
        schema_id,
        locale: target_locale,
        payload_json: response.output_json,
        provenance_json: provenance(input.operation, mcp_status, successful_call_count, ordinal)?,
    })
}

#[allow(
    clippy::too_many_arguments,
    reason = "the host capability request mirrors the committed WIT fields and explicit budget mutation"
)]
fn prepare_ai_request(
    input: &ExecuteInput,
    system_instruction: &'static str,
    untrusted_input_json: String,
    output_schema_id: &'static str,
    output_schema_json: String,
    ordinal: u32,
    max_output_tokens: u32,
    budget: &mut BudgetState,
) -> Result<AiRequest, Failure> {
    let byte_ceiling = system_instruction
        .len()
        .checked_add(untrusted_input_json.len())
        .and_then(|value| value.checked_add(output_schema_json.len()))
        .and_then(|value| u32::try_from(value).ok())
        .ok_or(Failure::BudgetExhausted)?;
    if byte_ceiling == 0
        || byte_ceiling > budget.input_tokens
        || max_output_tokens == 0
        || max_output_tokens > budget.output_tokens
    {
        return Err(Failure::BudgetExhausted);
    }
    budget.input_tokens -= byte_ceiling;
    budget.output_tokens -= max_output_tokens;
    Ok(AiRequest {
        provider_binding_id: input.provider_binding_id.clone(),
        operation: input.operation,
        system_instruction,
        untrusted_input_json,
        output_schema_id,
        output_schema_json,
        provider_request_ordinal: ordinal,
        max_input_tokens: byte_ceiling,
        max_output_tokens,
    })
}

fn validate_target_locale(
    config: &Config,
    input: &ExecuteInput,
) -> Result<Option<String>, Failure> {
    match input.operation {
        OperationKind::Summarize if input.target_locale.is_none() => Ok(None),
        OperationKind::Summarize => Err(Failure::ConfigInvalid),
        OperationKind::Translate => {
            let requested = input
                .target_locale
                .as_deref()
                .unwrap_or(&config.operations.translate.default_target_locale);
            let normalized = normalize_locale(requested).ok_or(Failure::ConfigInvalid)?;
            if normalized == requested {
                Ok(Some(normalized))
            } else {
                Err(Failure::ConfigInvalid)
            }
        }
    }
}

fn base_untrusted_input(
    config: &Config,
    input: &ExecuteInput,
    target_locale: Option<&str>,
) -> Result<Value, Failure> {
    let operation = match input.operation {
        OperationKind::Summarize => json!({
            "kind": "summarize",
            "style": summary_style_key(config.operations.summarize.style),
        }),
        OperationKind::Translate => json!({
            "kind": "translate",
            "targetLocale": target_locale.ok_or(Failure::ConfigInvalid)?,
        }),
    };
    let value = json!({
        "entry": {
            "canonicalUrl": input.entry.canonical_url,
            "contentHash": input.entry.content_hash,
            "entryId": input.entry.entry_id,
            "feedId": input.entry.feed_id,
            "sourceLocale": input.entry.source_locale,
            "text": input.entry.text,
            "title": input.entry.title,
        },
        "operation": operation,
    });
    canonical_json(value.clone(), MAX_CANONICAL_INPUT_BYTES).map_err(|_| Failure::OutputInvalid)?;
    Ok(value)
}

fn selected_bindings<'a>(
    config: &McpConfig,
    bindings: &'a [ToolBinding],
) -> Result<Vec<&'a ToolBinding>, Failure> {
    if config.mode == McpMode::Disabled || bindings.is_empty() {
        return Ok(Vec::new());
    }
    let mut seen_ids = HashSet::new();
    let mut selected = Vec::with_capacity(config.tools.len());
    for selection in &config.tools {
        let mut matches = bindings.iter().filter(|binding| {
            binding.connection_id == selection.connection_id
                && binding.tool_name == selection.tool_name
        });
        let binding = matches.next().ok_or(Failure::McpSchemaInvalid)?;
        if matches.next().is_some()
            || !seen_ids.insert(binding.binding_id.as_str())
            || !valid_binding(binding)
        {
            return Err(Failure::McpSchemaInvalid);
        }
        selected.push(binding);
    }
    if selected.len() != bindings.len() {
        return Err(Failure::McpSchemaInvalid);
    }
    selected.sort_by(|left, right| left.binding_id.cmp(&right.binding_id));
    Ok(selected)
}

fn valid_binding(binding: &ToolBinding) -> bool {
    !binding.binding_id.is_empty()
        && binding.binding_id.len() <= 128
        && binding
            .binding_id
            .bytes()
            .all(|byte| (0x21..=0x7e).contains(&byte))
        && !binding.display_label.trim().is_empty()
        && !binding.description.trim().is_empty()
        && parse_canonical_object(&binding.input_schema_json, MAX_SCHEMA_BYTES).is_ok()
        && binding.input_schema_digest.len() == 64
        && binding
            .input_schema_digest
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn tool_plan_input(
    input: &ExecuteInput,
    target_locale: Option<&str>,
    bindings: &[&ToolBinding],
) -> Result<String, Failure> {
    let tools = bindings
        .iter()
        .map(|binding| {
            json!({
                "bindingId": binding.binding_id,
                "connectionId": binding.connection_id,
                "description": binding.description,
                "displayLabel": binding.display_label,
                "inputSchemaDigest": binding.input_schema_digest,
                "toolName": binding.tool_name,
            })
        })
        .collect::<Vec<_>>();
    canonical_json(
        json!({
            "entry": {
                "canonicalUrl": input.entry.canonical_url,
                "sourceLocale": input.entry.source_locale,
                "text": input.entry.text,
                "title": input.entry.title,
            },
            "operation": input.operation.as_key(),
            "promptVersion": TOOL_PLAN_PROMPT_VERSION,
            "targetLocale": target_locale,
            "tools": tools,
        }),
        MAX_CANONICAL_INPUT_BYTES,
    )
    .map_err(|_| Failure::McpSchemaInvalid)
}

fn with_mcp_context(mut base: Value, contexts: Vec<Value>) -> Result<Value, Failure> {
    base.as_object_mut()
        .ok_or(Failure::McpSchemaInvalid)?
        .insert("mcpContext".to_owned(), Value::Array(contexts));
    canonical_json(base.clone(), MAX_CANONICAL_INPUT_BYTES)
        .map_err(|_| Failure::McpSchemaInvalid)?;
    Ok(base)
}

fn canonical_contract(document: &str) -> Result<String, Failure> {
    let value = parse_unique(document, MAX_SCHEMA_BYTES).map_err(|_| Failure::OutputInvalid)?;
    if !value.is_object() {
        return Err(Failure::OutputInvalid);
    }
    canonical_json(value, MAX_SCHEMA_BYTES).map_err(|_| Failure::OutputInvalid)
}

fn provenance(
    operation: OperationKind,
    status: McpStatus,
    successful_call_count: usize,
    provider_request_count: u32,
) -> Result<String, Failure> {
    canonical_json(
        json!({
            "mcp": {
                "status": status.as_str(),
                "successfulCallCount": successful_call_count,
            },
            "promptVersion": prompt_version(operation),
            "providerRequestCount": provider_request_count,
        }),
        4 * 1024,
    )
    .map_err(|_| Failure::OutputInvalid)
}

fn map_ai_error(error: AiError) -> Failure {
    match error {
        AiError::RateLimited => Failure::ProviderRateLimited,
        AiError::Timeout => Failure::ProviderTimeout,
        AiError::OutputSchemaInvalid | AiError::InvalidRequest => Failure::ProviderOutputInvalid,
        AiError::QuotaExceeded | AiError::CostLimitExceeded => Failure::BudgetExhausted,
        AiError::CapabilityDenied | AiError::ProviderUnavailable => Failure::ProviderUnavailable,
    }
}

fn map_mcp_error(error: McpError) -> Failure {
    match error {
        McpError::Timeout => Failure::McpTimeout,
        McpError::BudgetExhausted => Failure::McpBudgetExhausted,
        McpError::RecursionBlocked => Failure::McpRecursionBlocked,
        McpError::Disabled
        | McpError::CapabilityDenied
        | McpError::ConnectionDenied
        | McpError::ToolDenied
        | McpError::SideEffectConfirmationRequired
        | McpError::SchemaInvalid
        | McpError::ResultTooLarge => Failure::McpSchemaInvalid,
    }
}

const fn minimum_output_tokens(operation: OperationKind) -> u32 {
    match operation {
        OperationKind::Summarize => 128,
        OperationKind::Translate => 256,
    }
}

const fn summary_style_key(style: SummaryStyle) -> &'static str {
    match style {
        SummaryStyle::Concise => "CONCISE",
        SummaryStyle::Balanced => "BALANCED",
        SummaryStyle::Detailed => "DETAILED",
    }
}

#[derive(Clone, Copy)]
enum McpStatus {
    Disabled,
    Applied,
    Degraded,
}

impl McpStatus {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Disabled => "DISABLED",
            Self::Applied => "APPLIED",
            Self::Degraded => "DEGRADED",
        }
    }
}

struct BudgetState {
    mcp_calls: u32,
    input_tokens: u32,
    output_tokens: u32,
}

impl From<InvocationBudget> for BudgetState {
    fn from(value: InvocationBudget) -> Self {
        Self {
            mcp_calls: value.remaining_mcp_calls,
            input_tokens: value.remaining_input_tokens,
            output_tokens: value.remaining_output_tokens,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;

    use super::*;
    use crate::{config::tests::fixture_config, json::canonical_json};

    #[test]
    fn direct_summary_keeps_entry_data_out_of_policy_and_returns_safe_provenance() {
        let mut host = FakeHost::new([Ok(AiResponse {
            output_json: summary_output(),
            finish_reason: FinishReason::Completed,
        })]);
        let artifact = execute(&mut host, summary_input(false)).expect("summary artifact");
        assert_eq!(host.ai_requests.len(), 1);
        let request = &host.ai_requests[0];
        assert_eq!(request.provider_request_ordinal, 1);
        assert_eq!(request.output_schema_id, SUMMARY_SCHEMA_ID);
        assert!(!request.system_instruction.contains("rd-secret-entry"));
        assert!(request.untrusted_input_json.contains("rd-secret-entry"));
        assert_eq!(artifact.payload_json, summary_output());
        assert_eq!(artifact.locale, None);
        assert_eq!(
            artifact.provenance_json,
            r#"{"mcp":{"status":"DISABLED","successfulCallCount":0},"promptVersion":"raindrop-summary-v1","providerRequestCount":1}"#,
        );
    }

    #[test]
    fn translation_requires_exact_target_locale_and_schema() {
        let mut input = summary_input(false);
        input.operation = OperationKind::Translate;
        input.provider_binding_id = "00000000-0000-4000-8000-000000000102".to_owned();
        input.target_locale = Some("zh-CN".to_owned());
        let mut host = FakeHost::new([Ok(AiResponse {
            output_json: translation_output(),
            finish_reason: FinishReason::Completed,
        })]);
        let artifact = execute(&mut host, input).expect("translation artifact");
        assert_eq!(host.ai_requests[0].output_schema_id, TRANSLATION_SCHEMA_ID);
        assert_eq!(artifact.locale.as_deref(), Some("zh-CN"));
    }

    #[test]
    fn mcp_flow_uses_two_provider_requests_and_discards_partial_results_on_fail_open() {
        let plan = r#"{"calls":[{"arguments":{"query":"rust"},"toolBindingId":"binding-a"}],"schemaVersion":1}"#;
        let mut host = FakeHost::new([
            Ok(AiResponse {
                output_json: plan.to_owned(),
                finish_reason: FinishReason::Completed,
            }),
            Ok(AiResponse {
                output_json: summary_output(),
                finish_reason: FinishReason::Completed,
            }),
        ]);
        host.mcp_responses.push_back(Ok(McpResponse {
            result_json: r#"{"facts":["safe"]}"#.to_owned(),
            connection_label: "Search".to_owned(),
            tool_label: "Read".to_owned(),
        }));
        let artifact = execute(&mut host, summary_input(true)).expect("MCP artifact");
        assert_eq!(host.ai_requests.len(), 2);
        assert_eq!(host.ai_requests[0].provider_request_ordinal, 1);
        assert_eq!(host.ai_requests[0].output_schema_id, TOOL_PLAN_SCHEMA_ID);
        assert_eq!(host.ai_requests[1].provider_request_ordinal, 2);
        assert!(
            host.ai_requests[1]
                .untrusted_input_json
                .contains("mcpContext")
        );
        assert_eq!(host.mcp_requests[0].requested_timeout_ms, MCP_TIMEOUT_MS);
        assert!(artifact.provenance_json.contains(r#""status":"APPLIED""#));

        let mut host = FakeHost::new([
            Ok(AiResponse {
                output_json: plan.to_owned(),
                finish_reason: FinishReason::Completed,
            }),
            Ok(AiResponse {
                output_json: summary_output(),
                finish_reason: FinishReason::Completed,
            }),
        ]);
        host.mcp_responses.push_back(Err(McpError::Timeout));
        let artifact = execute(&mut host, summary_input(true)).expect("fail-open artifact");
        assert_eq!(host.ai_requests.len(), 2);
        assert!(
            !host.ai_requests[1]
                .untrusted_input_json
                .contains("mcpContext")
        );
        assert!(artifact.provenance_json.contains(r#""status":"DEGRADED""#));
    }

    #[test]
    fn direct_failures_are_fixed_and_never_return_partial_artifacts() {
        let mut disabled = summary_input(false);
        let mut config: Value = serde_json::from_str(&disabled.config_json).expect("config JSON");
        config["operations"]["summarize"]["enabled"] = json!(false);
        config["automatic"]["enabled"] = json!(false);
        disabled.config_json = canonical_json(config, 256 * 1024).expect("disabled config");
        let mut host = FakeHost::new([]);
        assert_eq!(execute(&mut host, disabled), Err(Failure::Disabled));
        assert!(host.ai_requests.is_empty());

        let mut host = FakeHost::new([Err(AiError::Timeout)]);
        assert_eq!(
            execute(&mut host, summary_input(false)),
            Err(Failure::ProviderTimeout),
        );

        let mut host = FakeHost::new([Ok(AiResponse {
            output_json: summary_output(),
            finish_reason: FinishReason::Length,
        })]);
        assert_eq!(
            execute(&mut host, summary_input(false)),
            Err(Failure::ProviderOutputInvalid),
        );
    }

    #[test]
    fn invalid_plan_and_aggregate_overflow_fail_open_without_leaking_tool_results() {
        let mut invalid_plan_host = FakeHost::new([
            Ok(AiResponse {
                output_json:
                    r#"{"calls":[{"arguments":{},"toolBindingId":"unknown"}],"schemaVersion":1}"#
                        .to_owned(),
                finish_reason: FinishReason::Completed,
            }),
            Ok(AiResponse {
                output_json: summary_output(),
                finish_reason: FinishReason::Completed,
            }),
        ]);
        let artifact = execute(&mut invalid_plan_host, summary_input(true))
            .expect("invalid plan should fail open");
        assert_eq!(invalid_plan_host.ai_requests.len(), 2);
        assert!(invalid_plan_host.mcp_requests.is_empty());
        assert!(
            !invalid_plan_host.ai_requests[1]
                .untrusted_input_json
                .contains("mcpContext")
        );
        assert!(artifact.provenance_json.contains(r#""status":"DEGRADED""#));

        let plan = r#"{"calls":[{"arguments":{"query":"a"},"toolBindingId":"binding-a"},{"arguments":{"query":"b"},"toolBindingId":"binding-b"}],"schemaVersion":1}"#;
        let mut aggregate_host = FakeHost::new([
            Ok(AiResponse {
                output_json: plan.to_owned(),
                finish_reason: FinishReason::Completed,
            }),
            Ok(AiResponse {
                output_json: summary_output(),
                finish_reason: FinishReason::Completed,
            }),
        ]);
        for label in ["first", "second"] {
            aggregate_host.mcp_responses.push_back(Ok(McpResponse {
                result_json: canonical_json(
                    json!({"label": label, "payload": "x".repeat(262_000)}),
                    MAX_MCP_RESULT_BYTES,
                )
                .expect("bounded MCP result"),
                connection_label: "Search".to_owned(),
                tool_label: label.to_owned(),
            }));
        }
        let artifact = execute(&mut aggregate_host, two_tool_input())
            .expect("aggregate overflow should fail open");
        assert_eq!(aggregate_host.mcp_requests.len(), 2);
        assert_eq!(aggregate_host.ai_requests.len(), 2);
        assert!(
            !aggregate_host.ai_requests[1]
                .untrusted_input_json
                .contains("mcpContext")
        );
        assert!(artifact.provenance_json.contains(r#""status":"DEGRADED""#));
    }

    #[test]
    fn host_result_enums_cover_every_committed_wit_variant() {
        let finishes = [
            FinishReason::Completed,
            FinishReason::Length,
            FinishReason::ContentFilter,
            FinishReason::ToolPlan,
            FinishReason::Unknown,
        ];
        assert_eq!(finishes.len(), 5);

        let ai_errors = [
            AiError::CapabilityDenied,
            AiError::ProviderUnavailable,
            AiError::QuotaExceeded,
            AiError::RateLimited,
            AiError::Timeout,
            AiError::OutputSchemaInvalid,
            AiError::CostLimitExceeded,
            AiError::InvalidRequest,
        ];
        assert!(ai_errors.into_iter().all(|error| {
            map_ai_error(error)
                .message_key()
                .starts_with("raindrop.ai-content.")
        }));

        let mcp_errors = [
            McpError::Disabled,
            McpError::CapabilityDenied,
            McpError::ConnectionDenied,
            McpError::ToolDenied,
            McpError::SideEffectConfirmationRequired,
            McpError::SchemaInvalid,
            McpError::Timeout,
            McpError::ResultTooLarge,
            McpError::BudgetExhausted,
            McpError::RecursionBlocked,
        ];
        assert!(mcp_errors.into_iter().all(|error| {
            map_mcp_error(error)
                .message_key()
                .starts_with("raindrop.ai-content.")
        }));
    }

    fn summary_input(with_tool: bool) -> ExecuteInput {
        let mut tool_bindings = Vec::new();
        if with_tool {
            tool_bindings.push(ToolBinding {
                binding_id: "binding-a".to_owned(),
                connection_id: "00000000-0000-4000-8000-000000000201".to_owned(),
                tool_name: "search.read".to_owned(),
                display_label: "Search".to_owned(),
                description: "rd-secret-tool-description".to_owned(),
                input_schema_json: r#"{"additionalProperties":false,"properties":{"query":{"type":"string"}},"type":"object"}"#.to_owned(),
                input_schema_digest: "a".repeat(64),
            });
        }
        ExecuteInput {
            operation: OperationKind::Summarize,
            target_locale: None,
            entry: EntryInput {
                entry_id: "entry-1".to_owned(),
                feed_id: "feed-1".to_owned(),
                content_hash: "b".repeat(64),
                title: "Fixture title".to_owned(),
                text: "rd-secret-entry".to_owned(),
                canonical_url: Some("https://example.test/1".to_owned()),
                source_locale: Some("en".to_owned()),
            },
            config_json: fixture_config(),
            provider_binding_id: "00000000-0000-4000-8000-000000000101".to_owned(),
            tool_bindings,
            budget: InvocationBudget {
                remaining_mcp_calls: 4,
                remaining_input_tokens: 64 * 1024,
                remaining_output_tokens: 4 * 1024,
            },
        }
    }

    fn two_tool_input() -> ExecuteInput {
        let mut input = summary_input(true);
        let mut config: Value = serde_json::from_str(&input.config_json).expect("config JSON");
        config["operations"]["summarize"]["mcp"]["maxToolCalls"] = json!(2);
        config["operations"]["summarize"]["mcp"]["tools"] = json!([
            {
                "connectionId": "00000000-0000-4000-8000-000000000201",
                "toolName": "search.read"
            },
            {
                "connectionId": "00000000-0000-4000-8000-000000000202",
                "toolName": "search.related"
            }
        ]);
        input.config_json = canonical_json(config, 256 * 1024).expect("two-tool config");
        input.tool_bindings.push(ToolBinding {
            binding_id: "binding-b".to_owned(),
            connection_id: "00000000-0000-4000-8000-000000000202".to_owned(),
            tool_name: "search.related".to_owned(),
            display_label: "Related".to_owned(),
            description: "Untrusted related-content description".to_owned(),
            input_schema_json: r#"{"additionalProperties":false,"properties":{"query":{"type":"string"}},"type":"object"}"#.to_owned(),
            input_schema_digest: "b".repeat(64),
        });
        input
    }

    fn summary_output() -> String {
        canonical_json(
            json!({
                "schemaVersion": 1,
                "sourceLanguage": "en",
                "summary": "Summary",
                "bullets": [],
                "conclusion": null,
            }),
            MAX_CANONICAL_INPUT_BYTES,
        )
        .expect("summary output")
    }

    fn translation_output() -> String {
        canonical_json(
            json!({
                "schemaVersion": 1,
                "detectedSourceLanguage": "en",
                "targetLocale": "zh-CN",
                "title": "标题",
                "bodyMarkdown": "正文",
            }),
            MAX_CANONICAL_INPUT_BYTES,
        )
        .expect("translation output")
    }

    struct FakeHost {
        ai_responses: VecDeque<Result<AiResponse, AiError>>,
        mcp_responses: VecDeque<Result<McpResponse, McpError>>,
        ai_requests: Vec<AiRequest>,
        mcp_requests: Vec<McpRequest>,
    }

    impl FakeHost {
        fn new(responses: impl IntoIterator<Item = Result<AiResponse, AiError>>) -> Self {
            Self {
                ai_responses: responses.into_iter().collect(),
                mcp_responses: VecDeque::new(),
                ai_requests: Vec::new(),
                mcp_requests: Vec::new(),
            }
        }
    }

    impl HostCapabilities for FakeHost {
        fn generate(&mut self, request: AiRequest) -> Result<AiResponse, AiError> {
            self.ai_requests.push(request);
            self.ai_responses.pop_front().expect("AI fixture response")
        }

        fn call_tool(&mut self, request: McpRequest) -> Result<McpResponse, McpError> {
            self.mcp_requests.push(request);
            self.mcp_responses
                .pop_front()
                .expect("MCP fixture response")
        }
    }
}
