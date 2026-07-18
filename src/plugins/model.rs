use std::fmt;

use time::OffsetDateTime;

use super::AiContentConfig;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PluginSystemState {
    Enabled,
    Disabled,
    Quarantined,
}

impl PluginSystemState {
    pub(crate) const fn as_storage(self) -> &'static str {
        match self {
            Self::Enabled => "ENABLED",
            Self::Disabled => "DISABLED",
            Self::Quarantined => "QUARANTINED",
        }
    }

    pub(crate) fn from_storage(value: &str) -> Option<Self> {
        match value {
            "ENABLED" => Some(Self::Enabled),
            "DISABLED" => Some(Self::Disabled),
            "QUARANTINED" => Some(Self::Quarantined),
            _ => None,
        }
    }
}

#[derive(Clone, Eq, PartialEq)]
pub struct PluginInstallation {
    pub(crate) id: String,
    pub(crate) plugin_key: String,
    pub(crate) version: String,
    pub(crate) abi_version: String,
    pub(crate) component_digest: String,
    pub(crate) system_state: PluginSystemState,
    pub(crate) revision: u64,
    pub(crate) installed_at: OffsetDateTime,
    pub(crate) updated_at: OffsetDateTime,
}

impl PluginInstallation {
    #[must_use]
    pub fn id(&self) -> &str {
        &self.id
    }

    #[must_use]
    pub fn plugin_key(&self) -> &str {
        &self.plugin_key
    }

    #[must_use]
    pub fn version(&self) -> &str {
        &self.version
    }

    #[must_use]
    pub fn abi_version(&self) -> &str {
        &self.abi_version
    }

    #[must_use]
    pub fn component_digest(&self) -> &str {
        &self.component_digest
    }

    #[must_use]
    pub const fn system_state(&self) -> PluginSystemState {
        self.system_state
    }

    #[must_use]
    pub const fn revision(&self) -> u64 {
        self.revision
    }

    #[must_use]
    pub const fn installed_at(&self) -> OffsetDateTime {
        self.installed_at
    }

    #[must_use]
    pub const fn updated_at(&self) -> OffsetDateTime {
        self.updated_at
    }
}

impl fmt::Debug for PluginInstallation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PluginInstallation")
            .field("plugin_key", &self.plugin_key)
            .field("version", &self.version)
            .field("abi_version", &self.abi_version)
            .field("system_state", &self.system_state)
            .field("revision", &self.revision)
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Eq, PartialEq)]
pub struct PluginConfig {
    pub(crate) id: String,
    pub(crate) plugin_id: String,
    pub(crate) owner_user_id: String,
    pub(crate) is_enabled: bool,
    pub(crate) revision: u64,
    pub(crate) config: AiContentConfig,
    pub(crate) created_at: OffsetDateTime,
    pub(crate) updated_at: OffsetDateTime,
}

impl PluginConfig {
    #[must_use]
    pub fn id(&self) -> &str {
        &self.id
    }

    #[must_use]
    pub fn plugin_id(&self) -> &str {
        &self.plugin_id
    }

    #[must_use]
    pub fn owner_user_id(&self) -> &str {
        &self.owner_user_id
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
    pub fn config_hash(&self) -> &str {
        self.config.config_hash()
    }

    #[must_use]
    pub fn canonical_json(&self) -> &str {
        self.config.canonical_json()
    }

    #[must_use]
    pub fn config(&self) -> &AiContentConfig {
        &self.config
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

impl fmt::Debug for PluginConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PluginConfig")
            .field("is_enabled", &self.is_enabled)
            .field("revision", &self.revision)
            .finish_non_exhaustive()
    }
}

pub struct CapabilityGrantInput {
    pub plugin_key: String,
    pub owner_user_id: String,
    pub expected_revision: Option<u64>,
    pub capability: String,
    pub operation: String,
    pub resource_type: String,
    pub resource_id: String,
    pub constraints_json: Vec<u8>,
}

impl fmt::Debug for CapabilityGrantInput {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CapabilityGrantInput")
            .field("capability", &self.capability)
            .field("operation", &self.operation)
            .field("resource_type", &self.resource_type)
            .field("expected_revision", &self.expected_revision)
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Eq, PartialEq)]
pub struct PluginCapabilityGrant {
    pub(crate) id: String,
    pub(crate) plugin_id: String,
    pub(crate) owner_user_id: String,
    pub(crate) capability: String,
    pub(crate) operation: String,
    pub(crate) resource_type: String,
    pub(crate) resource_id: String,
    pub(crate) grant_key_hash: String,
    pub(crate) constraints_json: String,
    pub(crate) revision: u64,
    pub(crate) created_at: OffsetDateTime,
    pub(crate) updated_at: OffsetDateTime,
    pub(crate) revoked_at: Option<OffsetDateTime>,
}

impl PluginCapabilityGrant {
    #[must_use]
    pub fn id(&self) -> &str {
        &self.id
    }

    #[must_use]
    pub fn plugin_id(&self) -> &str {
        &self.plugin_id
    }

    #[must_use]
    pub fn owner_user_id(&self) -> &str {
        &self.owner_user_id
    }

    #[must_use]
    pub fn capability(&self) -> &str {
        &self.capability
    }

    #[must_use]
    pub fn operation(&self) -> &str {
        &self.operation
    }

    #[must_use]
    pub fn resource_type(&self) -> &str {
        &self.resource_type
    }

    #[must_use]
    pub fn resource_id(&self) -> &str {
        &self.resource_id
    }

    #[must_use]
    pub fn grant_key_hash(&self) -> &str {
        &self.grant_key_hash
    }

    #[must_use]
    pub fn constraints_json(&self) -> &str {
        &self.constraints_json
    }

    #[must_use]
    pub const fn revision(&self) -> u64 {
        self.revision
    }

    #[must_use]
    pub const fn is_revoked(&self) -> bool {
        self.revoked_at.is_some()
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

impl fmt::Debug for PluginCapabilityGrant {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PluginCapabilityGrant")
            .field("capability", &self.capability)
            .field("operation", &self.operation)
            .field("resource_type", &self.resource_type)
            .field("revision", &self.revision)
            .field("is_revoked", &self.is_revoked())
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Eq, PartialEq)]
pub struct PluginKvValue {
    pub(crate) plugin_id: String,
    pub(crate) owner_user_id: String,
    pub(crate) key: String,
    pub(crate) value: Vec<u8>,
    pub(crate) revision: u64,
    pub(crate) created_at: OffsetDateTime,
    pub(crate) updated_at: OffsetDateTime,
}

impl PluginKvValue {
    #[must_use]
    pub fn plugin_id(&self) -> &str {
        &self.plugin_id
    }

    #[must_use]
    pub fn owner_user_id(&self) -> &str {
        &self.owner_user_id
    }

    #[must_use]
    pub fn key(&self) -> &str {
        &self.key
    }

    #[must_use]
    pub fn value(&self) -> &[u8] {
        &self.value
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

impl fmt::Debug for PluginKvValue {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PluginKvValue")
            .field("key", &self.key)
            .field("value_size_bytes", &self.value.len())
            .field("revision", &self.revision)
            .finish_non_exhaustive()
    }
}
