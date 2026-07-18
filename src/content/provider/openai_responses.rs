use http::{HeaderName, HeaderValue, header};
use secrecy::{ExposeSecret, SecretString};
use serde::Deserialize;
use serde_json::json;

use super::{
    EncodedProviderRequest, FinishReason, ProviderAdapterError, ProviderHeader, ProviderKind,
    StructuredGenerationRequest, StructuredGenerationResponse, TokenUsage, validation,
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

pub(super) fn decode(
    requested_model: &str,
    body: &[u8],
) -> Result<StructuredGenerationResponse, ProviderAdapterError> {
    let provider = ProviderKind::OpenAiResponses;
    let response: OpenAiResponsesResponse = validation::parse_response(provider, body)?;
    let output_text = validation::exactly_one_text(
        provider,
        response
            .output
            .iter()
            .filter(|output| output.kind == "message")
            .flat_map(|output| output.content.iter())
            .filter(|content| content.kind == "output_text")
            .filter_map(|content| content.text.as_deref()),
    )?;
    let finish_reason = match response.status.as_str() {
        "completed" => FinishReason::Stop,
        "incomplete" => match response
            .incomplete_details
            .as_ref()
            .map(|details| details.reason.as_str())
        {
            Some("max_output_tokens") => FinishReason::Length,
            Some("content_filter") => FinishReason::ContentFilter,
            Some("tool_call") => FinishReason::ToolCall,
            _ => FinishReason::Other,
        },
        _ => FinishReason::Other,
    };
    validation::structured_response(
        provider,
        requested_model,
        response.model.as_deref(),
        output_text,
        finish_reason,
        response
            .usage
            .map_or_else(TokenUsage::default, |usage| TokenUsage {
                input_tokens: usage.input_tokens,
                output_tokens: usage.output_tokens,
            }),
    )
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

#[derive(Deserialize)]
struct OpenAiResponsesResponse {
    model: Option<String>,
    status: String,
    output: Vec<OpenAiOutput>,
    incomplete_details: Option<OpenAiIncompleteDetails>,
    usage: Option<OpenAiResponsesUsage>,
}

#[derive(Deserialize)]
struct OpenAiOutput {
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    content: Vec<OpenAiOutputContent>,
}

#[derive(Deserialize)]
struct OpenAiOutputContent {
    #[serde(rename = "type")]
    kind: String,
    text: Option<String>,
}

#[derive(Deserialize)]
struct OpenAiIncompleteDetails {
    reason: String,
}

#[derive(Deserialize)]
struct OpenAiResponsesUsage {
    input_tokens: Option<u64>,
    output_tokens: Option<u64>,
}
