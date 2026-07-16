use std::{env, fs, net::SocketAddr, path::Path};

use url::Url;
use uuid::Uuid;

use super::{
    model::{
        BootstrapAdmin, BootstrapMode, ConfigArgs, ConfigError, ConfigSources, DatabaseKind,
        FileBootstrapAdmin, FileConfig, LoadedConfig, RuntimeConfig,
    },
    redact::secret,
};

const DEFAULT_BIND: &str = "0.0.0.0:8080";

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
    let bootstrap_admin = load_bootstrap_admin(env, file.bootstrap_admin)?;

    let mode = if database_url.is_some() {
        BootstrapMode::Ready
    } else {
        BootstrapMode::SetupRequired {
            token: secret(generate_setup_token()),
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
        ),
        mode,
        sources: ConfigSources {
            config_path: loaded_config_path,
            database_from_env,
        },
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
    let file = toml::from_str(&content).map_err(|source| ConfigError::ParseFile {
        path: path.to_owned(),
        source,
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
    if password.len() < 12 {
        return Err(ConfigError::InvalidValue {
            name: "RAINDROP_BOOTSTRAP_ADMIN_PASSWORD",
            reason: "expected at least 12 bytes",
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

fn generate_setup_token() -> String {
    format!(
        "rd_setup_{}{}",
        Uuid::new_v4().simple(),
        Uuid::new_v4().simple()
    )
}
