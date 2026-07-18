use secrecy::SecretString;
use serde_json::json;

use super::{
    EncodedProviderRequest, ProviderAdapterError, ProviderKind, StructuredGenerationRequest,
    openai_responses::openai_headers, validation,
};

pub(super) fn encode(
    request: &StructuredGenerationRequest,
    credential: SecretString,
) -> Result<EncodedProviderRequest, ProviderAdapterError> {
    let provider = ProviderKind::OpenAiChatCompletions;
    let input = validation::canonical_json(provider, &request.untrusted_input)?;
    let headers = openai_headers(provider, &request.idempotency_key, credential)?;
    let body = json!({
        "model": request.model,
        "messages": [
            { "role": "system", "content": request.system_instruction },
            { "role": "user", "content": input }
        ],
        "max_completion_tokens": request.max_output_tokens,
        "response_format": {
            "type": "json_schema",
            "json_schema": {
                "name": request.output_schema.name,
                "schema": request.output_schema.schema,
                "strict": true
            }
        }
    });
    validation::encode_request_body(provider, "/v1/chat/completions".to_owned(), headers, &body)
}
