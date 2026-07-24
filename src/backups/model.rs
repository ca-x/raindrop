use std::{fmt, net::IpAddr, str::FromStr};

use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use url::Url;
use uuid::Uuid;

use crate::feeds::{AddressDecision, AddressPolicy};

const MAX_DISPLAY_NAME_CHARS: usize = 80;
const MAX_ENDPOINT_BYTES: usize = 2_048;
const MAX_PREFIX_BYTES: usize = 512;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum BackupTargetKind {
    S3,
    Webdav,
}

impl BackupTargetKind {
    pub const fn as_storage(self) -> &'static str {
        match self {
            Self::S3 => "S3",
            Self::Webdav => "WEBDAV",
        }
    }

    pub fn parse_storage(value: &str) -> Result<Self, BackupError> {
        match value {
            "S3" => Ok(Self::S3),
            "WEBDAV" => Ok(Self::Webdav),
            _ => Err(BackupError::new(BackupErrorKind::CorruptData)),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct S3PublicConfig {
    pub endpoint: String,
    pub region: String,
    pub bucket: String,
    pub prefix: String,
    pub path_style: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WebDavPublicConfig {
    pub endpoint: String,
    pub prefix: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    content = "settings",
    rename_all = "SCREAMING_SNAKE_CASE"
)]
pub enum BackupPublicConfig {
    S3(S3PublicConfig),
    Webdav(WebDavPublicConfig),
}

impl BackupPublicConfig {
    pub const fn kind(&self) -> BackupTargetKind {
        match self {
            Self::S3(_) => BackupTargetKind::S3,
            Self::Webdav(_) => BackupTargetKind::Webdav,
        }
    }

    pub fn validate_and_normalize(self) -> Result<Self, BackupError> {
        match self {
            Self::S3(mut value) => {
                value.endpoint = normalize_endpoint(&value.endpoint)?;
                value.region = normalize_region(&value.region)?;
                value.bucket = normalize_bucket(&value.bucket)?;
                value.prefix = normalize_prefix(&value.prefix)?;
                Ok(Self::S3(value))
            }
            Self::Webdav(mut value) => {
                value.endpoint = normalize_endpoint(&value.endpoint)?;
                value.prefix = normalize_prefix(&value.prefix)?;
                Ok(Self::Webdav(value))
            }
        }
    }

    pub fn endpoint(&self) -> &str {
        match self {
            Self::S3(value) => &value.endpoint,
            Self::Webdav(value) => &value.endpoint,
        }
    }

    pub fn prefix(&self) -> &str {
        match self {
            Self::S3(value) => &value.prefix,
            Self::Webdav(value) => &value.prefix,
        }
    }
}

#[derive(Clone)]
pub struct S3SecretConfig {
    pub access_key_id: SecretString,
    pub secret_access_key: SecretString,
    pub session_token: Option<SecretString>,
}

#[derive(Clone)]
pub struct WebDavSecretConfig {
    pub username: SecretString,
    pub password: SecretString,
}

#[derive(Clone)]
pub enum BackupSecretConfig {
    S3(S3SecretConfig),
    Webdav(WebDavSecretConfig),
}

impl BackupSecretConfig {
    pub const fn kind(&self) -> BackupTargetKind {
        match self {
            Self::S3(_) => BackupTargetKind::S3,
            Self::Webdav(_) => BackupTargetKind::Webdav,
        }
    }

    pub fn validate(&self) -> Result<(), BackupError> {
        match self {
            Self::S3(value) => {
                validate_secret(value.access_key_id.expose_secret(), 1, 256)?;
                validate_secret(value.secret_access_key.expose_secret(), 1, 512)?;
                if let Some(token) = &value.session_token {
                    validate_secret(token.expose_secret(), 1, 4_096)?;
                }
            }
            Self::Webdav(value) => {
                validate_secret(value.username.expose_secret(), 1, 512)?;
                validate_secret(value.password.expose_secret(), 1, 4_096)?;
            }
        }
        Ok(())
    }

    pub(crate) fn to_secret_json(&self) -> Result<SecretString, BackupError> {
        let value = match self {
            Self::S3(secret) => serde_json::json!({
                "accessKeyId": secret.access_key_id.expose_secret(),
                "secretAccessKey": secret.secret_access_key.expose_secret(),
                "sessionToken": secret.session_token.as_ref().map(ExposeSecret::expose_secret),
            }),
            Self::Webdav(secret) => serde_json::json!({
                "username": secret.username.expose_secret(),
                "password": secret.password.expose_secret(),
            }),
        };
        serde_json::to_string(&value)
            .map(SecretString::from)
            .map_err(|_| BackupError::new(BackupErrorKind::InvalidInput))
    }

    pub(crate) fn from_secret_json(
        kind: BackupTargetKind,
        json: SecretString,
    ) -> Result<Self, BackupError> {
        let value: serde_json::Value = serde_json::from_str(json.expose_secret())
            .map_err(|_| BackupError::new(BackupErrorKind::SecretUnavailable))?;
        let object = value
            .as_object()
            .ok_or_else(|| BackupError::new(BackupErrorKind::SecretUnavailable))?;
        let secret = match kind {
            BackupTargetKind::S3 => Self::S3(S3SecretConfig {
                access_key_id: SecretString::from(required_string(object, "accessKeyId")?),
                secret_access_key: SecretString::from(required_string(object, "secretAccessKey")?),
                session_token: optional_string(object, "sessionToken")?.map(SecretString::from),
            }),
            BackupTargetKind::Webdav => Self::Webdav(WebDavSecretConfig {
                username: SecretString::from(required_string(object, "username")?),
                password: SecretString::from(required_string(object, "password")?),
            }),
        };
        secret.validate()?;
        Ok(secret)
    }
}

impl fmt::Debug for BackupSecretConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("BackupSecretConfig")
            .field("kind", &self.kind())
            .finish_non_exhaustive()
    }
}

fn required_string(
    object: &serde_json::Map<String, serde_json::Value>,
    field: &str,
) -> Result<String, BackupError> {
    object
        .get(field)
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| BackupError::new(BackupErrorKind::SecretUnavailable))
}

fn optional_string(
    object: &serde_json::Map<String, serde_json::Value>,
    field: &str,
) -> Result<Option<String>, BackupError> {
    match object.get(field) {
        None | Some(serde_json::Value::Null) => Ok(None),
        Some(value) => value
            .as_str()
            .map(|value| Some(value.to_owned()))
            .ok_or_else(|| BackupError::new(BackupErrorKind::SecretUnavailable)),
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RetentionPolicy {
    pub retain_count: Option<u16>,
    pub retain_days: Option<u16>,
}

impl RetentionPolicy {
    pub fn validate(self) -> Result<Self, BackupError> {
        if self
            .retain_count
            .is_some_and(|value| !(1..=1_000).contains(&value))
            || self
                .retain_days
                .is_some_and(|value| !(1..=3_650).contains(&value))
        {
            return Err(BackupError::new(BackupErrorKind::InvalidInput));
        }
        Ok(self)
    }
}

pub struct CreateBackupTarget {
    pub display_name: String,
    pub enabled: bool,
    pub config: BackupPublicConfig,
    pub secret: BackupSecretConfig,
    pub retention: RetentionPolicy,
}

pub struct UpdateBackupTarget {
    pub display_name: String,
    pub enabled: bool,
    pub config: BackupPublicConfig,
    pub secret: Option<BackupSecretConfig>,
    pub retention: RetentionPolicy,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupTarget {
    pub target_id: String,
    pub display_name: String,
    pub enabled: bool,
    pub config: BackupPublicConfig,
    pub retention: RetentionPolicy,
    pub revision: i64,
    pub has_credentials: bool,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    pub updated_at: OffsetDateTime,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupSchedule {
    pub enabled: bool,
    pub interval_hours: u16,
    pub target_ids: Vec<String>,
    #[serde(with = "time::serde::rfc3339::option")]
    pub next_run_at: Option<OffsetDateTime>,
    pub revision: i64,
}

impl Default for BackupSchedule {
    fn default() -> Self {
        Self {
            enabled: false,
            interval_hours: 24,
            target_ids: Vec::new(),
            next_run_at: None,
            revision: 0,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum BackupTriggerKind {
    Manual,
    Scheduled,
}

impl BackupTriggerKind {
    pub const fn as_storage(self) -> &'static str {
        match self {
            Self::Manual => "MANUAL",
            Self::Scheduled => "SCHEDULED",
        }
    }

    pub fn parse_storage(value: &str) -> Result<Self, BackupError> {
        match value {
            "MANUAL" => Ok(Self::Manual),
            "SCHEDULED" => Ok(Self::Scheduled),
            _ => Err(BackupError::new(BackupErrorKind::CorruptData)),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum BackupJobStatus {
    Queued,
    Running,
    Succeeded,
    Partial,
    Failed,
}

impl BackupJobStatus {
    pub const fn as_storage(self) -> &'static str {
        match self {
            Self::Queued => "QUEUED",
            Self::Running => "RUNNING",
            Self::Succeeded => "SUCCEEDED",
            Self::Partial => "PARTIAL",
            Self::Failed => "FAILED",
        }
    }

    pub fn parse_storage(value: &str) -> Result<Self, BackupError> {
        match value {
            "QUEUED" => Ok(Self::Queued),
            "RUNNING" => Ok(Self::Running),
            "SUCCEEDED" => Ok(Self::Succeeded),
            "PARTIAL" => Ok(Self::Partial),
            "FAILED" => Ok(Self::Failed),
            _ => Err(BackupError::new(BackupErrorKind::CorruptData)),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum BackupJobTargetStatus {
    Queued,
    Running,
    Succeeded,
    Failed,
}

impl BackupJobTargetStatus {
    pub const fn as_storage(self) -> &'static str {
        match self {
            Self::Queued => "QUEUED",
            Self::Running => "RUNNING",
            Self::Succeeded => "SUCCEEDED",
            Self::Failed => "FAILED",
        }
    }

    pub fn parse_storage(value: &str) -> Result<Self, BackupError> {
        match value {
            "QUEUED" => Ok(Self::Queued),
            "RUNNING" => Ok(Self::Running),
            "SUCCEEDED" => Ok(Self::Succeeded),
            "FAILED" => Ok(Self::Failed),
            _ => Err(BackupError::new(BackupErrorKind::CorruptData)),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupJobTarget {
    pub target_result_id: String,
    pub target_id: Option<String>,
    pub target_kind: BackupTargetKind,
    pub target_name: String,
    pub status: BackupJobTargetStatus,
    pub byte_size: Option<u64>,
    pub error_code: Option<String>,
    #[serde(with = "time::serde::rfc3339::option")]
    pub started_at: Option<OffsetDateTime>,
    #[serde(with = "time::serde::rfc3339::option")]
    pub completed_at: Option<OffsetDateTime>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupJob {
    pub job_id: String,
    pub trigger_kind: BackupTriggerKind,
    pub status: BackupJobStatus,
    pub target_count: u16,
    pub last_error_code: Option<String>,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339::option")]
    pub started_at: Option<OffsetDateTime>,
    #[serde(with = "time::serde::rfc3339::option")]
    pub completed_at: Option<OffsetDateTime>,
    pub targets: Vec<BackupJobTarget>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BackupErrorKind {
    InvalidInput,
    NotFound,
    Conflict,
    SecretUnavailable,
    TargetChanged,
    TargetUnreachable,
    TargetAuthentication,
    TargetProtocol,
    ExportFailed,
    LeaseLost,
    CorruptData,
    Database,
}

#[derive(Debug)]
pub struct BackupError {
    kind: BackupErrorKind,
}

impl BackupError {
    pub const fn new(kind: BackupErrorKind) -> Self {
        Self { kind }
    }

    pub const fn kind(&self) -> BackupErrorKind {
        self.kind
    }

    pub const fn public_code(&self) -> &'static str {
        match self.kind {
            BackupErrorKind::InvalidInput => "VALIDATION_ERROR",
            BackupErrorKind::NotFound => "NOT_FOUND",
            BackupErrorKind::Conflict => "CONFLICT",
            BackupErrorKind::SecretUnavailable => "BACKUP_KEYRING_UNAVAILABLE",
            BackupErrorKind::TargetChanged => "TARGET_CHANGED",
            BackupErrorKind::TargetUnreachable => "TARGET_UNREACHABLE",
            BackupErrorKind::TargetAuthentication => "TARGET_AUTH_FAILED",
            BackupErrorKind::TargetProtocol => "TARGET_PROTOCOL_ERROR",
            BackupErrorKind::ExportFailed => "BACKUP_EXPORT_FAILED",
            BackupErrorKind::LeaseLost => "LEASE_LOST",
            BackupErrorKind::CorruptData | BackupErrorKind::Database => "INTERNAL_ERROR",
        }
    }
}

impl fmt::Display for BackupError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.public_code())
    }
}

impl std::error::Error for BackupError {}

pub(crate) fn normalize_display_name(value: &str) -> Result<String, BackupError> {
    let value = value.trim();
    if value.is_empty()
        || value.chars().count() > MAX_DISPLAY_NAME_CHARS
        || value.chars().any(char::is_control)
    {
        return Err(BackupError::new(BackupErrorKind::InvalidInput));
    }
    Ok(value.to_owned())
}

pub(crate) fn validate_id(value: &str) -> Result<(), BackupError> {
    let parsed =
        Uuid::parse_str(value).map_err(|_| BackupError::new(BackupErrorKind::InvalidInput))?;
    if parsed.to_string() != value {
        return Err(BackupError::new(BackupErrorKind::InvalidInput));
    }
    Ok(())
}

fn normalize_endpoint(value: &str) -> Result<String, BackupError> {
    if value.len() > MAX_ENDPOINT_BYTES {
        return Err(BackupError::new(BackupErrorKind::InvalidInput));
    }
    let mut url =
        Url::parse(value.trim()).map_err(|_| BackupError::new(BackupErrorKind::InvalidInput))?;
    if url.scheme() != "https"
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(BackupError::new(BackupErrorKind::InvalidInput));
    }
    if let Some(host) = url.host_str()
        && let Ok(address) = IpAddr::from_str(host)
        && AddressPolicy::public_only().classify(address) != AddressDecision::Allowed
    {
        return Err(BackupError::new(BackupErrorKind::InvalidInput));
    }
    let path = normalize_prefix(url.path())?;
    let normalized_path = if path.is_empty() {
        "/".to_owned()
    } else {
        format!("/{path}/")
    };
    url.set_path(&normalized_path);
    Ok(url.to_string())
}

fn normalize_prefix(value: &str) -> Result<String, BackupError> {
    let value = value.trim().trim_matches('/');
    if value.len() > MAX_PREFIX_BYTES
        || value.contains('\\')
        || value.contains('?')
        || value.contains('#')
        || value.chars().any(char::is_control)
    {
        return Err(BackupError::new(BackupErrorKind::InvalidInput));
    }
    let mut segments = Vec::new();
    for segment in value.split('/').filter(|segment| !segment.is_empty()) {
        if matches!(segment, "." | "..") {
            return Err(BackupError::new(BackupErrorKind::InvalidInput));
        }
        segments.push(segment);
    }
    Ok(segments.join("/"))
}

fn normalize_region(value: &str) -> Result<String, BackupError> {
    let value = value.trim();
    if value.is_empty()
        || value.len() > 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
    {
        return Err(BackupError::new(BackupErrorKind::InvalidInput));
    }
    Ok(value.to_owned())
}

fn normalize_bucket(value: &str) -> Result<String, BackupError> {
    let value = value.trim();
    let bytes = value.as_bytes();
    if !(3..=63).contains(&bytes.len())
        || !bytes.first().is_some_and(u8::is_ascii_alphanumeric)
        || !bytes.last().is_some_and(u8::is_ascii_alphanumeric)
        || !bytes.iter().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'.' | b'-')
        })
        || value.contains("..")
        || value.parse::<IpAddr>().is_ok()
    {
        return Err(BackupError::new(BackupErrorKind::InvalidInput));
    }
    Ok(value.to_owned())
}

fn validate_secret(value: &str, minimum: usize, maximum: usize) -> Result<(), BackupError> {
    if !(minimum..=maximum).contains(&value.len()) || value.chars().any(char::is_control) {
        return Err(BackupError::new(BackupErrorKind::InvalidInput));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn public_configs_are_https_normalized_and_prefix_safe() {
        let normalized = BackupPublicConfig::S3(S3PublicConfig {
            endpoint: "https://s3.example/base".to_owned(),
            region: "us-east-1".to_owned(),
            bucket: "reader-backups".to_owned(),
            prefix: "/home/reader/".to_owned(),
            path_style: true,
        })
        .validate_and_normalize()
        .unwrap();
        let BackupPublicConfig::S3(value) = normalized else {
            panic!("S3 config expected")
        };
        assert_eq!(value.endpoint, "https://s3.example/base/");
        assert_eq!(value.prefix, "home/reader");

        for endpoint in [
            "http://s3.example",
            "https://user:pass@s3.example",
            "https://127.0.0.1",
        ] {
            assert!(
                BackupPublicConfig::Webdav(WebDavPublicConfig {
                    endpoint: endpoint.to_owned(),
                    prefix: String::new(),
                })
                .validate_and_normalize()
                .is_err()
            );
        }
    }

    #[test]
    fn secret_debug_never_exposes_values() {
        let secret = BackupSecretConfig::S3(S3SecretConfig {
            access_key_id: SecretString::from("access-key"),
            secret_access_key: SecretString::from("secret-key"),
            session_token: None,
        });
        let debug = format!("{secret:?}");
        assert!(!debug.contains("access-key"));
        assert!(!debug.contains("secret-key"));
    }
}
