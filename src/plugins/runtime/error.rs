use std::{error::Error, fmt};

use super::{
    bindings::types,
    capability::{CapabilityFailureHint, CapabilityUsage},
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PluginRuntimeErrorKind {
    InvalidComponent,
    ComponentDigestMismatch,
    LinkDenied,
    DescriptorMismatch,
    InvalidInvocation,
    CapabilityDenied,
    BrokerTimeout,
    BrokerFailure,
    FuelExhausted,
    MemoryLimit,
    GuestTimeout,
    GuestTrap,
    OutputTooLarge,
    RuntimeUnavailable,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PluginFailureCode {
    Disabled,
    ConfigInvalid,
    ProviderUnavailable,
    ProviderRateLimited,
    ProviderTimeout,
    ProviderOutputInvalid,
    McpSchemaInvalid,
    McpTimeout,
    McpBudgetExhausted,
    McpRecursionBlocked,
    BudgetExhausted,
    OutputInvalid,
}

impl PluginFailureCode {
    pub(crate) fn from_message_key(value: &str) -> Option<Self> {
        match value {
            "raindrop.ai-content.disabled" => Some(Self::Disabled),
            "raindrop.ai-content.config-invalid" => Some(Self::ConfigInvalid),
            "raindrop.ai-content.provider-unavailable" => Some(Self::ProviderUnavailable),
            "raindrop.ai-content.provider-rate-limited" => Some(Self::ProviderRateLimited),
            "raindrop.ai-content.provider-timeout" => Some(Self::ProviderTimeout),
            "raindrop.ai-content.provider-output-invalid" => Some(Self::ProviderOutputInvalid),
            "raindrop.ai-content.mcp-schema-invalid" => Some(Self::McpSchemaInvalid),
            "raindrop.ai-content.mcp-timeout" => Some(Self::McpTimeout),
            "raindrop.ai-content.mcp-budget-exhausted" => Some(Self::McpBudgetExhausted),
            "raindrop.ai-content.mcp-recursion-blocked" => Some(Self::McpRecursionBlocked),
            "raindrop.ai-content.budget-exhausted" => Some(Self::BudgetExhausted),
            "raindrop.ai-content.output-invalid" => Some(Self::OutputInvalid),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Eq, PartialEq)]
pub struct PluginRuntimeError {
    kind: PluginRuntimeErrorKind,
    failure_code: Option<PluginFailureCode>,
}

impl PluginRuntimeError {
    pub(crate) const fn new(kind: PluginRuntimeErrorKind) -> Self {
        Self {
            kind,
            failure_code: None,
        }
    }

    pub(crate) const fn with_failure_code(
        mut self,
        failure_code: Option<PluginFailureCode>,
    ) -> Self {
        self.failure_code = failure_code;
        self
    }

    #[must_use]
    pub const fn kind(&self) -> PluginRuntimeErrorKind {
        self.kind
    }

    #[must_use]
    pub const fn failure_code(&self) -> Option<PluginFailureCode> {
        self.failure_code
    }
}

impl fmt::Debug for PluginRuntimeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PluginRuntimeError")
            .field("kind", &self.kind)
            .finish()
    }
}

impl fmt::Display for PluginRuntimeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self.kind {
            PluginRuntimeErrorKind::InvalidComponent => "plugin component is invalid",
            PluginRuntimeErrorKind::ComponentDigestMismatch => {
                "plugin component digest does not match"
            }
            PluginRuntimeErrorKind::LinkDenied => "plugin component import is denied",
            PluginRuntimeErrorKind::DescriptorMismatch => "plugin descriptor does not match",
            PluginRuntimeErrorKind::InvalidInvocation => "plugin invocation is invalid",
            PluginRuntimeErrorKind::CapabilityDenied => "plugin capability is denied",
            PluginRuntimeErrorKind::BrokerTimeout => "plugin capability broker timed out",
            PluginRuntimeErrorKind::BrokerFailure => "plugin capability broker failed",
            PluginRuntimeErrorKind::FuelExhausted => "plugin fuel is exhausted",
            PluginRuntimeErrorKind::MemoryLimit => "plugin memory limit was exceeded",
            PluginRuntimeErrorKind::GuestTimeout => "plugin guest execution timed out",
            PluginRuntimeErrorKind::GuestTrap => "plugin guest execution trapped",
            PluginRuntimeErrorKind::OutputTooLarge => "plugin output is too large",
            PluginRuntimeErrorKind::RuntimeUnavailable => "plugin runtime is unavailable",
        })
    }
}

impl Error for PluginRuntimeError {}

pub struct PluginExecutionSuccess {
    artifact: types::ArtifactCandidate,
    usage: CapabilityUsage,
}

impl PluginExecutionSuccess {
    pub(crate) const fn new(artifact: types::ArtifactCandidate, usage: CapabilityUsage) -> Self {
        Self { artifact, usage }
    }

    #[must_use]
    pub const fn artifact(&self) -> &types::ArtifactCandidate {
        &self.artifact
    }

    #[must_use]
    pub const fn usage(&self) -> &CapabilityUsage {
        &self.usage
    }

    #[must_use]
    pub fn into_artifact(self) -> types::ArtifactCandidate {
        self.artifact
    }
}

impl fmt::Debug for PluginExecutionSuccess {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PluginExecutionSuccess")
            .field("usage", &self.usage)
            .finish_non_exhaustive()
    }
}

pub struct PluginExecutionFailure {
    error: PluginRuntimeError,
    usage: CapabilityUsage,
    failure_hint: Option<CapabilityFailureHint>,
}

impl PluginExecutionFailure {
    pub(crate) const fn new(
        error: PluginRuntimeError,
        usage: CapabilityUsage,
        failure_hint: Option<CapabilityFailureHint>,
    ) -> Self {
        Self {
            error,
            usage,
            failure_hint,
        }
    }

    #[must_use]
    pub const fn error(&self) -> &PluginRuntimeError {
        &self.error
    }

    #[must_use]
    pub const fn usage(&self) -> &CapabilityUsage {
        &self.usage
    }

    #[must_use]
    pub const fn failure_hint(&self) -> Option<CapabilityFailureHint> {
        self.failure_hint
    }

    #[must_use]
    pub fn into_error(self) -> PluginRuntimeError {
        self.error
    }
}

impl fmt::Debug for PluginExecutionFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PluginExecutionFailure")
            .field("error", &self.error)
            .field("usage", &self.usage)
            .field("failure_hint", &self.failure_hint)
            .finish()
    }
}

impl fmt::Display for PluginExecutionFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.error.fmt(formatter)
    }
}

impl Error for PluginExecutionFailure {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(&self.error)
    }
}
