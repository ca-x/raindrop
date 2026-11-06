use std::{fmt, sync::Arc, time::Duration};

use raindrop_ai_content::question::{QuestionContext, QuestionTurn, valid_question};
use time::OffsetDateTime;
use tokio::time::Instant;
use uuid::Uuid;

use crate::{
    content::jobs::{ArtifactKind, ContentJobOperation, ContentRepositoryErrorKind},
    plugins::{
        PluginSystemState, SummaryArtifact,
        runtime::{
            BrokerInvocationContext, CapabilitySession, CapabilitySessionConfig, DenyMcpBroker,
            PluginFailureCode, PluginRuntimeErrorKind, bindings::types,
        },
    },
};

use super::{ContentInvocationInput, OFFICIAL_AI_PLUGIN_KEY, OfficialAiProcessor};

pub struct ArticleQuestionRequest {
    pub user_id: String,
    pub entry_id: String,
    pub summary_job_id: String,
    pub question: String,
    pub response_locale: String,
    pub history: Vec<QuestionTurn>,
}

pub struct ArticleQuestionAnswer {
    pub answer_markdown: String,
    pub provider_label: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ArticleQuestionError {
    InvalidInput,
    NotFound,
    ContextChanged,
    Unavailable,
    TooLarge,
    RateLimited,
    Timeout,
    OutputInvalid,
}

impl fmt::Display for ArticleQuestionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "article question failed: {self:?}")
    }
}
impl std::error::Error for ArticleQuestionError {}

impl OfficialAiProcessor {
    pub(super) async fn answer_question(
        &self,
        request: ArticleQuestionRequest,
    ) -> Result<ArticleQuestionAnswer, ArticleQuestionError> {
        use ArticleQuestionError as E;
        if !valid_question(
            &request.question,
            &request.response_locale,
            &request.history,
        ) {
            return Err(E::InvalidInput);
        }
        let entry = self
            .content_repository
            .get_execution_entry_for_user(&request.user_id, &request.entry_id)
            .await
            .map_err(|error| {
                if error.kind() == ContentRepositoryErrorKind::NotFound {
                    E::NotFound
                } else {
                    E::Unavailable
                }
            })?;
        let result = self
            .content_repository
            .get_result(&request.user_id, &request.summary_job_id)
            .await
            .map_err(|error| {
                if error.kind() == ContentRepositoryErrorKind::NotFound {
                    E::NotFound
                } else {
                    E::Unavailable
                }
            })?;
        let artifact = result.artifact();
        if artifact.identity().entry_id() != entry.entry_id()
            || artifact.identity().kind() != ArtifactKind::AiSummary
        {
            return Err(E::InvalidInput);
        }
        if artifact.identity().entry_content_hash() != entry.content_hash() {
            return Err(E::ContextChanged);
        }
        SummaryArtifact::parse(artifact.payload_json().as_bytes()).map_err(|_| E::Unavailable)?;
        let installation = self
            .plugin_repository
            .get_installation(OFFICIAL_AI_PLUGIN_KEY)
            .await
            .map_err(|_| E::Unavailable)?;
        if installation.system_state() != PluginSystemState::Enabled
            || installation.component_digest() != self.compiled.component_digest()
            || installation.version() != self.compiled.version()
            || installation.abi_version() != self.compiled.abi_version()
        {
            return Err(E::Unavailable);
        }
        let stored = self
            .plugin_repository
            .get_ai_config(OFFICIAL_AI_PLUGIN_KEY, &request.user_id)
            .await
            .map_err(|_| E::Unavailable)?
            .ok_or(E::Unavailable)?;
        let config = stored.config();
        if !stored.is_enabled()
            || !config.summarize_enabled()
            || stored.plugin_id() != installation.id()
            || stored.owner_user_id() != request.user_id
        {
            return Err(E::Unavailable);
        }
        let binding = self
            .provider_repository
            .load_enabled_binding(config.summarize_provider_id(), &request.user_id)
            .await
            .map_err(|_| E::Unavailable)?;
        let metadata = binding.metadata();
        let policy = metadata.policy();
        let provider_label = metadata.display_name().to_owned();
        let output_tokens = config
            .summarize_max_output_tokens()
            .min(policy.max_output_tokens_per_request);
        let id = Uuid::new_v4().to_string();
        let invocation_id = format!("question:{id}");
        let deadline = Instant::now() + Duration::from_secs(90);
        let deadline_unix_ms =
            u64::try_from(OffsetDateTime::now_utc().unix_timestamp_nanos() / 1_000_000)
                .map_err(|_| E::Unavailable)?
                .saturating_add(90_000);
        let invocation = BrokerInvocationContext {
            invocation_id: invocation_id.clone(),
            job_id: id.clone(),
            user_subject: request.user_id.clone(),
            call_chain_id: id.clone(),
            operation: types::Operation::Summarize,
            trigger: types::Trigger::ReaderSidecar,
            remaining_depth: 0,
            expected_provider_kind: metadata.kind().as_storage().to_owned(),
            expected_provider_model: metadata.model().to_owned(),
            expected_provider_revision: metadata.revision(),
        };
        let provider_binding_id = metadata.id().to_owned();
        drop(binding);
        let session = CapabilitySession::new(
            CapabilitySessionConfig {
                invocation,
                provider_binding_id: provider_binding_id.clone(),
                tool_bindings: Vec::new(),
                remaining_provider_requests: 1,
                remaining_mcp_calls: 0,
                remaining_input_tokens: policy.max_input_tokens_per_request,
                remaining_output_tokens: output_tokens,
                max_output_tokens_per_request: output_tokens,
                remaining_cost_micros: 250_000,
                deadline_unix_ms,
                deadline,
            },
            Arc::clone(&self.ai_broker),
            Arc::new(DenyMcpBroker),
        )
        .map_err(|_| E::Unavailable)?;
        let input = ContentInvocationInput::new(&entry, ContentJobOperation::Summarize, None)
            .map_err(|_| E::TooLarge)?;
        let operation = types::OperationRequest {
            invocation_id,
            job_id: id.clone(),
            idempotency_key: id.clone(),
            plugin_key: self.compiled.plugin_key().to_owned(),
            plugin_version: self.compiled.version().to_owned(),
            component_digest: self.compiled.component_digest().to_owned(),
            user_scope: types::UserScope {
                subject: request.user_id,
            },
            trigger: types::Trigger::ReaderSidecar,
            operation: types::Operation::Summarize,
            target_locale: None,
            entry: input.to_wit_entry(),
            config_json: stored.canonical_json().to_owned(),
            config_hash: stored.config_hash().to_owned(),
            provider_binding_id,
            tool_bindings: Vec::new(),
            call_chain_id: id,
            budget: types::InvocationBudget {
                remaining_depth: 0,
                deadline_unix_ms,
                remaining_provider_requests: 1,
                remaining_mcp_calls: 0,
                remaining_input_tokens: policy.max_input_tokens_per_request,
                remaining_output_tokens: output_tokens,
                max_output_tokens_per_request: output_tokens,
                remaining_cost_micros: 250_000,
            },
        };
        let context = QuestionContext {
            question: request.question,
            response_locale: request.response_locale,
            history: request.history,
            summary_json: artifact.payload_json().to_owned(),
        };
        let answer = self
            .runtime
            .answer_question(&self.compiled, session, operation, context)
            .await
            .map_err(|error| match (error.kind(), error.failure_code()) {
                (
                    PluginRuntimeErrorKind::BrokerTimeout
                    | PluginRuntimeErrorKind::DeadlineExceeded,
                    _,
                )
                | (_, Some(PluginFailureCode::ProviderTimeout)) => E::Timeout,
                (_, Some(PluginFailureCode::BudgetExhausted)) => E::TooLarge,
                (_, Some(PluginFailureCode::ProviderRateLimited)) => E::RateLimited,
                (
                    _,
                    Some(
                        PluginFailureCode::OutputInvalid | PluginFailureCode::ProviderOutputInvalid,
                    ),
                ) => E::OutputInvalid,
                _ => E::Unavailable,
            })?;
        Ok(ArticleQuestionAnswer {
            answer_markdown: answer.answer_markdown,
            provider_label,
        })
    }
}
