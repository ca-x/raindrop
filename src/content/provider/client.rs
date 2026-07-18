use std::{error::Error, fmt};

use time::OffsetDateTime;

use super::{
    ProviderAdapterError, ProviderAdapterErrorKind, ProviderBinding, ProviderKind,
    ProviderTransport, ProviderTransportError, ProviderTransportErrorKind,
    StructuredGenerationRequest, StructuredGenerationResponse,
};

pub struct ProviderClient<T> {
    transport: T,
}

impl<T> ProviderClient<T> {
    #[must_use]
    pub const fn new(transport: T) -> Self {
        Self { transport }
    }
}

impl<T> ProviderClient<T>
where
    T: ProviderTransport,
{
    pub async fn generate(
        &self,
        binding: &ProviderBinding,
        request: &StructuredGenerationRequest,
    ) -> Result<StructuredGenerationResponse, ProviderCallError> {
        let metadata = binding.metadata();
        if request.model != metadata.model() {
            return Err(ProviderCallError::new(
                metadata.id(),
                metadata.kind(),
                ProviderCallErrorKind::InvalidRequest,
            ));
        }
        if request.max_output_tokens > metadata.policy().max_output_tokens_per_request {
            return Err(ProviderCallError::new(
                metadata.id(),
                metadata.kind(),
                ProviderCallErrorKind::RequestTooLarge,
            ));
        }

        let encoded = metadata
            .kind()
            .encode_request(request, binding.credential().clone())
            .map_err(|error| ProviderCallError::adapter(metadata.id(), metadata.kind(), error))?;
        let response = self
            .transport
            .execute(metadata.id(), metadata.endpoint(), encoded)
            .await
            .map_err(|error| ProviderCallError::transport(metadata.id(), metadata.kind(), error))?;
        let retry_after_at = response.retry_after().map(|retry| retry.at());
        metadata
            .kind()
            .decode_response(metadata.model(), response.status(), response.body())
            .map_err(|error| {
                ProviderCallError::adapter(metadata.id(), metadata.kind(), error)
                    .with_retry_after_at(retry_after_at)
            })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProviderCallErrorKind {
    InvalidRequest,
    RequestTooLarge,
    Transport,
    Timeout,
    Authentication,
    RateLimited,
    Rejected,
    Upstream,
    ResponseTooLarge,
    MalformedResponse,
    OutputSchemaInvalid,
}

enum ProviderCallErrorSource {
    Adapter(ProviderAdapterError),
    Transport(ProviderTransportError),
}

pub struct ProviderCallError {
    provider_id: String,
    provider_kind: ProviderKind,
    kind: ProviderCallErrorKind,
    retry_after_at: Option<OffsetDateTime>,
    source: Option<ProviderCallErrorSource>,
}

impl ProviderCallError {
    fn new(provider_id: &str, provider_kind: ProviderKind, kind: ProviderCallErrorKind) -> Self {
        Self {
            provider_id: provider_id.to_owned(),
            provider_kind,
            kind,
            retry_after_at: None,
            source: None,
        }
    }

    fn adapter(
        provider_id: &str,
        provider_kind: ProviderKind,
        source: ProviderAdapterError,
    ) -> Self {
        let kind = match source.kind() {
            ProviderAdapterErrorKind::InvalidRequest => ProviderCallErrorKind::InvalidRequest,
            ProviderAdapterErrorKind::RequestTooLarge => ProviderCallErrorKind::RequestTooLarge,
            ProviderAdapterErrorKind::ResponseTooLarge => ProviderCallErrorKind::ResponseTooLarge,
            ProviderAdapterErrorKind::Authentication => ProviderCallErrorKind::Authentication,
            ProviderAdapterErrorKind::RateLimited => ProviderCallErrorKind::RateLimited,
            ProviderAdapterErrorKind::Timeout => ProviderCallErrorKind::Timeout,
            ProviderAdapterErrorKind::Rejected => ProviderCallErrorKind::Rejected,
            ProviderAdapterErrorKind::Upstream => ProviderCallErrorKind::Upstream,
            ProviderAdapterErrorKind::MalformedResponse => ProviderCallErrorKind::MalformedResponse,
            ProviderAdapterErrorKind::OutputSchemaInvalid => {
                ProviderCallErrorKind::OutputSchemaInvalid
            }
        };
        let mut error = Self::new(provider_id, provider_kind, kind);
        error.source = Some(ProviderCallErrorSource::Adapter(source));
        error
    }

    fn transport(
        provider_id: &str,
        provider_kind: ProviderKind,
        source: ProviderTransportError,
    ) -> Self {
        let kind = match source.kind() {
            ProviderTransportErrorKind::Timeout => ProviderCallErrorKind::Timeout,
            ProviderTransportErrorKind::ResponseTooLarge => ProviderCallErrorKind::ResponseTooLarge,
            _ => ProviderCallErrorKind::Transport,
        };
        let mut error = Self::new(provider_id, provider_kind, kind);
        error.source = Some(ProviderCallErrorSource::Transport(source));
        error
    }

    fn with_retry_after_at(mut self, retry_after_at: Option<OffsetDateTime>) -> Self {
        self.retry_after_at = retry_after_at;
        self
    }

    #[must_use]
    pub fn provider_id(&self) -> &str {
        &self.provider_id
    }

    #[must_use]
    pub const fn provider_kind(&self) -> ProviderKind {
        self.provider_kind
    }

    #[must_use]
    pub const fn kind(&self) -> ProviderCallErrorKind {
        self.kind
    }

    #[must_use]
    pub const fn retry_after_at(&self) -> Option<OffsetDateTime> {
        self.retry_after_at
    }
}

impl fmt::Debug for ProviderCallError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProviderCallError")
            .field("provider_id", &self.provider_id)
            .field("provider_kind", &self.provider_kind)
            .field("kind", &self.kind)
            .field("retry_after_at", &self.retry_after_at)
            .finish()
    }
}

impl fmt::Display for ProviderCallError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self.kind {
            ProviderCallErrorKind::InvalidRequest => "AI provider request is invalid",
            ProviderCallErrorKind::RequestTooLarge => "AI provider request is too large",
            ProviderCallErrorKind::Transport => "AI provider transport failed",
            ProviderCallErrorKind::Timeout => "AI provider request timed out",
            ProviderCallErrorKind::Authentication => "AI provider authentication failed",
            ProviderCallErrorKind::RateLimited => "AI provider rate limit was reached",
            ProviderCallErrorKind::Rejected => "AI provider rejected the request",
            ProviderCallErrorKind::Upstream => "AI provider failed",
            ProviderCallErrorKind::ResponseTooLarge => "AI provider response is too large",
            ProviderCallErrorKind::MalformedResponse => "AI provider response is malformed",
            ProviderCallErrorKind::OutputSchemaInvalid => {
                "AI provider structured output is invalid"
            }
        })
    }
}

impl Error for ProviderCallError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self.source.as_ref() {
            Some(ProviderCallErrorSource::Adapter(error)) => Some(error),
            Some(ProviderCallErrorSource::Transport(error)) => Some(error),
            None => None,
        }
    }
}
