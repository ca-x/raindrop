mod anthropic;
mod gemini;
mod openai_chat;
mod openai_responses;
mod types;
mod validation;

pub use types::{
    EncodedProviderRequest, FinishReason, OutputSchema, ProviderAdapterError,
    ProviderAdapterErrorKind, ProviderHeader, ProviderKind, StructuredGenerationRequest,
    StructuredGenerationResponse, TokenUsage,
};

use http::StatusCode;
use secrecy::SecretString;

impl ProviderKind {
    pub fn encode_request(
        self,
        request: &StructuredGenerationRequest,
        credential: SecretString,
    ) -> Result<EncodedProviderRequest, ProviderAdapterError> {
        request.validate()?;
        match self {
            Self::AnthropicMessages => anthropic::encode(request, credential),
            Self::OpenAiResponses => openai_responses::encode(request, credential),
            Self::OpenAiChatCompletions => openai_chat::encode(request, credential),
            Self::GoogleGemini => gemini::encode(request, credential),
        }
    }

    pub fn decode_response(
        self,
        _requested_model: &str,
        _status: StatusCode,
        body: &[u8],
    ) -> Result<StructuredGenerationResponse, ProviderAdapterError> {
        validation::validate_response_body(self, body)?;
        Err(ProviderAdapterError::for_provider(
            self,
            ProviderAdapterErrorKind::MalformedResponse,
        ))
    }
}
