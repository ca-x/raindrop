use std::{fmt, sync::Arc, time::Duration};

use async_trait::async_trait;
use blake3::Hasher;
use time::OffsetDateTime;

use crate::{
    content::provider::{
        FinishReason, OutputSchema, ProviderCallError, ProviderCallErrorKind, ProviderClient,
        ProviderCoreError, ProviderCoreErrorKind, ProviderPolicy, ProviderRepository,
        ProviderTransport, StructuredGenerationRequest,
    },
    plugins::runtime::{
        AiBrokerError, AiBrokerErrorKind, AiBrokerRequest, AiBrokerResponse, AiCapabilityBroker,
        AiFinishReason, BrokerInvocationContext,
    },
};

use super::{
    admission::{ProviderAdmissionController, ProviderAdmissionError},
    cost::estimate_cost_micros,
    schema::{OfficialSchema, canonical_input},
};

const IDEMPOTENCY_CONTEXT: &str = "raindrop.ai-provider-call-idempotency.v1";

pub struct ProviderAiBroker<T> {
    repository: Arc<ProviderRepository>,
    client: Arc<ProviderClient<T>>,
    admission: Arc<ProviderAdmissionController>,
}

impl<T> ProviderAiBroker<T> {
    #[must_use]
    pub fn new(repository: Arc<ProviderRepository>, client: Arc<ProviderClient<T>>) -> Self {
        Self {
            repository,
            client,
            admission: Arc::new(ProviderAdmissionController::default()),
        }
    }
}

impl<T> fmt::Debug for ProviderAiBroker<T> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProviderAiBroker")
            .finish_non_exhaustive()
    }
}

#[async_trait]
impl<T> AiCapabilityBroker for ProviderAiBroker<T>
where
    T: ProviderTransport,
{
    async fn generate_structured(
        &self,
        context: &BrokerInvocationContext,
        request: AiBrokerRequest,
    ) -> Result<AiBrokerResponse, AiBrokerError> {
        if request.provider_request_ordinal == 0 || request.operation != context.operation {
            return Err(invalid_request());
        }
        let schema = OfficialSchema::validate_request(
            request.operation,
            &request.output_schema_id,
            &request.output_schema_json,
        )?;
        let untrusted_input = canonical_input(&request.untrusted_input_json)?;
        let binding = self
            .repository
            .load_enabled_binding(&request.provider_binding_id, &context.user_subject)
            .await
            .map_err(map_repository_error)?;
        let metadata = binding.metadata();
        if metadata.kind().as_storage() != context.expected_provider_kind
            || metadata.model() != context.expected_provider_model
            || metadata.revision() != context.expected_provider_revision
        {
            return Err(AiBrokerError::new(
                AiBrokerErrorKind::CapabilityDenied,
                false,
                None,
            ));
        }
        let policy = metadata.policy();
        validate_token_policy(policy, &request)?;
        let maximum_cost = estimate_cost_micros(
            policy,
            u64::from(request.max_input_tokens),
            u64::from(request.max_output_tokens),
        );
        validate_cost_ceiling(policy, request.max_cost_micros, maximum_cost)?;
        let _admission = self
            .admission
            .acquire(metadata.id(), metadata.revision(), policy)
            .await
            .map_err(map_admission_error)?;
        let provider_request = StructuredGenerationRequest {
            model: metadata.model().to_owned(),
            system_instruction: request.system_instruction,
            untrusted_input: untrusted_input.clone(),
            output_schema: OutputSchema {
                name: schema.schema_name().to_owned(),
                schema: schema.schema_value()?,
            },
            max_output_tokens: request.max_output_tokens,
            idempotency_key: provider_call_idempotency_key(
                &context.job_id,
                request.provider_request_ordinal,
            ),
        };
        let response = tokio::time::timeout(
            request.timeout,
            self.client.generate(&binding, &provider_request),
        )
        .await
        .map_err(|_| AiBrokerError::new(AiBrokerErrorKind::Timeout, true, None))?
        .map_err(map_provider_call_error)?;
        let output_json = schema.validate_output(response.output, &untrusted_input)?;
        let estimated_cost_micros = response
            .usage
            .input_tokens
            .zip(response.usage.output_tokens)
            .and_then(|(input, output)| estimate_cost_micros(policy, input, output))
            .or(maximum_cost);
        validate_cost_ceiling(policy, request.max_cost_micros, estimated_cost_micros)?;

        Ok(AiBrokerResponse {
            output_json,
            finish_reason: map_finish_reason(response.finish_reason),
            input_tokens: response.usage.input_tokens,
            output_tokens: response.usage.output_tokens,
            model_label: response.model_label,
            estimated_cost_micros,
        })
    }
}

fn validate_token_policy(
    policy: ProviderPolicy,
    request: &AiBrokerRequest,
) -> Result<(), AiBrokerError> {
    if request.max_input_tokens == 0
        || request.max_input_tokens > policy.max_input_tokens_per_request
        || request.max_output_tokens == 0
        || request.max_output_tokens > policy.max_output_tokens_per_request
    {
        Err(AiBrokerError::new(
            AiBrokerErrorKind::QuotaExceeded,
            false,
            None,
        ))
    } else {
        Ok(())
    }
}

fn validate_cost_ceiling(
    policy: ProviderPolicy,
    invocation_ceiling: u64,
    estimated: Option<u64>,
) -> Result<(), AiBrokerError> {
    if estimated.is_some_and(|cost| {
        cost > invocation_ceiling
            || policy
                .max_cost_micros_per_request
                .is_some_and(|provider_ceiling| cost > provider_ceiling)
    }) {
        Err(AiBrokerError::new(
            AiBrokerErrorKind::CostLimitExceeded,
            false,
            None,
        ))
    } else {
        Ok(())
    }
}

fn map_repository_error(error: ProviderCoreError) -> AiBrokerError {
    match error.kind() {
        ProviderCoreErrorKind::InvalidProviderId
        | ProviderCoreErrorKind::InvalidUserId
        | ProviderCoreErrorKind::NotFound => {
            AiBrokerError::new(AiBrokerErrorKind::CapabilityDenied, false, None)
        }
        ProviderCoreErrorKind::Database => {
            AiBrokerError::new(AiBrokerErrorKind::ProviderUnavailable, true, None)
        }
        ProviderCoreErrorKind::ProviderDisabled
        | ProviderCoreErrorKind::SecretUnavailable
        | ProviderCoreErrorKind::CorruptData
        | ProviderCoreErrorKind::InvalidDisplayName
        | ProviderCoreErrorKind::InvalidEndpoint
        | ProviderCoreErrorKind::InvalidModel
        | ProviderCoreErrorKind::InvalidCredential
        | ProviderCoreErrorKind::UnsupportedCapability
        | ProviderCoreErrorKind::InvalidPolicy
        | ProviderCoreErrorKind::InvalidPatch
        | ProviderCoreErrorKind::RevisionConflict => {
            AiBrokerError::new(AiBrokerErrorKind::ProviderUnavailable, false, None)
        }
    }
}

fn map_admission_error(error: ProviderAdmissionError) -> AiBrokerError {
    match error {
        ProviderAdmissionError::ConcurrencyLimited => {
            AiBrokerError::new(AiBrokerErrorKind::RateLimited, true, None)
        }
        ProviderAdmissionError::RateLimited { retry_after } => AiBrokerError::new(
            AiBrokerErrorKind::RateLimited,
            true,
            retry_after.and_then(retry_at_unix_ms),
        ),
    }
}

fn map_provider_call_error(error: ProviderCallError) -> AiBrokerError {
    let retry_at = error.retry_after_at().and_then(offset_unix_ms);
    match error.kind() {
        ProviderCallErrorKind::InvalidRequest
        | ProviderCallErrorKind::RequestTooLarge
        | ProviderCallErrorKind::Rejected => {
            AiBrokerError::new(AiBrokerErrorKind::InvalidRequest, false, retry_at)
        }
        ProviderCallErrorKind::Timeout => {
            AiBrokerError::new(AiBrokerErrorKind::Timeout, true, retry_at)
        }
        ProviderCallErrorKind::RateLimited => {
            AiBrokerError::new(AiBrokerErrorKind::RateLimited, true, retry_at)
        }
        ProviderCallErrorKind::ResponseTooLarge
        | ProviderCallErrorKind::MalformedResponse
        | ProviderCallErrorKind::OutputSchemaInvalid => {
            AiBrokerError::new(AiBrokerErrorKind::OutputSchemaInvalid, false, retry_at)
        }
        ProviderCallErrorKind::Transport | ProviderCallErrorKind::Upstream => {
            AiBrokerError::new(AiBrokerErrorKind::ProviderUnavailable, true, retry_at)
        }
        ProviderCallErrorKind::Authentication => {
            AiBrokerError::new(AiBrokerErrorKind::ProviderUnavailable, false, retry_at)
        }
    }
}

const fn map_finish_reason(reason: FinishReason) -> AiFinishReason {
    match reason {
        FinishReason::Stop => AiFinishReason::Completed,
        FinishReason::Length => AiFinishReason::Length,
        FinishReason::ContentFilter => AiFinishReason::ContentFilter,
        FinishReason::ToolCall => AiFinishReason::ToolPlan,
        FinishReason::Other => AiFinishReason::Unknown,
    }
}

fn provider_call_idempotency_key(job_id: &str, ordinal: u32) -> String {
    let mut hasher = Hasher::new_derive_key(IDEMPOTENCY_CONTEXT);
    frame(&mut hasher, job_id.as_bytes());
    frame(&mut hasher, &ordinal.to_be_bytes());
    hasher.finalize().to_hex().to_string()
}

fn frame(hasher: &mut Hasher, value: &[u8]) {
    hasher.update(&(value.len() as u64).to_be_bytes());
    hasher.update(value);
}

fn retry_at_unix_ms(after: Duration) -> Option<u64> {
    std::time::SystemTime::now()
        .checked_add(after)?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .and_then(|duration| u64::try_from(duration.as_millis()).ok())
}

fn offset_unix_ms(value: OffsetDateTime) -> Option<u64> {
    let nanos = value.unix_timestamp_nanos();
    u64::try_from(nanos.checked_div(1_000_000)?).ok()
}

const fn invalid_request() -> AiBrokerError {
    AiBrokerError::new(AiBrokerErrorKind::InvalidRequest, false, None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_call_idempotency_is_stable_and_domain_separated() {
        let first = provider_call_idempotency_key("job-a", 1);
        assert_eq!(first, provider_call_idempotency_key("job-a", 1));
        assert_ne!(first, provider_call_idempotency_key("job-a", 2));
        assert_ne!(first, provider_call_idempotency_key("job-b", 1));
        assert_eq!(first.len(), 64);
    }

    #[test]
    fn cost_ceiling_checks_provider_and_invocation_limits() {
        let policy = ProviderPolicy {
            max_concurrency: 1,
            requests_per_minute: None,
            max_input_tokens_per_request: 100,
            max_output_tokens_per_request: 100,
            input_cost_micros_per_million_tokens: Some(1),
            output_cost_micros_per_million_tokens: Some(1),
            max_cost_micros_per_request: Some(10),
        };
        assert!(validate_cost_ceiling(policy, 10, Some(10)).is_ok());
        assert_eq!(
            validate_cost_ceiling(policy, 9, Some(10))
                .expect_err("invocation cost ceiling")
                .kind(),
            AiBrokerErrorKind::CostLimitExceeded,
        );
        assert_eq!(
            validate_cost_ceiling(
                ProviderPolicy {
                    max_cost_micros_per_request: Some(9),
                    ..policy
                },
                10,
                Some(10),
            )
            .expect_err("provider cost ceiling")
            .kind(),
            AiBrokerErrorKind::CostLimitExceeded,
        );
    }
}
