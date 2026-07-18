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

#[derive(Clone, Copy, Eq, PartialEq)]
pub struct PluginRuntimeError {
    kind: PluginRuntimeErrorKind,
}

impl PluginRuntimeError {
    pub(crate) const fn new(kind: PluginRuntimeErrorKind) -> Self {
        Self { kind }
    }

    #[must_use]
    pub const fn kind(&self) -> PluginRuntimeErrorKind {
        self.kind
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
