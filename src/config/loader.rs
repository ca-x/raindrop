use std::{env, fs, net::SocketAddr, path::Path, time::Duration};

use url::Url;
use uuid::Uuid;

use crate::content::provider::ProviderSecretKeyring;

use super::{
    model::{
        BootstrapAdmin, BootstrapMode, ConfigArgs, ConfigError, ConfigSources, DatabaseKind,
        FeedRetentionConfig, FileBootstrapAdmin, FileConfig, LoadedConfig, RuntimeConfig,
    },
    redact::secret,
};

const DEFAULT_BIND: &str = "0.0.0.0:8080";
const DEFAULT_FEED_ORPHAN_RETENTION_DAYS: u32 = 30;
const MAX_FEED_ORPHAN_RETENTION_DAYS: u32 = 3_650;
const SECONDS_PER_DAY: u64 = 86_400;

pub trait EnvSource {
    fn get(&self, key: &str) -> Option<String>;
}

pub struct SystemEnv;

impl EnvSource for SystemEnv {
    fn get(&self, key: &str) -> Option<String> {
        env::var(key).ok()
    }
}

pub fn load(args: &ConfigArgs, env: &impl EnvSource) -> Result<LoadedConfig, ConfigError> {
    let data_dir = env
        .get("RAINDROP_DATA_DIR")
        .map_or_else(|| args.data_dir.clone(), Into::into);
    let config_path = args
        .config_path
        .clone()
        .unwrap_or_else(|| data_dir.join("config.toml"));
    let (file, loaded_config_path) = load_file(&config_path)?;

    let bind_source = env
        .get("RAINDROP_BIND")
        .or(file.bind)
        .unwrap_or_else(|| DEFAULT_BIND.to_owned());
    let bind = bind_source
        .parse::<SocketAddr>()
        .map_err(|_| ConfigError::InvalidValue {
            name: "RAINDROP_BIND",
            reason: "expected an IP:port socket address",
        })?;

    let public_url = env
        .get("RAINDROP_PUBLIC_URL")
        .or(file.public_url)
        .map(|value| parse_public_url(&value))
        .transpose()?;

    let database_from_env = env.get("RAINDROP_DATABASE_URL").is_some();
    let database_value = env.get("RAINDROP_DATABASE_URL").or(file.database_url);
    let database_kind = database_value
        .as_deref()
        .map(|value| DatabaseKind::parse(value, "RAINDROP_DATABASE_URL"))
        .transpose()?;
    let database_url = database_value.map(secret);

    let session_secret = env
        .get("RAINDROP_SESSION_SECRET")
        .or(file.session_secret)
        .map(validate_session_secret)
        .transpose()?;
    let provider_secret_keys = load_provider_secret_keys(env, file.provider_secret_keys)?;
    let feed_retention = load_feed_retention(env, file.feed_orphan_retention_days)?;
    let bootstrap_admin = load_bootstrap_admin(env, file.bootstrap_admin)?;

    let mode = if database_url.is_some() {
        BootstrapMode::Ready
    } else {
        BootstrapMode::SetupRequired {
            token: new_setup_token(),
        }
    };

    Ok(LoadedConfig {
        runtime: RuntimeConfig::new(
            bind,
            public_url,
            data_dir,
            database_url,
            session_secret,
            bootstrap_admin,
            database_kind,
        )
        .with_provider_secret_keys(provider_secret_keys)
        .with_feed_retention(feed_retention),
        mode,
        sources: ConfigSources {
            config_path: loaded_config_path,
            database_from_env,
        },
    })
}

fn load_provider_secret_keys(
    env: &impl EnvSource,
    file_entries: Option<Vec<String>>,
) -> Result<Vec<secrecy::SecretString>, ConfigError> {
    let (entries, source_name, configured) = match env.get("RAINDROP_PROVIDER_SECRET_KEYS") {
        Some(value) => (
            value.split(',').map(str::to_owned).collect::<Vec<_>>(),
            "RAINDROP_PROVIDER_SECRET_KEYS",
            true,
        ),
        None => {
            let configured = file_entries.is_some();
            (
                file_entries.unwrap_or_default(),
                "provider_secret_keys",
                configured,
            )
        }
    };
    if !configured {
        return Ok(Vec::new());
    }
    let entries = entries.into_iter().map(secret).collect::<Vec<_>>();
    ProviderSecretKeyring::validate_entries(&entries).map_err(|_| ConfigError::InvalidValue {
        name: source_name,
        reason: "expected active-first key-id:base64url entries with unique 32-byte keys",
    })?;
    Ok(entries)
}

fn load_feed_retention(
    env: &impl EnvSource,
    file_days: Option<u32>,
) -> Result<FeedRetentionConfig, ConfigError> {
    let (days, name) = match env.get("RAINDROP_FEED_ORPHAN_RETENTION_DAYS") {
        Some(value) => (
            value
                .parse::<u32>()
                .map_err(|_| ConfigError::InvalidValue {
                    name: "RAINDROP_FEED_ORPHAN_RETENTION_DAYS",
                    reason: "expected integer days from 0 to 3650",
                })?,
            "RAINDROP_FEED_ORPHAN_RETENTION_DAYS",
        ),
        None => (
            file_days.unwrap_or(DEFAULT_FEED_ORPHAN_RETENTION_DAYS),
            "feed_orphan_retention_days",
        ),
    };
    if days > MAX_FEED_ORPHAN_RETENTION_DAYS {
        return Err(ConfigError::InvalidValue {
            name,
            reason: "expected integer days from 0 to 3650",
        });
    }
    Ok(FeedRetentionConfig {
        orphan_grace: (days > 0).then(|| Duration::from_secs(u64::from(days) * SECONDS_PER_DAY)),
    })
}

fn load_file(path: &Path) -> Result<(FileConfig, Option<std::path::PathBuf>), ConfigError> {
    if !path.exists() {
        return Ok((FileConfig::default(), None));
    }

    let content = fs::read_to_string(path).map_err(|source| ConfigError::ReadFile {
        path: path.to_owned(),
        source,
    })?;
    let file = toml::from_str(&content).map_err(|mut source| {
        source.set_input(None);
        ConfigError::ParseFile {
            path: path.to_owned(),
            source,
        }
    })?;
    Ok((file, Some(path.to_owned())))
}

fn parse_public_url(value: &str) -> Result<Url, ConfigError> {
    let url = Url::parse(value).map_err(|_| ConfigError::InvalidValue {
        name: "RAINDROP_PUBLIC_URL",
        reason: "expected an absolute http:// or https:// URL",
    })?;
    if matches!(url.scheme(), "http" | "https") && url.host().is_some() {
        Ok(url)
    } else {
        Err(ConfigError::InvalidValue {
            name: "RAINDROP_PUBLIC_URL",
            reason: "expected an absolute http:// or https:// URL",
        })
    }
}

fn validate_session_secret(value: String) -> Result<secrecy::SecretString, ConfigError> {
    if value.len() < 32 {
        return Err(ConfigError::InvalidValue {
            name: "RAINDROP_SESSION_SECRET",
            reason: "expected at least 32 bytes",
        });
    }
    Ok(secret(value))
}

fn load_bootstrap_admin(
    env: &impl EnvSource,
    file: Option<FileBootstrapAdmin>,
) -> Result<Option<BootstrapAdmin>, ConfigError> {
    let file = file.unwrap_or_default();
    let username = env
        .get("RAINDROP_BOOTSTRAP_ADMIN_USERNAME")
        .or(file.username);
    let password = env
        .get("RAINDROP_BOOTSTRAP_ADMIN_PASSWORD")
        .or(file.password);
    let email = env.get("RAINDROP_BOOTSTRAP_ADMIN_EMAIL").or(file.email);

    if username.is_none() && password.is_none() && email.is_none() {
        return Ok(None);
    }
    let username = username.ok_or(ConfigError::IncompleteBootstrapAdmin {
        missing: "RAINDROP_BOOTSTRAP_ADMIN_USERNAME",
    })?;
    let password = password.ok_or(ConfigError::IncompleteBootstrapAdmin {
        missing: "RAINDROP_BOOTSTRAP_ADMIN_PASSWORD",
    })?;
    if username.trim().is_empty() {
        return Err(ConfigError::InvalidValue {
            name: "RAINDROP_BOOTSTRAP_ADMIN_USERNAME",
            reason: "expected a non-empty username",
        });
    }
    if password.is_empty() {
        return Err(ConfigError::InvalidValue {
            name: "RAINDROP_BOOTSTRAP_ADMIN_PASSWORD",
            reason: "expected a non-empty password",
        });
    }

    Ok(Some(BootstrapAdmin {
        username: username.trim().to_owned(),
        password: secret(password),
        email: email.and_then(|value| {
            let trimmed = value.trim();
            (!trimmed.is_empty()).then(|| trimmed.to_owned())
        }),
    }))
}

pub fn new_setup_token() -> secrecy::SecretString {
    secret(format!(
        "rd_setup_{}{}",
        Uuid::new_v4().simple(),
        Uuid::new_v4().simple()
    ))
}
