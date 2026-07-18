use http::{HeaderName, HeaderValue};
use serde::de::DeserializeOwned;
use serde_json::Value;

use super::{
    EncodedProviderRequest, FinishReason, ProviderAdapterError, ProviderAdapterErrorKind,
    ProviderHeader, ProviderKind, StructuredGenerationRequest, StructuredGenerationResponse,
    TokenUsage,
};

const MAX_MODEL_BYTES: usize = 200;
const MAX_SYSTEM_INSTRUCTION_BYTES: usize = 64 * 1024;
const MAX_UNTRUSTED_INPUT_BYTES: usize = 512 * 1024;
const MAX_SCHEMA_NAME_BYTES: usize = 64;
const MAX_OUTPUT_SCHEMA_BYTES: usize = 64 * 1024;
const MAX_OUTPUT_TOKENS: u32 = 16_384;
const MAX_IDEMPOTENCY_KEY_BYTES: usize = 200;
const MAX_REQUEST_BODY_BYTES: usize = 1024 * 1024;
const MAX_RESPONSE_BODY_BYTES: usize = 2 * 1024 * 1024;
const MAX_OUTPUT_BYTES: usize = 512 * 1024;

pub(super) fn validate_request(
    request: &StructuredGenerationRequest,
) -> Result<(), ProviderAdapterError> {
    validate_bounded_text(&request.model, 1, MAX_MODEL_BYTES, false)?;
    validate_bounded_text(
        &request.system_instruction,
        0,
        MAX_SYSTEM_INSTRUCTION_BYTES,
        true,
    )?;
    validate_json_object(&request.untrusted_input, MAX_UNTRUSTED_INPUT_BYTES)?;
    validate_schema_name(&request.output_schema.name)?;
    validate_json_object(&request.output_schema.schema, MAX_OUTPUT_SCHEMA_BYTES)?;
    if !(1..=MAX_OUTPUT_TOKENS).contains(&request.max_output_tokens) {
        return Err(invalid_request());
    }
    validate_visible_ascii(&request.idempotency_key, 1, MAX_IDEMPOTENCY_KEY_BYTES)?;
    Ok(())
}

pub(super) fn validate_encoded_request(
    path: &str,
    body: &[u8],
) -> Result<(), ProviderAdapterError> {
    if !path.starts_with('/') || path.bytes().any(|byte| byte.is_ascii_control()) {
        return Err(invalid_request());
    }
    if body.len() > MAX_REQUEST_BODY_BYTES {
        return Err(ProviderAdapterError::request(
            ProviderAdapterErrorKind::RequestTooLarge,
        ));
    }
    Ok(())
}

pub(super) fn validate_response_body(
    provider: ProviderKind,
    body: &[u8],
) -> Result<(), ProviderAdapterError> {
    if body.len() > MAX_RESPONSE_BODY_BYTES {
        return Err(ProviderAdapterError::for_provider(
            provider,
            ProviderAdapterErrorKind::ResponseTooLarge,
        ));
    }
    Ok(())
}

pub(super) fn validate_response_status(
    provider: ProviderKind,
    status: http::StatusCode,
) -> Result<(), ProviderAdapterError> {
    if status.is_success() {
        return Ok(());
    }
    let kind = match status {
        http::StatusCode::UNAUTHORIZED | http::StatusCode::FORBIDDEN => {
            ProviderAdapterErrorKind::Authentication
        }
        http::StatusCode::REQUEST_TIMEOUT | http::StatusCode::GATEWAY_TIMEOUT => {
            ProviderAdapterErrorKind::Timeout
        }
        http::StatusCode::TOO_MANY_REQUESTS => ProviderAdapterErrorKind::RateLimited,
        _ if status.is_client_error() => ProviderAdapterErrorKind::Rejected,
        _ => ProviderAdapterErrorKind::Upstream,
    };
    Err(ProviderAdapterError::for_provider(provider, kind))
}

pub(super) fn parse_response<T: DeserializeOwned>(
    provider: ProviderKind,
    body: &[u8],
) -> Result<T, ProviderAdapterError> {
    serde_json::from_slice(body).map_err(|_| malformed_response(provider))
}

pub(super) fn exactly_one_text<'a>(
    provider: ProviderKind,
    values: impl IntoIterator<Item = &'a str>,
) -> Result<&'a str, ProviderAdapterError> {
    let mut values = values.into_iter().filter(|value| !value.trim().is_empty());
    let value = values.next().ok_or_else(|| malformed_response(provider))?;
    if values.next().is_some() {
        return Err(malformed_response(provider));
    }
    Ok(value)
}

pub(super) fn structured_response(
    provider: ProviderKind,
    requested_model: &str,
    response_model: Option<&str>,
    output_text: &str,
    finish_reason: FinishReason,
    usage: TokenUsage,
) -> Result<StructuredGenerationResponse, ProviderAdapterError> {
    let output = serde_json::from_str::<Value>(output_text).map_err(|_| {
        ProviderAdapterError::for_provider(provider, ProviderAdapterErrorKind::OutputSchemaInvalid)
    })?;
    if !output.is_object() {
        return Err(ProviderAdapterError::for_provider(
            provider,
            ProviderAdapterErrorKind::OutputSchemaInvalid,
        ));
    }
    let output_bytes = serde_json::to_vec(&output).map_err(|_| {
        ProviderAdapterError::for_provider(provider, ProviderAdapterErrorKind::OutputSchemaInvalid)
    })?;
    if output_bytes.len() > MAX_OUTPUT_BYTES {
        return Err(ProviderAdapterError::for_provider(
            provider,
            ProviderAdapterErrorKind::OutputSchemaInvalid,
        ));
    }
    let model_label = response_model.unwrap_or(requested_model);
    if model_label.is_empty()
        || model_label.len() > MAX_MODEL_BYTES
        || model_label
            .chars()
            .any(|character| character.is_ascii_control())
    {
        return Err(malformed_response(provider));
    }
    Ok(StructuredGenerationResponse {
        output,
        finish_reason,
        usage,
        model_label: model_label.to_owned(),
    })
}

pub(super) fn canonical_json(
    provider: ProviderKind,
    value: &Value,
) -> Result<String, ProviderAdapterError> {
    serde_json::to_string(value).map_err(|_| {
        ProviderAdapterError::for_provider(provider, ProviderAdapterErrorKind::InvalidRequest)
    })
}

pub(super) fn encode_request_body(
    provider: ProviderKind,
    path: String,
    headers: Vec<ProviderHeader>,
    body: &Value,
) -> Result<EncodedProviderRequest, ProviderAdapterError> {
    let body = serde_json::to_vec(body).map_err(|_| {
        ProviderAdapterError::for_provider(provider, ProviderAdapterErrorKind::InvalidRequest)
    })?;
    EncodedProviderRequest::new(path, headers, body)
        .map_err(|error| ProviderAdapterError::for_provider(provider, error.kind()))
}

pub(super) fn public_header(
    provider: ProviderKind,
    name: HeaderName,
    value: &str,
) -> Result<ProviderHeader, ProviderAdapterError> {
    let value = HeaderValue::from_str(value).map_err(|_| {
        ProviderAdapterError::for_provider(provider, ProviderAdapterErrorKind::InvalidRequest)
    })?;
    Ok(ProviderHeader::public(name, value))
}

fn validate_bounded_text(
    value: &str,
    minimum_bytes: usize,
    maximum_bytes: usize,
    allow_line_controls: bool,
) -> Result<(), ProviderAdapterError> {
    if !(minimum_bytes..=maximum_bytes).contains(&value.len()) {
        return Err(invalid_request());
    }
    if value.chars().any(|character| {
        character.is_ascii_control()
            && !(allow_line_controls && matches!(character, '\n' | '\r' | '\t'))
    }) {
        return Err(invalid_request());
    }
    Ok(())
}

fn validate_visible_ascii(
    value: &str,
    minimum_bytes: usize,
    maximum_bytes: usize,
) -> Result<(), ProviderAdapterError> {
    if !(minimum_bytes..=maximum_bytes).contains(&value.len())
        || !value.bytes().all(|byte| (0x21..=0x7e).contains(&byte))
    {
        return Err(invalid_request());
    }
    Ok(())
}

fn validate_schema_name(value: &str) -> Result<(), ProviderAdapterError> {
    let bytes = value.as_bytes();
    if !(1..=MAX_SCHEMA_NAME_BYTES).contains(&bytes.len())
        || !bytes[0].is_ascii_alphanumeric()
        || !bytes
            .iter()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
    {
        return Err(invalid_request());
    }
    Ok(())
}

fn validate_json_object(value: &Value, maximum_bytes: usize) -> Result<(), ProviderAdapterError> {
    if !value.is_object() {
        return Err(invalid_request());
    }
    let encoded = serde_json::to_vec(value).map_err(|_| invalid_request())?;
    if encoded.len() > maximum_bytes {
        return Err(invalid_request());
    }
    Ok(())
}

const fn invalid_request() -> ProviderAdapterError {
    ProviderAdapterError::request(ProviderAdapterErrorKind::InvalidRequest)
}

const fn malformed_response(provider: ProviderKind) -> ProviderAdapterError {
    ProviderAdapterError::for_provider(provider, ProviderAdapterErrorKind::MalformedResponse)
}
