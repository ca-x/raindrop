use std::{net::SocketAddr, path::PathBuf};

use secrecy::SecretString;
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

#[derive(Debug)]
pub struct RuntimeConfig {
    pub bind: SocketAddr,
    pub public_url: Option<Url>,
    pub data_dir: PathBuf,
    pub database_url: Option<SecretString>,
    pub session_secret: Option<SecretString>,
    pub bootstrap_admin: Option<BootstrapAdmin>,
    database_kind: Option<DatabaseKind>,
}

impl RuntimeConfig {
    #[must_use]
    pub const fn database_kind(&self) -> Option<DatabaseKind> {
        self.database_kind
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
            bootstrap_admin,
            database_kind,
        }
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
