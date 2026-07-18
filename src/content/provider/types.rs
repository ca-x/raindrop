use std::{error::Error, fmt};

use http::{HeaderName, HeaderValue};
use secrecy::SecretString;
use serde_json::Value;

use super::validation;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProviderKind {
    AnthropicMessages,
    OpenAiResponses,
    OpenAiChatCompletions,
    GoogleGemini,
}

pub struct OutputSchema {
    pub name: String,
    pub schema: Value,
}

pub struct StructuredGenerationRequest {
    pub model: String,
    pub system_instruction: String,
    pub untrusted_input: Value,
    pub output_schema: OutputSchema,
    pub max_output_tokens: u32,
    pub idempotency_key: String,
}

impl StructuredGenerationRequest {
    pub fn validate(&self) -> Result<(), ProviderAdapterError> {
        validation::validate_request(self)
    }
}

impl fmt::Debug for OutputSchema {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("OutputSchema")
            .field("name", &self.name)
            .field("schema_bytes", &encoded_json_len(&self.schema))
            .finish()
    }
}

impl fmt::Debug for StructuredGenerationRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("StructuredGenerationRequest")
            .field("model", &self.model)
            .field("system_instruction_bytes", &self.system_instruction.len())
            .field("input_bytes", &encoded_json_len(&self.untrusted_input))
            .field("output_schema", &self.output_schema)
            .field("max_output_tokens", &self.max_output_tokens)
            .field("idempotency_key", &"[REDACTED]")
            .finish()
    }
}

enum ProviderHeaderValue {
    Public(HeaderValue),
    Secret(SecretString),
}

pub struct ProviderHeader {
    name: HeaderName,
    value: ProviderHeaderValue,
}

impl ProviderHeader {
    #[must_use]
    pub fn public(name: HeaderName, value: HeaderValue) -> Self {
        Self {
            name,
            value: ProviderHeaderValue::Public(value),
        }
    }

    #[must_use]
    pub fn secret(name: HeaderName, value: SecretString) -> Self {
        Self {
            name,
            value: ProviderHeaderValue::Secret(value),
        }
    }

    #[must_use]
    pub const fn name(&self) -> &HeaderName {
        &self.name
    }

    #[must_use]
    pub const fn is_secret(&self) -> bool {
        matches!(self.value, ProviderHeaderValue::Secret(_))
    }

    #[must_use]
    pub const fn public_value(&self) -> Option<&HeaderValue> {
        match &self.value {
            ProviderHeaderValue::Public(value) => Some(value),
            ProviderHeaderValue::Secret(_) => None,
        }
    }

    #[must_use]
    pub const fn secret_value(&self) -> Option<&SecretString> {
        match &self.value {
            ProviderHeaderValue::Public(_) => None,
            ProviderHeaderValue::Secret(value) => Some(value),
        }
    }
}

impl fmt::Debug for ProviderHeader {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut debug = formatter.debug_struct("ProviderHeader");
        debug.field("name", &self.name);
        match &self.value {
            ProviderHeaderValue::Public(value) => debug.field("value", value),
            ProviderHeaderValue::Secret(_) => debug.field("value", &"[REDACTED]"),
        };
        debug.finish()
    }
}

pub struct EncodedProviderRequest {
    path: String,
    headers: Vec<ProviderHeader>,
    body: Vec<u8>,
}

impl EncodedProviderRequest {
    pub fn new(
        path: String,
        headers: Vec<ProviderHeader>,
        body: Vec<u8>,
    ) -> Result<Self, ProviderAdapterError> {
        validation::validate_encoded_request(&path, &body)?;
        Ok(Self {
            path,
            headers,
            body,
        })
    }

    #[must_use]
    pub fn path(&self) -> &str {
        &self.path
    }

    #[must_use]
    pub fn headers(&self) -> &[ProviderHeader] {
        &self.headers
    }

    #[must_use]
    pub fn body(&self) -> &[u8] {
        &self.body
    }
}

impl fmt::Debug for EncodedProviderRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("EncodedProviderRequest")
            .field("path", &self.path)
            .field("headers", &self.headers)
            .field("body_bytes", &self.body.len())
            .finish()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FinishReason {
    Stop,
    Length,
    ContentFilter,
    ToolCall,
    Other,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct TokenUsage {
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
}

#[derive(PartialEq)]
pub struct StructuredGenerationResponse {
    pub output: Value,
    pub finish_reason: FinishReason,
    pub usage: TokenUsage,
    pub model_label: String,
}

impl fmt::Debug for StructuredGenerationResponse {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("StructuredGenerationResponse")
            .field("output_bytes", &encoded_json_len(&self.output))
            .field("finish_reason", &self.finish_reason)
            .field("usage", &self.usage)
            .field("model_label", &self.model_label)
            .finish()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProviderAdapterErrorKind {
    InvalidRequest,
    RequestTooLarge,
    ResponseTooLarge,
    Authentication,
    RateLimited,
    Timeout,
    Rejected,
    Upstream,
    MalformedResponse,
    OutputSchemaInvalid,
}

pub struct ProviderAdapterError {
    provider: Option<ProviderKind>,
    kind: ProviderAdapterErrorKind,
}

impl ProviderAdapterError {
    pub(super) const fn request(kind: ProviderAdapterErrorKind) -> Self {
        Self {
            provider: None,
            kind,
        }
    }

    pub(super) const fn for_provider(
        provider: ProviderKind,
        kind: ProviderAdapterErrorKind,
    ) -> Self {
        Self {
            provider: Some(provider),
            kind,
        }
    }

    #[must_use]
    pub const fn provider(&self) -> Option<ProviderKind> {
        self.provider
    }

    #[must_use]
    pub const fn kind(&self) -> ProviderAdapterErrorKind {
        self.kind
    }
}

impl fmt::Debug for ProviderAdapterError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProviderAdapterError")
            .field("provider", &self.provider)
            .field("kind", &self.kind)
            .finish()
    }
}

impl fmt::Display for ProviderAdapterError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self.kind {
            ProviderAdapterErrorKind::InvalidRequest => "AI provider request is invalid",
            ProviderAdapterErrorKind::RequestTooLarge => "AI provider request is too large",
            ProviderAdapterErrorKind::ResponseTooLarge => "AI provider response is too large",
            ProviderAdapterErrorKind::Authentication => "AI provider authentication failed",
            ProviderAdapterErrorKind::RateLimited => "AI provider rate limit was reached",
            ProviderAdapterErrorKind::Timeout => "AI provider request timed out",
            ProviderAdapterErrorKind::Rejected => "AI provider rejected the request",
            ProviderAdapterErrorKind::Upstream => "AI provider failed",
            ProviderAdapterErrorKind::MalformedResponse => "AI provider response is malformed",
            ProviderAdapterErrorKind::OutputSchemaInvalid => {
                "AI provider structured output is invalid"
            }
        })
    }
}

impl Error for ProviderAdapterError {}

fn encoded_json_len(value: &Value) -> Option<usize> {
    serde_json::to_vec(value).ok().map(|encoded| encoded.len())
}
