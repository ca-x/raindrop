mod anthropic;
mod catalog;
mod client;
mod gemini;
mod model;
mod network;
mod openai_chat;
mod openai_responses;
mod repository;
mod secret;
mod transport;
mod types;
mod validation;

pub use catalog::{
    ModelLimits, cached_model_catalog, catalog_model_limits, model_catalog, service_model_limits,
};
pub use client::{ProviderCallError, ProviderCallErrorKind, ProviderClient};
pub use model::{
    CreateProvider, ProviderBinding, ProviderCapabilities, ProviderCoreError,
    ProviderCoreErrorKind, ProviderEndpoint, ProviderMetadata, ProviderPolicy, ProviderScope,
    UpdateProvider,
};
pub use network::ProviderNetworkPolicy;
pub use repository::ProviderRepository;
pub use secret::{ProviderSecretError, ProviderSecretErrorKind, ProviderSecretKeyring};
pub use transport::{
    HttpsProviderTransport, ProviderRetryAfter, ProviderTimeoutStage, ProviderTransport,
    ProviderTransportError, ProviderTransportErrorKind, ProviderTransportResponse,
};
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
