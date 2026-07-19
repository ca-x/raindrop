use std::{fmt, net::SocketAddr, path::PathBuf, time::Duration};

use secrecy::{ExposeSecret, SecretString};
use serde::Deserialize;
use url::Url;

#[derive(Debug, Clone)]
pub struct ConfigArgs {
    pub data_dir: PathBuf,
    pub config_path: Option<PathBuf>,
}

impl ConfigArgs {
    #[must_use]
    pub fn for_test(data_dir: impl Into<PathBuf>) -> Self {
        Self {
            data_dir: data_dir.into(),
            config_path: None,
        }
    }
}

#[derive(Debug)]
pub struct LoadedConfig {
    pub runtime: RuntimeConfig,
    pub mode: BootstrapMode,
    pub sources: ConfigSources,
}

pub struct RuntimeConfig {
    pub bind: SocketAddr,
    pub public_url: Option<Url>,
    pub data_dir: PathBuf,
    pub database_url: Option<SecretString>,
    pub session_secret: Option<SecretString>,
    provider_secret_keys: Vec<SecretString>,
    pub bootstrap_admin: Option<BootstrapAdmin>,
    feed_retention: FeedRetentionConfig,
    database_kind: Option<DatabaseKind>,
}

impl RuntimeConfig {
    #[must_use]
    pub const fn database_kind(&self) -> Option<DatabaseKind> {
        self.database_kind
    }

    #[must_use]
    pub const fn feed_retention(&self) -> FeedRetentionConfig {
        self.feed_retention
    }

    #[must_use]
    pub fn provider_secret_keys(&self) -> &[SecretString] {
        &self.provider_secret_keys
    }

    pub fn take_provider_secret_keys(&mut self) -> Vec<SecretString> {
        std::mem::take(&mut self.provider_secret_keys)
    }

    pub(crate) const fn new(
        bind: SocketAddr,
        public_url: Option<Url>,
        data_dir: PathBuf,
        database_url: Option<SecretString>,
        session_secret: Option<SecretString>,
        bootstrap_admin: Option<BootstrapAdmin>,
        database_kind: Option<DatabaseKind>,
    ) -> Self {
        Self {
            bind,
            public_url,
            data_dir,
            database_url,
            session_secret,
            provider_secret_keys: Vec::new(),
            bootstrap_admin,
            feed_retention: FeedRetentionConfig::DEFAULT,
            database_kind,
        }
    }

    pub(crate) const fn with_feed_retention(mut self, feed_retention: FeedRetentionConfig) -> Self {
        self.feed_retention = feed_retention;
        self
    }

    pub(crate) fn with_provider_secret_keys(
        mut self,
        provider_secret_keys: Vec<SecretString>,
    ) -> Self {
        self.provider_secret_keys = provider_secret_keys;
        self
    }
}

impl fmt::Debug for RuntimeConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let provider_secret_key_ids = self
            .provider_secret_keys
            .iter()
            .filter_map(|entry| entry.expose_secret().split_once(':').map(|(id, _)| id))
            .collect::<Vec<_>>();
        formatter
            .debug_struct("RuntimeConfig")
            .field("bind", &self.bind)
            .field("public_url", &self.public_url)
            .field("data_dir", &self.data_dir)
            .field("database_url", &self.database_url)
            .field("session_secret", &self.session_secret)
            .field("provider_secret_key_ids", &provider_secret_key_ids)
            .field(
                "provider_secret_key_count",
                &self.provider_secret_keys.len(),
            )
            .field("bootstrap_admin", &self.bootstrap_admin)
            .field("feed_retention", &self.feed_retention)
            .field("database_kind", &self.database_kind)
            .finish()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FeedRetentionConfig {
    pub orphan_grace: Option<Duration>,
}

impl FeedRetentionConfig {
    pub(crate) const DEFAULT: Self = Self {
        orphan_grace: Some(Duration::from_secs(30 * 86_400)),
    };
}

impl Default for FeedRetentionConfig {
    fn default() -> Self {
        Self::DEFAULT
    }
}

#[derive(Debug)]
pub struct BootstrapAdmin {
    pub username: String,
    pub password: SecretString,
    pub email: Option<String>,
}

#[derive(Debug)]
pub enum BootstrapMode {
    SetupRequired { token: SecretString },
    Ready,
}

#[derive(Debug)]
pub struct ConfigSources {
    pub config_path: Option<PathBuf>,
    pub database_from_env: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DatabaseKind {
    Sqlite,
    Postgres,
    MySql,
}

impl DatabaseKind {
    pub(crate) fn parse(value: &str, source: &'static str) -> Result<Self, ConfigError> {
        let scheme = value
            .split_once(':')
            .map(|(scheme, _)| scheme.to_ascii_lowercase())
            .ok_or(ConfigError::InvalidValue {
                name: source,
                reason: "expected sqlite://, postgres://, postgresql://, or mysql:// URL",
            })?;

        match scheme.as_str() {
            "sqlite" => Ok(Self::Sqlite),
            "postgres" | "postgresql" => Ok(Self::Postgres),
            "mysql" => Ok(Self::MySql),
            _ => Err(ConfigError::InvalidValue {
                name: source,
                reason: "expected sqlite://, postgres://, postgresql://, or mysql:// URL",
            }),
        }
    }
}

#[derive(Debug, Deserialize, Default)]
pub(crate) struct FileConfig {
    pub bind: Option<String>,
    pub public_url: Option<String>,
    pub database_url: Option<String>,
    pub session_secret: Option<String>,
    pub provider_secret_keys: Option<Vec<String>>,
    pub feed_orphan_retention_days: Option<u32>,
    pub bootstrap_admin: Option<FileBootstrapAdmin>,
}

#[derive(Debug, Deserialize, Default)]
pub(crate) struct FileBootstrapAdmin {
    pub username: Option<String>,
    pub password: Option<String>,
    pub email: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("failed to read configuration file {path}: {source}")]
    ReadFile {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to parse configuration file {path}: {source}")]
    ParseFile {
        path: PathBuf,
        #[source]
        source: toml::de::Error,
    },
    #[error("{name} is invalid: {reason}")]
    InvalidValue {
        name: &'static str,
        reason: &'static str,
    },
    #[error("{missing} is required when bootstrap admin configuration is present")]
    IncompleteBootstrapAdmin { missing: &'static str },
}
