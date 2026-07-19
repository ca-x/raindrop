use std::{error::Error, fmt};

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
