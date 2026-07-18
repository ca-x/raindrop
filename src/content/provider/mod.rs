mod anthropic;
mod gemini;
mod openai_chat;
mod openai_responses;
mod secret;
mod types;
mod validation;

pub use secret::{ProviderSecretError, ProviderSecretErrorKind, ProviderSecretKeyring};
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
        requested_model: &str,
        status: StatusCode,
        body: &[u8],
    ) -> Result<StructuredGenerationResponse, ProviderAdapterError> {
        validation::validate_response_body(self, body)?;
        validation::validate_response_status(self, status)?;
        match self {
            Self::AnthropicMessages => anthropic::decode(requested_model, body),
            Self::OpenAiResponses => openai_responses::decode(requested_model, body),
            Self::OpenAiChatCompletions => openai_chat::decode(requested_model, body),
            Self::GoogleGemini => gemini::decode(requested_model, body),
        }
    }
}
