use futures_util::FutureExt;
use raindrop_ai_content::AiContentService;
use serde_json::json;
use url::Url;

use super::{
    CapabilitySession, NativePlugin, PluginExecutionFailure, PluginExecutionSuccess,
    PluginFailureCode, PluginRuntime, PluginRuntimeError, PluginRuntimeErrorKind, bindings::types,
    host::NativeHost,
};
use crate::plugins::{
    AiContentConfig, LifecycleEvent, SummaryArtifact, TranslationArtifact,
    json::{
        canonical_json, normalize_locale, parse_unique_json, validate_lower_hex_hash,
        validate_text, validate_visible_ascii,
    },
};

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
        compiled: &NativePlugin,
        session: CapabilitySession,
        request: types::OperationRequest,
    ) -> Result<types::ArtifactCandidate, PluginRuntimeError> {
        self.execute_detailed(compiled, session, request)
            .await
            .map(PluginExecutionSuccess::into_artifact)
            .map_err(PluginExecutionFailure::into_error)
    }

    pub async fn execute_detailed(
        &self,
        compiled: &NativePlugin,
        mut session: CapabilitySession,
        request: types::OperationRequest,
    ) -> Result<PluginExecutionSuccess, PluginExecutionFailure> {
        let result = self.execute_native(compiled, &mut session, &request).await;
        match result {
            Ok(artifact) => Ok(PluginExecutionSuccess::new(
                artifact,
                session.usage().clone(),
            )),
            Err(error) => Err(PluginExecutionFailure::new(
                error,
                session.usage().clone(),
                session.failure_hint(),
            )),
        }
    }

    async fn execute_native(
        &self,
        compiled: &NativePlugin,
        session: &mut CapabilitySession,
        request: &types::OperationRequest,
    ) -> Result<types::ArtifactCandidate, PluginRuntimeError> {
        validate_operation_request(compiled, session, request)?;
        session.suspend();
        let service = self.service().await?;
        validate_descriptor(compiled, AiContentService::descriptor())?;
        session.activate();
        let deadline = session.deadline();
        let mut host = NativeHost {
            session,
            error: None,
        };
        let result = {
            let execution =
                std::panic::AssertUnwindSafe(service.execute(&mut host, request.clone()))
                    .catch_unwind();
            tokio::select! {
                biased;
                () = service.cancelled() => return Err(PluginRuntimeError::new(PluginRuntimeErrorKind::RuntimeUnavailable)),
                result = tokio::time::timeout_at(deadline, execution) => result,
            }
        };
        if let Some(error) = host.error {
            return Err(error);
        }
        let artifact = result
            .map_err(|_| PluginRuntimeError::new(PluginRuntimeErrorKind::BrokerTimeout))?
            .map_err(|_| PluginRuntimeError::new(PluginRuntimeErrorKind::NativeFailure))?
            .map_err(map_plugin_error)?;
        validate_artifact(request, artifact)
    }

    pub async fn on_event(
        &self,
        compiled: &NativePlugin,
        session: CapabilitySession,
        request: types::LifecycleRequest,
    ) -> Result<types::EventOutcome, PluginRuntimeError> {
        validate_lifecycle_request(compiled, &session, &request)?;
        let service = self.service().await?;
        validate_descriptor(compiled, AiContentService::descriptor())?;
        let outcome =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| service.on_event(request)))
                .map_err(|_| PluginRuntimeError::new(PluginRuntimeErrorKind::NativeFailure))?
                .map_err(map_plugin_error)?;
        validate_event_outcome(outcome)
    }
}

fn validate_descriptor(
    compiled: &NativePlugin,
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
    compiled: &NativePlugin,
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
    let content_valid = valid_optional_title(&request.entry.title)
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

fn valid_optional_title(value: &str) -> bool {
    value.len() <= 64 * 1024
        && !value
            .chars()
            .any(|character| character.is_control() && !matches!(character, '\n' | '\r' | '\t'))
}

fn validate_lifecycle_request(
    compiled: &NativePlugin,
    session: &CapabilitySession,
    request: &types::LifecycleRequest,
) -> Result<(), PluginRuntimeError> {
    let event = &request.event;
    session.validate_lifecycle_context(&request.invocation_id, &event.user_scope.subject)?;
    let config = AiContentConfig::parse(request.config_json.as_bytes())
        .map_err(|_| PluginRuntimeError::new(PluginRuntimeErrorKind::InvalidInvocation))?;
    let identity_valid = request.plugin_key == compiled.plugin_key()
        && request.plugin_version == compiled.version()
        && request.component_digest == compiled.component_digest()
        && request.config_json == config.canonical_json()
        && request.config_hash == config.config_hash();
    if !identity_valid
        || request.config_json.len() > MAX_CONFIG_BYTES
        || lifecycle_request_size(request) > MAX_REQUEST_BYTES
        || event.event_type != "feed.refresh.persisted"
        || event.schema_version != 1
    {
        return Err(PluginRuntimeError::new(
            PluginRuntimeErrorKind::InvalidInvocation,
        ));
    }
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

fn lifecycle_request_size(request: &types::LifecycleRequest) -> usize {
    128 + request.invocation_id.len()
        + request.plugin_key.len()
        + request.plugin_version.len()
        + request.component_digest.len()
        + request.config_json.len()
        + request.config_hash.len()
        + request.event.event_id.len()
        + request.event.event_type.len()
        + request.event.refresh_id.len()
        + request.event.occurred_at.len()
        + request.event.idempotency_key.len()
        + request.event.user_scope.subject.len()
        + request.event.context_json.len()
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
            .map(|binding| {
                binding.binding_id.len()
                    + binding.connection_id.len()
                    + binding.tool_name.len()
                    + binding.display_label.len()
                    + binding.description.len()
                    + binding.input_schema_json.len()
                    + binding.input_schema_digest.len()
                    + 48
            })
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

fn map_plugin_error(error: types::PluginError) -> PluginRuntimeError {
    let failure_code = PluginFailureCode::from_message_key(&error.message_key);
    PluginRuntimeError::new(match error.code {
        types::PluginErrorCode::Disabled | types::PluginErrorCode::CapabilityDenied => {
            PluginRuntimeErrorKind::CapabilityDenied
        }
        types::PluginErrorCode::InvalidRequest
        | types::PluginErrorCode::ConfigInvalid
        | types::PluginErrorCode::OutputInvalid => PluginRuntimeErrorKind::InvalidInvocation,
        types::PluginErrorCode::OutputTooLarge => PluginRuntimeErrorKind::OutputTooLarge,
        types::PluginErrorCode::DeadlineExceeded => PluginRuntimeErrorKind::DeadlineExceeded,
        types::PluginErrorCode::BudgetExhausted | types::PluginErrorCode::HostFailure => {
            PluginRuntimeErrorKind::NativeFailure
        }
    })
    .with_failure_code(failure_code)
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn operation_request_size_counts_complete_tool_descriptors_at_the_exact_limit() {
        let mut request = operation_request_fixture();
        request.tool_bindings.push(types::ToolBinding {
            binding_id: "binding-1".to_owned(),
            connection_id: "00000000-0000-4000-8000-000000000001".to_owned(),
            tool_name: "search.read".to_owned(),
            display_label: "Search".to_owned(),
            description: "Untrusted description".to_owned(),
            input_schema_json: r#"{"type":"object"}"#.to_owned(),
            input_schema_digest: "d".repeat(64),
        });

        let descriptor_bytes = request.tool_bindings[0].binding_id.len()
            + request.tool_bindings[0].connection_id.len()
            + request.tool_bindings[0].tool_name.len()
            + request.tool_bindings[0].display_label.len()
            + request.tool_bindings[0].description.len()
            + request.tool_bindings[0].input_schema_json.len()
            + request.tool_bindings[0].input_schema_digest.len()
            + 48;
        let with_descriptor = operation_request_size(&request);
        request.tool_bindings.clear();
        assert_eq!(
            with_descriptor - operation_request_size(&request),
            descriptor_bytes
        );

        let current = operation_request_size(&request);
        assert!(current < MAX_REQUEST_BYTES);
        request
            .entry
            .text
            .push_str(&"x".repeat(MAX_REQUEST_BYTES - current));
        assert_eq!(operation_request_size(&request), MAX_REQUEST_BYTES);
        request.entry.text.push('x');
        assert_eq!(operation_request_size(&request), MAX_REQUEST_BYTES + 1);
    }

    #[test]
    fn lifecycle_request_size_counts_the_wrapper_at_the_exact_limit() {
        let mut request = types::LifecycleRequest {
            invocation_id: "invocation-1".to_owned(),
            plugin_key: "raindrop.ai-content".to_owned(),
            plugin_version: "1.0.0".to_owned(),
            component_digest: "a".repeat(64),
            config_json: "{}".to_owned(),
            config_hash: "b".repeat(64),
            event: types::LifecycleEvent {
                event_id: "event-1".to_owned(),
                event_type: "feed.refresh.persisted".to_owned(),
                schema_version: 1,
                refresh_id: "refresh-1".to_owned(),
                sequence: 1,
                occurred_at: "2026-07-19T00:00:00Z".to_owned(),
                idempotency_key: "refresh:refresh-1:persisted:v1".to_owned(),
                user_scope: types::UserScope {
                    subject: "user-1".to_owned(),
                },
                context_json: "{}".to_owned(),
            },
        };
        let wrapper_bytes = 128
            + request.invocation_id.len()
            + request.plugin_key.len()
            + request.plugin_version.len()
            + request.component_digest.len()
            + request.config_json.len()
            + request.config_hash.len()
            + request.event.event_id.len()
            + request.event.event_type.len()
            + request.event.refresh_id.len()
            + request.event.occurred_at.len()
            + request.event.idempotency_key.len()
            + request.event.user_scope.subject.len()
            + request.event.context_json.len();
        assert_eq!(lifecycle_request_size(&request), wrapper_bytes);

        assert!(wrapper_bytes < MAX_REQUEST_BYTES);
        request
            .event
            .context_json
            .push_str(&"x".repeat(MAX_REQUEST_BYTES - wrapper_bytes));
        assert_eq!(lifecycle_request_size(&request), MAX_REQUEST_BYTES);
        request.event.context_json.push('x');
        assert_eq!(lifecycle_request_size(&request), MAX_REQUEST_BYTES + 1);
    }

    fn operation_request_fixture() -> types::OperationRequest {
        types::OperationRequest {
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
            operation: types::Operation::Summarize,
            target_locale: None,
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
        }
    }
}
