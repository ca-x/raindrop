use std::{fmt, net::IpAddr};

use secrecy::{ExposeSecret, SecretString};
use time::OffsetDateTime;
use url::{Host, Url};
use uuid::Uuid;

use crate::feeds::{AddressDecision, AddressPolicy};

use super::ProviderKind;

const MAX_ENDPOINT_BYTES: usize = 2_048;
const MAX_ADAPTER_PATH_BYTES: usize = 2_048;
const MAX_DISPLAY_NAME_BYTES: usize = 80;
const MAX_MODEL_BYTES: usize = 200;
const MAX_CREDENTIAL_BYTES: usize = 8_192;
const MAX_CONCURRENCY: u16 = 64;
const MAX_REQUESTS_PER_MINUTE: u32 = 1_000_000;
const MAX_INPUT_TOKENS_PER_REQUEST: u32 = 1_048_576;
const MAX_OUTPUT_TOKENS_PER_REQUEST: u32 = 16_384;
const MAX_COST_MICROS: u64 = 1_000_000_000_000;

impl ProviderKind {
    pub fn from_storage(value: &str) -> Result<Self, ProviderCoreError> {
        match value {
            "ANTHROPIC_MESSAGES" => Ok(Self::AnthropicMessages),
            "OPENAI_RESPONSES" => Ok(Self::OpenAiResponses),
            "OPENAI_CHAT_COMPLETIONS" => Ok(Self::OpenAiChatCompletions),
            "GOOGLE_GEMINI" => Ok(Self::GoogleGemini),
            _ => Err(ProviderCoreError::new(ProviderCoreErrorKind::CorruptData)),
        }
    }

    #[must_use]
    pub const fn default_endpoint(self) -> &'static str {
        match self {
            Self::AnthropicMessages => "https://api.anthropic.com/",
            Self::OpenAiResponses | Self::OpenAiChatCompletions => "https://api.openai.com/",
            Self::GoogleGemini => "https://generativelanguage.googleapis.com/",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ProviderScope {
    Instance,
    User(String),
}

impl ProviderScope {
    pub fn user(user_id: impl Into<String>) -> Result<Self, ProviderCoreError> {
        let user_id = user_id.into();
        validate_uuid(&user_id, ProviderCoreErrorKind::InvalidUserId)?;
        Ok(Self::User(user_id))
    }

    pub fn from_owner_user_id(owner_user_id: Option<String>) -> Result<Self, ProviderCoreError> {
        owner_user_id.map_or(Ok(Self::Instance), Self::user)
    }

    pub(super) fn validate(&self) -> Result<(), ProviderCoreError> {
        match self {
            Self::Instance => Ok(()),
            Self::User(user_id) => validate_uuid(user_id, ProviderCoreErrorKind::InvalidUserId),
        }
    }

    #[must_use]
    pub fn owner_user_id(&self) -> Option<&str> {
        match self {
            Self::Instance => None,
            Self::User(user_id) => Some(user_id),
        }
    }
}

#[derive(Clone, Eq, PartialEq)]
pub struct ProviderEndpoint {
    url: Url,
    canonical_host: String,
    effective_port: u16,
    path_segment_count: usize,
}

impl ProviderEndpoint {
    pub fn new(kind: ProviderKind, raw: Option<&str>) -> Result<Self, ProviderCoreError> {
        let raw = raw.unwrap_or_else(|| kind.default_endpoint());
        validate_endpoint_source(raw)?;
        let mut url = Url::parse(raw).map_err(|_| invalid_endpoint())?;
        if url.scheme() != "https"
            || url.host().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
        {
            return Err(invalid_endpoint());
        }
        if let Some(address) = literal_address(&url)
            && AddressPolicy::public_only().classify(address) != AddressDecision::Allowed
        {
            return Err(invalid_endpoint());
        }

        let path = url.path();
        let path = if path.ends_with('/') {
            path.to_owned()
        } else {
            format!("{path}/")
        };
        url.set_path(&path);
        let canonical_host = url.host_str().ok_or_else(invalid_endpoint)?.to_owned();
        let effective_port = url.port_or_known_default().ok_or_else(invalid_endpoint)?;
        let path_segment_count = url
            .path_segments()
            .ok_or_else(invalid_endpoint)?
            .filter(|segment| !segment.is_empty())
            .count();
        if url.as_str().len() > MAX_ENDPOINT_BYTES {
            return Err(invalid_endpoint());
        }
        Ok(Self {
            url,
            canonical_host,
            effective_port,
            path_segment_count,
        })
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        self.url.as_str()
    }

    pub fn join_adapter_path(&self, path: &str) -> Result<Url, ProviderCoreError> {
        validate_adapter_path(path)?;
        let joined = self.url.join(&path[1..]).map_err(|_| invalid_endpoint())?;
        if joined.scheme() != self.url.scheme()
            || joined.host_str() != self.url.host_str()
            || joined.port_or_known_default() != self.url.port_or_known_default()
            || !joined.path().starts_with(self.url.path())
            || joined.query().is_some()
            || joined.fragment().is_some()
        {
            return Err(invalid_endpoint());
        }
        Ok(joined)
    }

    #[must_use]
    pub fn canonical_host(&self) -> &str {
        &self.canonical_host
    }

    #[must_use]
    pub const fn effective_port(&self) -> u16 {
        self.effective_port
    }
}

impl fmt::Debug for ProviderEndpoint {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProviderEndpoint")
            .field("scheme", &"https")
            .field("canonical_host", &self.canonical_host)
            .field("effective_port", &self.effective_port)
            .field("path_segment_count", &self.path_segment_count)
            .finish()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ProviderCapabilities {
    pub supports_usage: bool,
    pub supports_idempotency: bool,
    pub supports_streaming: bool,
}

impl ProviderCapabilities {
    pub fn validate(self) -> Result<(), ProviderCoreError> {
        if self.supports_streaming {
            Err(ProviderCoreError::new(
                ProviderCoreErrorKind::UnsupportedCapability,
            ))
        } else {
            Ok(())
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ProviderPolicy {
    pub max_concurrency: u16,
    pub requests_per_minute: Option<u32>,
    pub max_input_tokens_per_request: u32,
    pub max_output_tokens_per_request: u32,
    pub input_cost_micros_per_million_tokens: Option<u64>,
    pub output_cost_micros_per_million_tokens: Option<u64>,
    pub max_cost_micros_per_request: Option<u64>,
}

impl ProviderPolicy {
    pub fn validate(self) -> Result<(), ProviderCoreError> {
        if !(1..=MAX_CONCURRENCY).contains(&self.max_concurrency)
            || self
                .requests_per_minute
                .is_some_and(|value| !(1..=MAX_REQUESTS_PER_MINUTE).contains(&value))
            || !(1..=MAX_INPUT_TOKENS_PER_REQUEST).contains(&self.max_input_tokens_per_request)
            || !(1..=MAX_OUTPUT_TOKENS_PER_REQUEST).contains(&self.max_output_tokens_per_request)
            || [
                self.input_cost_micros_per_million_tokens,
                self.output_cost_micros_per_million_tokens,
                self.max_cost_micros_per_request,
            ]
            .into_iter()
            .flatten()
            .any(|value| value > MAX_COST_MICROS)
        {
            Err(ProviderCoreError::new(ProviderCoreErrorKind::InvalidPolicy))
        } else {
            Ok(())
        }
    }
}

pub struct CreateProvider {
    pub scope: ProviderScope,
    pub display_name: String,
    pub kind: ProviderKind,
    pub endpoint: Option<String>,
    pub model: String,
    pub credential: SecretString,
    pub capabilities: ProviderCapabilities,
    pub policy: ProviderPolicy,
    pub is_enabled: bool,
}

impl CreateProvider {
    pub fn validate(&self) -> Result<(), ProviderCoreError> {
        self.scope.validate()?;
        validate_display_name(&self.display_name)?;
        ProviderEndpoint::new(self.kind, self.endpoint.as_deref())?;
        validate_model(&self.model)?;
        validate_credential(&self.credential)?;
        self.capabilities.validate()?;
        self.policy.validate()
    }
}

impl fmt::Debug for CreateProvider {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CreateProvider")
            .field("scope", &self.scope)
            .field("display_name", &self.display_name)
            .field("kind", &self.kind)
            .field("endpoint", &self.endpoint.as_ref().map(|_| "[REDACTED]"))
            .field("model", &"[REDACTED]")
            .field("credential", &"[REDACTED]")
            .field("capabilities", &self.capabilities)
            .field("policy", &self.policy)
            .field("is_enabled", &self.is_enabled)
            .finish()
    }
}

#[derive(Default)]
pub struct UpdateProvider {
    pub expected_revision: u64,
    pub display_name: Option<String>,
    pub endpoint: Option<String>,
    pub model: Option<String>,
    pub credential: Option<SecretString>,
    pub capabilities: Option<ProviderCapabilities>,
    pub policy: Option<ProviderPolicy>,
    pub is_enabled: Option<bool>,
}

impl UpdateProvider {
    pub fn validate(&self, kind: ProviderKind) -> Result<(), ProviderCoreError> {
        if self.display_name.is_none()
            && self.endpoint.is_none()
            && self.model.is_none()
            && self.credential.is_none()
            && self.capabilities.is_none()
            && self.policy.is_none()
            && self.is_enabled.is_none()
        {
            return Err(ProviderCoreError::new(ProviderCoreErrorKind::InvalidPatch));
        }
        if let Some(value) = &self.display_name {
            validate_display_name(value)?;
        }
        if let Some(value) = &self.endpoint {
            ProviderEndpoint::new(kind, Some(value))?;
        }
        if let Some(value) = &self.model {
            validate_model(value)?;
        }
        if let Some(value) = &self.credential {
            validate_credential(value)?;
        }
        if let Some(value) = self.capabilities {
            value.validate()?;
        }
        if let Some(value) = self.policy {
            value.validate()?;
        }
        Ok(())
    }
}

impl fmt::Debug for UpdateProvider {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("UpdateProvider")
            .field("expected_revision", &self.expected_revision)
            .field("display_name", &self.display_name)
            .field("endpoint", &self.endpoint.as_ref().map(|_| "[REDACTED]"))
            .field("model", &self.model.as_ref().map(|_| "[REDACTED]"))
            .field(
                "credential",
                &self.credential.as_ref().map(|_| "[REDACTED]"),
            )
            .field("capabilities", &self.capabilities)
            .field("policy", &self.policy)
            .field("is_enabled", &self.is_enabled)
            .finish()
    }
}

#[derive(Clone, Eq, PartialEq)]
pub struct ProviderMetadata {
    pub(super) id: String,
    pub(super) scope: ProviderScope,
    pub(super) display_name: String,
    pub(super) kind: ProviderKind,
    pub(super) endpoint: ProviderEndpoint,
    pub(super) model: String,
    pub(super) capabilities: ProviderCapabilities,
    pub(super) policy: ProviderPolicy,
    pub(super) is_enabled: bool,
    pub(super) revision: u64,
    pub(super) created_at: OffsetDateTime,
    pub(super) updated_at: OffsetDateTime,
}

impl ProviderMetadata {
    #[must_use]
    pub fn id(&self) -> &str {
        &self.id
    }

    #[must_use]
    pub const fn scope(&self) -> &ProviderScope {
        &self.scope
    }

    #[must_use]
    pub fn display_name(&self) -> &str {
        &self.display_name
    }

    #[must_use]
    pub const fn kind(&self) -> ProviderKind {
        self.kind
    }

    #[must_use]
    pub const fn endpoint(&self) -> &ProviderEndpoint {
        &self.endpoint
    }

    #[must_use]
    pub fn model(&self) -> &str {
        &self.model
    }

    #[must_use]
    pub const fn capabilities(&self) -> ProviderCapabilities {
        self.capabilities
    }

    #[must_use]
    pub const fn policy(&self) -> ProviderPolicy {
        self.policy
    }

    #[must_use]
    pub const fn is_enabled(&self) -> bool {
        self.is_enabled
    }

    #[must_use]
    pub const fn revision(&self) -> u64 {
        self.revision
    }

    #[must_use]
    pub const fn created_at(&self) -> OffsetDateTime {
        self.created_at
    }

    #[must_use]
    pub const fn updated_at(&self) -> OffsetDateTime {
        self.updated_at
    }
}

impl fmt::Debug for ProviderMetadata {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProviderMetadata")
            .field("id", &self.id)
            .field("scope", &self.scope)
            .field("display_name", &self.display_name)
            .field("kind", &self.kind)
            .field("endpoint", &"[REDACTED]")
            .field("model", &"[REDACTED]")
            .field("capabilities", &self.capabilities)
            .field("policy", &self.policy)
            .field("is_enabled", &self.is_enabled)
            .field("revision", &self.revision)
            .field("created_at", &self.created_at)
            .field("updated_at", &self.updated_at)
            .finish()
    }
}

pub struct ProviderBinding {
    pub(super) metadata: ProviderMetadata,
    pub(super) credential: SecretString,
}

impl ProviderBinding {
    #[must_use]
    pub const fn metadata(&self) -> &ProviderMetadata {
        &self.metadata
    }

    pub(super) const fn credential(&self) -> &SecretString {
        &self.credential
    }
}

impl fmt::Debug for ProviderBinding {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let _credential = self.credential();
        formatter
            .debug_struct("ProviderBinding")
            .field("id", &self.metadata.id)
            .field("kind", &self.metadata.kind)
            .field("is_enabled", &self.metadata.is_enabled)
            .field("revision", &self.metadata.revision)
            .field("endpoint", &"[REDACTED]")
            .field("model", &"[REDACTED]")
            .field("credential", &"[REDACTED]")
            .finish()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProviderCoreErrorKind {
    InvalidProviderId,
    InvalidUserId,
    InvalidDisplayName,
    InvalidEndpoint,
    InvalidModel,
    InvalidCredential,
    UnsupportedCapability,
    InvalidPolicy,
    InvalidPatch,
    NotFound,
    ProviderDisabled,
    RevisionConflict,
    SecretUnavailable,
    Database,
    CorruptData,
}

pub struct ProviderCoreError {
    kind: ProviderCoreErrorKind,
}

impl ProviderCoreError {
    pub(super) const fn new(kind: ProviderCoreErrorKind) -> Self {
        Self { kind }
    }

    #[must_use]
    pub const fn kind(&self) -> ProviderCoreErrorKind {
        self.kind
    }
}

impl fmt::Debug for ProviderCoreError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProviderCoreError")
            .field("kind", &self.kind)
            .finish()
    }
}

impl fmt::Display for ProviderCoreError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self.kind {
            ProviderCoreErrorKind::InvalidProviderId => "AI provider identifier is invalid",
            ProviderCoreErrorKind::InvalidUserId => "AI provider owner identifier is invalid",
            ProviderCoreErrorKind::InvalidDisplayName => "AI provider display name is invalid",
            ProviderCoreErrorKind::InvalidEndpoint => "AI provider endpoint is invalid",
            ProviderCoreErrorKind::InvalidModel => "AI provider model is invalid",
            ProviderCoreErrorKind::InvalidCredential => "AI provider credential is invalid",
            ProviderCoreErrorKind::UnsupportedCapability => {
                "AI provider capability is not supported"
            }
            ProviderCoreErrorKind::InvalidPolicy => "AI provider policy is invalid",
            ProviderCoreErrorKind::InvalidPatch => "AI provider update is empty",
            ProviderCoreErrorKind::NotFound => "AI provider is unavailable",
            ProviderCoreErrorKind::ProviderDisabled => "AI provider is disabled",
            ProviderCoreErrorKind::RevisionConflict => "AI provider revision conflicts",
            ProviderCoreErrorKind::SecretUnavailable => "AI provider credential is unavailable",
            ProviderCoreErrorKind::Database => "AI provider database operation failed",
            ProviderCoreErrorKind::CorruptData => "AI provider stored data is corrupt",
        })
    }
}

impl std::error::Error for ProviderCoreError {}

pub(super) fn validate_provider_id(value: &str) -> Result<(), ProviderCoreError> {
    validate_uuid(value, ProviderCoreErrorKind::InvalidProviderId)
}

pub(super) fn normalize_display_name(value: &str) -> Result<String, ProviderCoreError> {
    let value = value.trim();
    if !(1..=MAX_DISPLAY_NAME_BYTES).contains(&value.len())
        || value.chars().any(|character| character.is_ascii_control())
    {
        Err(ProviderCoreError::new(
            ProviderCoreErrorKind::InvalidDisplayName,
        ))
    } else {
        Ok(value.to_owned())
    }
}

pub(super) fn validate_model(value: &str) -> Result<(), ProviderCoreError> {
    if !(1..=MAX_MODEL_BYTES).contains(&value.len())
        || value.chars().any(|character| character.is_ascii_control())
    {
        Err(ProviderCoreError::new(ProviderCoreErrorKind::InvalidModel))
    } else {
        Ok(())
    }
}

fn validate_display_name(value: &str) -> Result<(), ProviderCoreError> {
    normalize_display_name(value).map(|_| ())
}

fn validate_credential(value: &SecretString) -> Result<(), ProviderCoreError> {
    if (1..=MAX_CREDENTIAL_BYTES).contains(&value.expose_secret().len()) {
        Ok(())
    } else {
        Err(ProviderCoreError::new(
            ProviderCoreErrorKind::InvalidCredential,
        ))
    }
}

fn validate_uuid(value: &str, kind: ProviderCoreErrorKind) -> Result<(), ProviderCoreError> {
    Uuid::parse_str(value)
        .map(|_| ())
        .map_err(|_| ProviderCoreError::new(kind))
}

fn validate_endpoint_source(raw: &str) -> Result<(), ProviderCoreError> {
    if raw.is_empty()
        || raw.len() > MAX_ENDPOINT_BYTES
        || raw
            .chars()
            .any(|character| character == ' ' || character.is_ascii_control())
        || raw.contains('\\')
    {
        return Err(invalid_endpoint());
    }
    let path = raw_endpoint_path(raw);
    let lower_path = path.to_ascii_lowercase();
    if lower_path.contains("%2e")
        || lower_path.contains("%2f")
        || lower_path.contains("%5c")
        || path.split('/').any(|segment| matches!(segment, "." | ".."))
    {
        return Err(invalid_endpoint());
    }
    Ok(())
}

fn raw_endpoint_path(raw: &str) -> &str {
    let Some((_, after_scheme)) = raw.split_once("://") else {
        return "";
    };
    let start = after_scheme.find('/').unwrap_or(after_scheme.len());
    let rest = &after_scheme[start..];
    let end = rest.find(['?', '#']).unwrap_or(rest.len());
    &rest[..end]
}

fn literal_address(url: &Url) -> Option<IpAddr> {
    match url.host()? {
        Host::Ipv4(address) => Some(IpAddr::V4(address)),
        Host::Ipv6(address) => Some(IpAddr::V6(address)),
        Host::Domain(_) => None,
    }
}

fn validate_adapter_path(value: &str) -> Result<(), ProviderCoreError> {
    if !(2..=MAX_ADAPTER_PATH_BYTES).contains(&value.len())
        || !value.starts_with('/')
        || value.contains("//")
        || value.contains(['?', '#', '\\'])
        || value
            .chars()
            .any(|character| character == ' ' || character.is_ascii_control())
        || value
            .split('/')
            .any(|segment| matches!(segment, "." | ".."))
    {
        Err(invalid_endpoint())
    } else {
        Ok(())
    }
}

const fn invalid_endpoint() -> ProviderCoreError {
    ProviderCoreError::new(ProviderCoreErrorKind::InvalidEndpoint)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metadata_binding_and_errors_redact_operational_values() {
        let metadata = ProviderMetadata {
            id: "00000000-0000-4000-8000-000000000901".to_owned(),
            scope: ProviderScope::Instance,
            display_name: "Provider".to_owned(),
            kind: ProviderKind::OpenAiResponses,
            endpoint: ProviderEndpoint::new(
                ProviderKind::OpenAiResponses,
                Some("https://gateway.example/endpoint-sentinel/"),
            )
            .unwrap(),
            model: "model-sentinel".to_owned(),
            capabilities: ProviderCapabilities {
                supports_usage: true,
                supports_idempotency: true,
                supports_streaming: false,
            },
            policy: ProviderPolicy {
                max_concurrency: 2,
                requests_per_minute: Some(60),
                max_input_tokens_per_request: 128_000,
                max_output_tokens_per_request: 16_384,
                input_cost_micros_per_million_tokens: None,
                output_cost_micros_per_million_tokens: None,
                max_cost_micros_per_request: Some(250_000),
            },
            is_enabled: true,
            revision: u64::MAX,
            created_at: OffsetDateTime::UNIX_EPOCH,
            updated_at: OffsetDateTime::UNIX_EPOCH,
        };
        let binding = ProviderBinding {
            metadata: metadata.clone(),
            credential: SecretString::from("credential-sentinel"),
        };
        let error = ProviderCoreError::new(ProviderCoreErrorKind::CorruptData);
        let formatted = format!("{metadata:?} {binding:?} {error:?} {error}");

        assert!(formatted.contains(&u64::MAX.to_string()));
        for sentinel in ["endpoint-sentinel", "model-sentinel", "credential-sentinel"] {
            assert!(!formatted.contains(sentinel));
        }
    }
}
