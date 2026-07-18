use http::{HeaderName, HeaderValue, header};
use secrecy::SecretString;
use serde_json::json;
use url::Url;

use super::{
    EncodedProviderRequest, ProviderAdapterError, ProviderAdapterErrorKind, ProviderHeader,
    ProviderKind, StructuredGenerationRequest, validation,
};

pub(super) fn encode(
    request: &StructuredGenerationRequest,
    credential: SecretString,
) -> Result<EncodedProviderRequest, ProviderAdapterError> {
    let provider = ProviderKind::GoogleGemini;
    let input = validation::canonical_json(provider, &request.untrusted_input)?;
    let headers = vec![
        ProviderHeader::public(
            header::CONTENT_TYPE,
            HeaderValue::from_static("application/json"),
        ),
        ProviderHeader::public(header::ACCEPT, HeaderValue::from_static("application/json")),
        ProviderHeader::secret(HeaderName::from_static("x-goog-api-key"), credential),
        validation::public_header(
            provider,
            HeaderName::from_static("x-goog-request-id"),
            &request.idempotency_key,
        )?,
    ];
    let body = json!({
        "systemInstruction": {
            "parts": [{ "text": request.system_instruction }]
        },
        "contents": [{
            "role": "user",
            "parts": [{ "text": input }]
        }],
        "generationConfig": {
            "maxOutputTokens": request.max_output_tokens,
            "responseMimeType": "application/json",
            "responseJsonSchema": request.output_schema.schema
        }
    });
    validation::encode_request_body(
        provider,
        gemini_path(provider, &request.model)?,
        headers,
        &body,
    )
}

fn gemini_path(provider: ProviderKind, model: &str) -> Result<String, ProviderAdapterError> {
    let mut url = Url::parse("https://provider.invalid/v1beta/models/").map_err(|_| {
        ProviderAdapterError::for_provider(provider, ProviderAdapterErrorKind::InvalidRequest)
    })?;
    url.path_segments_mut()
        .map_err(|_| {
            ProviderAdapterError::for_provider(provider, ProviderAdapterErrorKind::InvalidRequest)
        })?
        .pop_if_empty()
        .push(model);
    Ok(format!("{}:generateContent", url.path()))
}
