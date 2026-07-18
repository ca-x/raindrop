use secrecy::SecretString;
use serde::Deserialize;
use serde_json::json;

use super::{
    EncodedProviderRequest, FinishReason, ProviderAdapterError, ProviderAdapterErrorKind,
    ProviderKind, StructuredGenerationRequest, StructuredGenerationResponse, TokenUsage,
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

pub(super) fn decode(
    requested_model: &str,
    body: &[u8],
) -> Result<StructuredGenerationResponse, ProviderAdapterError> {
    let provider = ProviderKind::OpenAiChatCompletions;
    let response: OpenAiChatResponse = validation::parse_response(provider, body)?;
    if response.choices.len() != 1 {
        return Err(ProviderAdapterError::for_provider(
            provider,
            ProviderAdapterErrorKind::MalformedResponse,
        ));
    }
    let choice = &response.choices[0];
    let output_text = validation::exactly_one_text(provider, [choice.message.content.as_str()])?;
    let finish_reason = match choice.finish_reason.as_str() {
        "stop" => FinishReason::Stop,
        "length" => FinishReason::Length,
        "content_filter" => FinishReason::ContentFilter,
        "tool_calls" | "function_call" => FinishReason::ToolCall,
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
                input_tokens: usage.prompt_tokens,
                output_tokens: usage.completion_tokens,
            }),
    )
}

#[derive(Deserialize)]
struct OpenAiChatResponse {
    model: Option<String>,
    choices: Vec<OpenAiChatChoice>,
    usage: Option<OpenAiChatUsage>,
}

#[derive(Deserialize)]
struct OpenAiChatChoice {
    message: OpenAiChatMessage,
    finish_reason: String,
}

#[derive(Deserialize)]
struct OpenAiChatMessage {
    content: String,
}

#[derive(Deserialize)]
struct OpenAiChatUsage {
    prompt_tokens: Option<u64>,
    completion_tokens: Option<u64>,
}
