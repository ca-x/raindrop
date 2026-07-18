use http::{HeaderName, HeaderValue, header};
use secrecy::SecretString;
use serde_json::json;

use super::{
    EncodedProviderRequest, ProviderAdapterError, ProviderHeader, ProviderKind,
    StructuredGenerationRequest, validation,
};

pub(super) fn encode(
    request: &StructuredGenerationRequest,
    credential: SecretString,
) -> Result<EncodedProviderRequest, ProviderAdapterError> {
    let provider = ProviderKind::AnthropicMessages;
    let input = validation::canonical_json(provider, &request.untrusted_input)?;
    let headers = vec![
        ProviderHeader::public(
            header::CONTENT_TYPE,
            HeaderValue::from_static("application/json"),
        ),
        ProviderHeader::public(header::ACCEPT, HeaderValue::from_static("application/json")),
        ProviderHeader::secret(HeaderName::from_static("x-api-key"), credential),
        ProviderHeader::public(
            HeaderName::from_static("anthropic-version"),
            HeaderValue::from_static("2023-06-01"),
        ),
        validation::public_header(
            provider,
            HeaderName::from_static("idempotency-key"),
            &request.idempotency_key,
        )?,
    ];
    let body = json!({
        "model": request.model,
        "max_tokens": request.max_output_tokens,
        "system": request.system_instruction,
        "messages": [{
            "role": "user",
            "content": [{ "type": "text", "text": input }]
        }],
        "output_config": {
            "format": {
                "type": "json_schema",
                "schema": request.output_schema.schema
            }
        }
    });
    validation::encode_request_body(provider, "/v1/messages".to_owned(), headers, &body)
}
