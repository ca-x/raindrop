use std::{error::Error, fmt};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PluginRegistryErrorKind {
    InvalidInput,
    InvalidJson,
    DuplicateJsonKey,
    PayloadTooLarge,
    InvalidManifest,
    ComponentDigestMismatch,
    UnknownSigningKey,
    InvalidSignature,
    InvalidConfig,
    InvalidArtifact,
    InvalidLifecycleEvent,
    NotFound,
    RevisionConflict,
    QuotaExceeded,
    CorruptData,
    Database,
}

#[derive(Clone, Copy, Eq, PartialEq)]
pub struct PluginRegistryError {
    kind: PluginRegistryErrorKind,
}

impl PluginRegistryError {
    pub(crate) const fn new(kind: PluginRegistryErrorKind) -> Self {
        Self { kind }
    }

    #[must_use]
    pub const fn kind(&self) -> PluginRegistryErrorKind {
        self.kind
    }
}

impl fmt::Debug for PluginRegistryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PluginRegistryError")
            .field("kind", &self.kind)
            .finish()
    }
}

impl fmt::Display for PluginRegistryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self.kind {
            PluginRegistryErrorKind::InvalidInput => "plugin registry input is invalid",
            PluginRegistryErrorKind::InvalidJson => "plugin JSON is invalid",
            PluginRegistryErrorKind::DuplicateJsonKey => "plugin JSON contains a duplicate key",
            PluginRegistryErrorKind::PayloadTooLarge => "plugin payload is too large",
            PluginRegistryErrorKind::InvalidManifest => "plugin manifest is invalid",
            PluginRegistryErrorKind::ComponentDigestMismatch => {
                "plugin component digest does not match"
            }
            PluginRegistryErrorKind::UnknownSigningKey => "plugin signing key is unknown",
            PluginRegistryErrorKind::InvalidSignature => "plugin signature is invalid",
            PluginRegistryErrorKind::InvalidConfig => "plugin configuration is invalid",
            PluginRegistryErrorKind::InvalidArtifact => "plugin artifact is invalid",
            PluginRegistryErrorKind::InvalidLifecycleEvent => "plugin lifecycle event is invalid",
            PluginRegistryErrorKind::NotFound => "plugin registry record was not found",
            PluginRegistryErrorKind::RevisionConflict => "plugin registry revision conflicts",
            PluginRegistryErrorKind::QuotaExceeded => "plugin registry quota is exceeded",
            PluginRegistryErrorKind::CorruptData => "plugin registry data is corrupt",
            PluginRegistryErrorKind::Database => "plugin registry database operation failed",
        })
    }
}

impl Error for PluginRegistryError {}
