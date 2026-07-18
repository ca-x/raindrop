use http::{HeaderName, HeaderValue, header};
use secrecy::SecretString;
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

pub(super) fn decode(
    requested_model: &str,
    body: &[u8],
) -> Result<StructuredGenerationResponse, ProviderAdapterError> {
    let provider = ProviderKind::AnthropicMessages;
    let response: AnthropicResponse = validation::parse_response(provider, body)?;
    let output_text = validation::exactly_one_text(
        provider,
        response
            .content
            .iter()
            .filter(|block| block.kind == "text")
            .filter_map(|block| block.text.as_deref()),
    )?;
    let finish_reason = match response.stop_reason.as_str() {
        "end_turn" | "stop_sequence" => FinishReason::Stop,
        "max_tokens" => FinishReason::Length,
        "refusal" => FinishReason::ContentFilter,
        "tool_use" => FinishReason::ToolCall,
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

#[derive(Deserialize)]
struct AnthropicResponse {
    model: Option<String>,
    content: Vec<AnthropicContentBlock>,
    stop_reason: String,
    usage: Option<AnthropicUsage>,
}

#[derive(Deserialize)]
struct AnthropicContentBlock {
    #[serde(rename = "type")]
    kind: String,
    text: Option<String>,
}

#[derive(Deserialize)]
struct AnthropicUsage {
    input_tokens: Option<u64>,
    output_tokens: Option<u64>,
}
