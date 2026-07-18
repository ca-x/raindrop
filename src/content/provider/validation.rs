use serde_json::Value;

use super::{
    ProviderAdapterError, ProviderAdapterErrorKind, ProviderKind, StructuredGenerationRequest,
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
