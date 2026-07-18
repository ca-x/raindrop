use http::{HeaderName, HeaderValue, header};
use secrecy::{ExposeSecret, SecretString};
use serde_json::json;

use super::{
    EncodedProviderRequest, ProviderAdapterError, ProviderHeader, ProviderKind,
    StructuredGenerationRequest, validation,
};

pub(super) fn encode(
    request: &StructuredGenerationRequest,
    credential: SecretString,
) -> Result<EncodedProviderRequest, ProviderAdapterError> {
    let provider = ProviderKind::OpenAiResponses;
    let input = validation::canonical_json(provider, &request.untrusted_input)?;
    let headers = openai_headers(provider, &request.idempotency_key, credential)?;
    let body = json!({
        "model": request.model,
        "instructions": request.system_instruction,
        "input": [{
            "role": "user",
            "content": [{ "type": "input_text", "text": input }]
        }],
        "max_output_tokens": request.max_output_tokens,
        "text": {
            "format": {
                "type": "json_schema",
                "name": request.output_schema.name,
                "schema": request.output_schema.schema,
                "strict": true
            }
        }
    });
    validation::encode_request_body(provider, "/v1/responses".to_owned(), headers, &body)
}

pub(super) fn openai_headers(
    provider: ProviderKind,
    idempotency_key: &str,
    credential: SecretString,
) -> Result<Vec<ProviderHeader>, ProviderAdapterError> {
    Ok(vec![
        ProviderHeader::public(
            header::CONTENT_TYPE,
            HeaderValue::from_static("application/json"),
        ),
        ProviderHeader::public(header::ACCEPT, HeaderValue::from_static("application/json")),
        ProviderHeader::secret(
            header::AUTHORIZATION,
            SecretString::from(format!("Bearer {}", credential.expose_secret())),
        ),
        validation::public_header(
            provider,
            HeaderName::from_static("idempotency-key"),
            idempotency_key,
        )?,
    ])
}
