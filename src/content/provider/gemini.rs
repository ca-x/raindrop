use http::{HeaderName, HeaderValue, header};
use secrecy::SecretString;
use serde::Deserialize;
use serde_json::json;
use url::Url;

use super::{
    EncodedProviderRequest, FinishReason, ProviderAdapterError, ProviderAdapterErrorKind,
    ProviderHeader, ProviderKind, StructuredGenerationRequest, StructuredGenerationResponse,
    TokenUsage, validation,
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

pub(super) fn decode(
    requested_model: &str,
    body: &[u8],
) -> Result<StructuredGenerationResponse, ProviderAdapterError> {
    let provider = ProviderKind::GoogleGemini;
    let response: GeminiResponse = validation::parse_response(provider, body)?;
    if response.candidates.len() != 1 {
        return Err(ProviderAdapterError::for_provider(
            provider,
            ProviderAdapterErrorKind::MalformedResponse,
        ));
    }
    let candidate = &response.candidates[0];
    let output_text = validation::exactly_one_text(
        provider,
        candidate
            .content
            .parts
            .iter()
            .filter_map(|part| part.text.as_deref()),
    )?;
    let finish_reason = match candidate.finish_reason.as_str() {
        "STOP" => FinishReason::Stop,
        "MAX_TOKENS" => FinishReason::Length,
        "SAFETY" | "RECITATION" | "PROHIBITED_CONTENT" | "SPII" | "BLOCKLIST" => {
            FinishReason::ContentFilter
        }
        "MALFORMED_FUNCTION_CALL" | "UNEXPECTED_TOOL_CALL" => FinishReason::ToolCall,
        _ => FinishReason::Other,
    };
    validation::structured_response(
        provider,
        requested_model,
        response.model_version.as_deref(),
        output_text,
        finish_reason,
        response
            .usage_metadata
            .map_or_else(TokenUsage::default, |usage| TokenUsage {
                input_tokens: usage.prompt_token_count,
                output_tokens: usage.candidates_token_count,
            }),
    )
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GeminiResponse {
    model_version: Option<String>,
    candidates: Vec<GeminiCandidate>,
    usage_metadata: Option<GeminiUsage>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GeminiCandidate {
    content: GeminiContent,
    finish_reason: String,
}

#[derive(Deserialize)]
struct GeminiContent {
    parts: Vec<GeminiPart>,
}

#[derive(Deserialize)]
struct GeminiPart {
    text: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GeminiUsage {
    prompt_token_count: Option<u64>,
    candidates_token_count: Option<u64>,
}
