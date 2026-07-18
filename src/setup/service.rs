use std::{
    fs::{self, File, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU8, Ordering},
    },
};

use constant_time_eq::constant_time_eq;
use sea_orm::{ConnectionTrait, DatabaseConnection, Statement};
use secrecy::{ExposeSecret, SecretString};
use tokio::sync::Mutex;
use url::Url;
use uuid::Uuid;

use crate::{
    auth::{
        CreateAdminError, CreateAdminInput, PasswordService, SessionService, User, create_admin,
        validate_create_admin_input,
    },
    config::DatabaseKind,
    db::{DatabaseConfig, DbError, connect, migrate},
};

#[derive(Clone)]
pub struct SetupService {
    inner: Arc<SetupInner>,
}

struct SetupInner {
    data_dir: PathBuf,
    config_path: PathBuf,
    token_hash: blake3::Hash,
    public_url: Option<Url>,
    mode: AtomicU8,
    completion: Mutex<()>,
    sessions: SessionService,
    fail_directory_sync: AtomicBool,
    fail_after_durable_boundary: AtomicBool,
}

const MODE_FULL: u8 = 0;
const MODE_ADMIN_ONLY: u8 = 1;
const MODE_READY: u8 = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SetupMode {
    Full,
    AdminOnly,
}

impl SetupService {
    #[must_use]
    pub fn required(
        data_dir: impl AsRef<Path>,
        token: SecretString,
        public_url: Option<Url>,
    ) -> Self {
        let data_dir = data_dir.as_ref().to_owned();
        Self {
            inner: Arc::new(SetupInner {
                config_path: data_dir.join("config.toml"),
                data_dir,
                token_hash: blake3::hash(token.expose_secret().as_bytes()),
                public_url,
                mode: AtomicU8::new(MODE_FULL),
                completion: Mutex::new(()),
                sessions: SessionService::unavailable(),
                fail_directory_sync: AtomicBool::new(false),
                fail_after_durable_boundary: AtomicBool::new(false),
            }),
        }
    }

    #[must_use]
    pub fn admin_only(
        data_dir: impl AsRef<Path>,
        token: SecretString,
        public_url: Option<Url>,
        database: DatabaseConnection,
    ) -> Self {
        let data_dir = data_dir.as_ref().to_owned();
        Self {
            inner: Arc::new(SetupInner {
                config_path: data_dir.join("config.toml"),
                data_dir,
                token_hash: blake3::hash(token.expose_secret().as_bytes()),
                public_url,
                mode: AtomicU8::new(MODE_ADMIN_ONLY),
                completion: Mutex::new(()),
                sessions: SessionService::new(database),
                fail_directory_sync: AtomicBool::new(false),
                fail_after_durable_boundary: AtomicBool::new(false),
            }),
        }
    }

    pub async fn from_configured_database(
        data_dir: impl AsRef<Path>,
        token: SecretString,
        public_url: Option<Url>,
        database: DatabaseConnection,
    ) -> Result<Self, SetupError> {
        match inspect_bootstrap_state(&database).await? {
            ConfiguredBootstrapState::Empty => {
                Ok(Self::admin_only(data_dir, token, public_url, database))
            }
            ConfiguredBootstrapState::Ready => Ok(Self::ready(data_dir, public_url, database)),
            ConfiguredBootstrapState::Inconsistent => Err(SetupError::InconsistentBootstrap),
        }
    }

    #[must_use]
    pub fn ready(
        data_dir: impl AsRef<Path>,
        public_url: Option<Url>,
        database: DatabaseConnection,
    ) -> Self {
        let data_dir = data_dir.as_ref().to_owned();
        Self {
            inner: Arc::new(SetupInner {
                config_path: data_dir.join("config.toml"),
                data_dir,
                token_hash: blake3::hash(b"setup-complete"),
                public_url,
                mode: AtomicU8::new(MODE_READY),
                completion: Mutex::new(()),
                sessions: SessionService::new(database),
                fail_directory_sync: AtomicBool::new(false),
                fail_after_durable_boundary: AtomicBool::new(false),
            }),
        }
    }

    #[must_use]
    pub fn is_ready(&self) -> bool {
        self.inner.mode.load(Ordering::Acquire) == MODE_READY
    }

    #[must_use]
    pub fn setup_mode(&self) -> Option<SetupMode> {
        match self.inner.mode.load(Ordering::Acquire) {
            MODE_FULL => Some(SetupMode::Full),
            MODE_ADMIN_ONLY => Some(SetupMode::AdminOnly),
            _ => None,
        }
    }

    #[must_use]
    pub fn secure_cookie(&self) -> bool {
        self.inner
            .public_url
            .as_ref()
            .is_some_and(|url| url.scheme() == "https")
    }

    #[must_use]
    pub fn sessions(&self) -> SessionService {
        self.inner.sessions.clone()
    }

    pub fn database(&self) -> Result<DatabaseConnection, SetupError> {
        self.inner
            .sessions
            .database()
            .map_err(|_| SetupError::NotReady)
    }

    pub fn require_token(&self, candidate: &str) -> Result<(), SetupError> {
        let candidate = blake3::hash(candidate.as_bytes());
        constant_time_eq(candidate.as_bytes(), self.inner.token_hash.as_bytes())
            .then_some(())
            .ok_or(SetupError::Unauthorized)
    }

    pub async fn database_check(
        &self,
        token: &str,
        database_url: &str,
    ) -> Result<DatabaseKind, SetupError> {
        self.require_token(token)?;
        self.require_mode(SetupMode::Full)?;
        prepare_data_dir(&self.inner.data_dir)?;
        let kind = validate_database_url(database_url)?;
        let database = connect_database(database_url).await?;
        database
            .close()
            .await
            .map_err(DbError::from)
            .map_err(SetupError::Database)?;
        Ok(kind)
    }

    pub async fn complete(
        &self,
        token: &str,
        input: SetupCompleteInput,
    ) -> Result<User, SetupError> {
        let _guard = self.inner.completion.lock().await;
        self.require_token(token)?;
        self.require_mode(SetupMode::Full)?;
        validate_database_url(input.database_url.expose_secret())?;

        let admin_input = CreateAdminInput {
            username: input.username,
            password: input.password,
            email: input.email,
        };
        validate_create_admin_input(&admin_input).map_err(SetupError::CreateAdmin)?;
        prepare_data_dir(&self.inner.data_dir)?;

        let database = connect_database(input.database_url.expose_secret()).await?;
        migrate(&database).await.map_err(SetupError::Database)?;
        match inspect_bootstrap_state(&database).await? {
            ConfiguredBootstrapState::Empty => {}
            ConfiguredBootstrapState::Ready => return Err(SetupError::AlreadyComplete),
            ConfiguredBootstrapState::Inconsistent => {
                return Err(SetupError::InconsistentBootstrap);
            }
        }

        let mut temporary = self.write_temporary_config(&input.database_url)?;
        let inject_directory_sync_failure =
            self.inner.fail_directory_sync.swap(false, Ordering::AcqRel);
        #[cfg(unix)]
        let durable_result = if inject_directory_sync_failure {
            durable_replace_with_directory_sync_failure(
                temporary.path(),
                &self.inner.config_path,
                &self.inner.data_dir,
            )
        } else {
            durable_replace(
                temporary.path(),
                &self.inner.config_path,
                &self.inner.data_dir,
            )
        };
        #[cfg(not(unix))]
        let durable_result = {
            let _ = inject_directory_sync_failure;
            durable_replace(
                temporary.path(),
                &self.inner.config_path,
                &self.inner.data_dir,
            )
        };
        if let Err(failure) = durable_result {
            return Err(temporary.cleanup_after_failure(failure));
        }
        temporary.disarm();
        self.inner.sessions.attach_database(database.clone());
        self.inner.mode.store(MODE_ADMIN_ONLY, Ordering::Release);
        if self
            .inner
            .fail_after_durable_boundary
            .swap(false, Ordering::AcqRel)
        {
            return Err(SetupError::InjectedFailure);
        }

        self.finish_administrator(&database, admin_input).await
    }

    pub async fn complete_admin(
        &self,
        token: &str,
        input: SetupAdminInput,
    ) -> Result<User, SetupError> {
        let _guard = self.inner.completion.lock().await;
        self.require_token(token)?;
        self.require_mode(SetupMode::AdminOnly)?;
        let input = CreateAdminInput {
            username: input.username,
            password: input.password,
            email: input.email,
        };
        validate_create_admin_input(&input).map_err(SetupError::CreateAdmin)?;
        let database = self.database()?;
        self.finish_administrator(&database, input).await
    }

    async fn finish_administrator(
        &self,
        database: &DatabaseConnection,
        input: CreateAdminInput,
    ) -> Result<User, SetupError> {
        match create_admin(database, &PasswordService::default(), input).await {
            Ok(admin) => {
                self.inner.mode.store(MODE_READY, Ordering::Release);
                Ok(admin)
            }
            Err(CreateAdminError::AlreadyClaimed) => {
                match inspect_bootstrap_state(database).await? {
                    ConfiguredBootstrapState::Ready => {
                        self.inner.mode.store(MODE_READY, Ordering::Release);
                        Err(SetupError::AlreadyComplete)
                    }
                    ConfiguredBootstrapState::Empty | ConfiguredBootstrapState::Inconsistent => {
                        Err(SetupError::InconsistentBootstrap)
                    }
                }
            }
            Err(error) => Err(SetupError::CreateAdmin(error)),
        }
    }

    fn require_mode(&self, expected: SetupMode) -> Result<(), SetupError> {
        match self.setup_mode() {
            None => Err(SetupError::AlreadyComplete),
            Some(actual) if actual == expected => Ok(()),
            Some(_) => Err(SetupError::WrongMode),
        }
    }

    fn write_temporary_config(
        &self,
        database_url: &SecretString,
    ) -> Result<TemporaryConfig, SetupError> {
        fs::create_dir_all(&self.inner.data_dir).map_err(SetupError::WriteConfig)?;
        let mut config = if self.inner.config_path.exists() {
            let content =
                fs::read_to_string(&self.inner.config_path).map_err(SetupError::WriteConfig)?;
            toml::from_str::<toml::Table>(&content).map_err(|mut source| {
                source.set_input(None);
                SetupError::ParseConfig(source)
            })?
        } else {
            toml::Table::new()
        };
        config.insert(
            "database_url".to_owned(),
            toml::Value::String(database_url.expose_secret().to_owned()),
        );
        let encoded = toml::to_string_pretty(&config).map_err(SetupError::SerializeConfig)?;
        let temporary_path = self
            .inner
            .data_dir
            .join(format!(".config.toml.{}.tmp", Uuid::new_v4().simple()));
        let mut file = private_file(&temporary_path).map_err(SetupError::WriteConfig)?;
        let mut temporary = TemporaryConfig::new(temporary_path);
        if let Err(source) = file.write_all(encoded.as_bytes()) {
            drop(file);
            return Err(
                temporary.cleanup_after_failure(ConfigFailure::new(ConfigOperation::Write, source))
            );
        }
        drop(file);
        if let Err(source) = ensure_private_permissions(temporary.path()) {
            return Err(temporary
                .cleanup_after_failure(ConfigFailure::new(ConfigOperation::Permissions, source)));
        }
        Ok(temporary)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ConfiguredBootstrapState {
    Empty,
    Ready,
    Inconsistent,
}

async fn inspect_bootstrap_state(
    database: &DatabaseConnection,
) -> Result<ConfiguredBootstrapState, SetupError> {
    // The user and claim counts must come from one database snapshot. Separate
    // autocommit reads can straddle a concurrent administrator transaction and
    // briefly combine the pre-commit user count with the post-commit claim.
    let row = database
        .query_one(Statement::from_string(
            database.get_database_backend(),
            "SELECT (SELECT COUNT(*) FROM users) AS user_count, \
                    (SELECT COUNT(*) FROM bootstrap_state WHERE id = 1) AS claim_count"
                .to_owned(),
        ))
        .await
        .map_err(DbError::from)
        .map_err(SetupError::Database)?
        .ok_or(SetupError::InconsistentBootstrap)?;
    let users: i64 = row
        .try_get("", "user_count")
        .map_err(|_| SetupError::InconsistentBootstrap)?;
    let claims: i64 = row
        .try_get("", "claim_count")
        .map_err(|_| SetupError::InconsistentBootstrap)?;
    Ok(match (users, claims) {
        (0, 0) => ConfiguredBootstrapState::Empty,
        (1.., 1) => ConfiguredBootstrapState::Ready,
        _ => ConfiguredBootstrapState::Inconsistent,
    })
}

pub struct SetupCompleteInput {
    pub database_url: SecretString,
    pub username: String,
    pub password: SecretString,
    pub email: Option<String>,
}

pub struct SetupAdminInput {
    pub username: String,
    pub password: SecretString,
    pub email: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum SetupError {
    #[error("setup token is invalid")]
    Unauthorized,
    #[error("setup is already complete")]
    AlreadyComplete,
    #[error("setup is not complete")]
    NotReady,
    #[error("setup operation is not available in the current mode")]
    WrongMode,
    #[error("configured bootstrap state is inconsistent")]
    InconsistentBootstrap,
    #[error("database URL is invalid")]
    InvalidDatabase,
    #[error("database is unavailable")]
    Database(#[source] DbError),
    #[error("administrator could not be created")]
    CreateAdmin(#[source] CreateAdminError),
    #[error("configuration could not be read")]
    ParseConfig(#[source] toml::de::Error),
    #[error("configuration could not be serialized")]
    SerializeConfig(#[source] toml::ser::Error),
    #[error("configuration could not be written")]
    WriteConfig(#[source] io::Error),
    #[error("injected failure after durable configuration")]
    InjectedFailure,
}

fn validate_database_url(database_url: &str) -> Result<DatabaseKind, SetupError> {
    if database_url.len() > 4096 {
        return Err(SetupError::InvalidDatabase);
    }
    DatabaseKind::parse(database_url, "databaseUrl").map_err(|_| SetupError::InvalidDatabase)
}

async fn connect_database(database_url: &str) -> Result<DatabaseConnection, SetupError> {
    connect(&DatabaseConfig::new(SecretString::from(
        database_url.to_owned(),
    )))
    .await
    .map_err(SetupError::Database)
}

fn private_file(path: &Path) -> io::Result<File> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path)
}

fn prepare_data_dir(path: &Path) -> Result<(), SetupError> {
    fs::create_dir_all(path).map_err(SetupError::WriteConfig)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))
            .map_err(SetupError::WriteConfig)?;
    }
    Ok(())
}

fn ensure_private_permissions(path: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    }
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ConfigOperation {
    Write,
    Sync,
    Permissions,
    Replace,
    #[cfg(unix)]
    DirectorySync,
    #[cfg(not(any(unix, windows)))]
    Unsupported,
}

impl std::fmt::Display for ConfigOperation {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let name = match self {
            Self::Write => "write",
            Self::Sync => "sync",
            Self::Permissions => "permissions",
            Self::Replace => "replace",
            #[cfg(unix)]
            Self::DirectorySync => "directory sync",
            #[cfg(not(any(unix, windows)))]
            Self::Unsupported => "unsupported platform",
        };
        formatter.write_str(name)
    }
}

#[derive(Debug)]
struct ConfigFailure {
    operation: ConfigOperation,
    source: io::Error,
}

impl ConfigFailure {
    fn new(operation: ConfigOperation, source: io::Error) -> Self {
        Self { operation, source }
    }

    fn into_redacted_io(self) -> io::Error {
        io::Error::new(
            self.source.kind(),
            format!(
                "configuration {} failed ({:?})",
                self.operation,
                self.source.kind()
            ),
        )
    }
}

impl std::fmt::Display for ConfigFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "configuration {} failed ({:?})",
            self.operation,
            self.source.kind()
        )
    }
}

struct TemporaryConfig {
    path: PathBuf,
    armed: bool,
}

impl TemporaryConfig {
    fn new(path: PathBuf) -> Self {
        Self { path, armed: true }
    }

    fn path(&self) -> &Path {
        &self.path
    }

    fn disarm(&mut self) {
        self.armed = false;
    }

    fn cleanup_after_failure(&mut self, failure: ConfigFailure) -> SetupError {
        match fs::remove_file(&self.path) {
            Ok(()) => {
                self.disarm();
                SetupError::WriteConfig(failure.into_redacted_io())
            }
            Err(cleanup) if cleanup.kind() == io::ErrorKind::NotFound => {
                self.disarm();
                SetupError::WriteConfig(failure.into_redacted_io())
            }
            Err(cleanup) => SetupError::WriteConfig(io::Error::new(
                failure.source.kind(),
                format!(
                    "configuration {} failed ({:?}); cleanup failed ({:?})",
                    failure.operation,
                    failure.source.kind(),
                    cleanup.kind()
                ),
            )),
        }
    }
}

impl Drop for TemporaryConfig {
    fn drop(&mut self) {
        if self.armed {
            let _ = fs::remove_file(&self.path);
        }
    }
}

fn durable_replace(temp: &Path, config: &Path, data_dir: &Path) -> Result<(), ConfigFailure> {
    sync_temporary_config(temp)?;

    #[cfg(unix)]
    {
        replace_and_sync_directory(temp, config, data_dir, sync_directory)
    }
    #[cfg(windows)]
    {
        let _ = data_dir;
        replace_windows(temp, config)
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = (config, data_dir);
        Err(ConfigFailure::new(
            ConfigOperation::Unsupported,
            io::Error::new(
                io::ErrorKind::Unsupported,
                "durable configuration replacement is unsupported on this platform",
            ),
        ))
    }
}

fn sync_temporary_config(temp: &Path) -> Result<(), ConfigFailure> {
    File::open(temp)
        .and_then(|file| file.sync_all())
        .map_err(|source| ConfigFailure::new(ConfigOperation::Sync, source))
}

#[cfg(unix)]
fn replace_and_sync_directory(
    temp: &Path,
    config: &Path,
    data_dir: &Path,
    sync: impl FnOnce(&Path) -> io::Result<()>,
) -> Result<(), ConfigFailure> {
    fs::rename(temp, config)
        .map_err(|source| ConfigFailure::new(ConfigOperation::Replace, source))?;
    sync(data_dir).map_err(|source| ConfigFailure::new(ConfigOperation::DirectorySync, source))
}

#[cfg(unix)]
fn durable_replace_with_directory_sync_failure(
    temp: &Path,
    config: &Path,
    data_dir: &Path,
) -> Result<(), ConfigFailure> {
    sync_temporary_config(temp)?;
    replace_and_sync_directory(temp, config, data_dir, |_| {
        Err(io::Error::other(
            "injected directory synchronization failure",
        ))
    })
}

#[cfg(unix)]
fn sync_directory(path: &Path) -> io::Result<()> {
    File::open(path)?.sync_all()
}

#[cfg(windows)]
fn replace_windows(temp: &Path, config: &Path) -> Result<(), ConfigFailure> {
    use std::os::windows::ffi::OsStrExt;

    use windows_sys::Win32::Storage::FileSystem::{
        MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
    };

    fn wide_path(path: &Path) -> io::Result<Vec<u16>> {
        let mut encoded = path.as_os_str().encode_wide().collect::<Vec<_>>();
        if encoded.contains(&0) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "configuration path contains an embedded null",
            ));
        }
        encoded.push(0);
        Ok(encoded)
    }

    let temp =
        wide_path(temp).map_err(|source| ConfigFailure::new(ConfigOperation::Replace, source))?;
    let config =
        wide_path(config).map_err(|source| ConfigFailure::new(ConfigOperation::Replace, source))?;
    let replaced = unsafe {
        MoveFileExW(
            temp.as_ptr(),
            config.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if replaced == 0 {
        return Err(ConfigFailure::new(
            ConfigOperation::Replace,
            io::Error::last_os_error(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use sea_orm::{EntityTrait, PaginatorTrait};
    use tempfile::tempdir;

    use super::*;
    use crate::db::entities::user;

    #[tokio::test]
    async fn durable_configuration_recovers_as_admin_only_then_ready_after_retry() {
        let data = tempdir().expect("temporary directory should be created");
        let database_url = format!(
            "sqlite://{}?mode=rwc",
            data.path().join("recovery.db").display()
        );
        let setup = SetupService::required(
            data.path(),
            SecretString::from("rd_setup_crash_token".to_owned()),
            None,
        );
        setup
            .inner
            .fail_after_durable_boundary
            .store(true, Ordering::Release);

        let error = setup
            .complete(
                "rd_setup_crash_token",
                SetupCompleteInput {
                    database_url: SecretString::from(database_url.clone()),
                    username: "Reader".to_owned(),
                    password: SecretString::from("correct horse battery staple".to_owned()),
                    email: None,
                },
            )
            .await
            .expect_err("injected crash should stop before administrator commit");
        assert!(matches!(error, SetupError::InjectedFailure));
        assert!(data.path().join("config.toml").exists());

        let database = connect_database(&database_url)
            .await
            .expect("database should reconnect");
        assert_eq!(
            user::Entity::find()
                .count(&database)
                .await
                .expect("users should count"),
            0
        );
        let restarted = SetupService::from_configured_database(
            data.path(),
            SecretString::from("rd_setup_restart_token".to_owned()),
            None,
            database.clone(),
        )
        .await
        .expect("configured service should recover");
        assert_eq!(restarted.setup_mode(), Some(SetupMode::AdminOnly));

        restarted
            .complete_admin(
                "rd_setup_restart_token",
                SetupAdminInput {
                    username: "Reader".to_owned(),
                    password: SecretString::from("correct horse battery staple".to_owned()),
                    email: None,
                },
            )
            .await
            .expect("administrator retry should complete");
        assert!(data.path().join("config.toml").exists());

        let restarted = SetupService::from_configured_database(
            data.path(),
            SecretString::from("unused-ready-token".to_owned()),
            None,
            database,
        )
        .await
        .expect("configured service should restart");
        assert!(restarted.is_ready());
        assert_eq!(restarted.setup_mode(), None);
    }

    #[tokio::test]
    async fn directory_sync_failure_cannot_commit_an_administrator_before_restart() {
        let data = tempdir().expect("temporary directory should be created");
        let database_url = format!(
            "sqlite://{}?mode=rwc",
            data.path().join("sync-failure.db").display()
        );
        let setup = SetupService::required(
            data.path(),
            SecretString::from("rd_setup_sync_failure_token".to_owned()),
            None,
        );
        setup
            .inner
            .fail_directory_sync
            .store(true, Ordering::Release);

        let error = setup
            .complete(
                "rd_setup_sync_failure_token",
                SetupCompleteInput {
                    database_url: SecretString::from(database_url.clone()),
                    username: "Reader".to_owned(),
                    password: SecretString::from("correct horse battery staple".to_owned()),
                    email: None,
                },
            )
            .await
            .expect_err("directory sync failure should abort before administrator commit");
        assert!(matches!(error, SetupError::WriteConfig(_)));
        assert_eq!(setup.setup_mode(), Some(SetupMode::Full));
        assert!(temporary_config_paths(data.path()).is_empty());
        assert!(matches!(
            setup
                .complete_admin(
                    "rd_setup_sync_failure_token",
                    SetupAdminInput {
                        username: "Reader".to_owned(),
                        password: SecretString::from("correct horse battery staple".to_owned()),
                        email: None,
                    },
                )
                .await,
            Err(SetupError::WrongMode)
        ));

        let database = connect_database(&database_url)
            .await
            .expect("database should reconnect");
        assert_eq!(
            user::Entity::find()
                .count(&database)
                .await
                .expect("users should count"),
            0
        );
        let restarted = SetupService::from_configured_database(
            data.path(),
            SecretString::from("rd_setup_sync_restart_token".to_owned()),
            None,
            database,
        )
        .await
        .expect("persisted config should restart in a recoverable mode");
        assert_eq!(restarted.setup_mode(), Some(SetupMode::AdminOnly));
    }

    #[tokio::test]
    async fn deleting_the_claimed_administrator_does_not_reopen_bootstrap() {
        let data = tempdir().expect("temporary directory should be created");
        let database_url = format!(
            "sqlite://{}?mode=rwc",
            data.path().join("deleted-admin.db").display()
        );
        let setup = SetupService::required(
            data.path(),
            SecretString::from("rd_setup_deleted_admin_token".to_owned()),
            None,
        );
        let admin = setup
            .complete(
                "rd_setup_deleted_admin_token",
                SetupCompleteInput {
                    database_url: SecretString::from(database_url.clone()),
                    username: "Reader".to_owned(),
                    password: SecretString::from("correct horse battery staple".to_owned()),
                    email: None,
                },
            )
            .await
            .expect("administrator should be created");
        let database = setup.database().expect("database should be attached");
        user::Entity::delete_by_id(admin.id)
            .exec(&database)
            .await
            .expect("administrator should be deleted");

        let error = match SetupService::from_configured_database(
            data.path(),
            SecretString::from("unused-token".to_owned()),
            None,
            database,
        )
        .await
        {
            Ok(_) => panic!("a retained claim without users must be inconsistent"),
            Err(error) => error,
        };
        assert!(matches!(error, SetupError::InconsistentBootstrap));
        assert_eq!(
            error.to_string(),
            "configured bootstrap state is inconsistent"
        );
    }

    #[test]
    fn durable_replace_reports_the_failed_platform_operation_without_paths() {
        let data = tempdir().expect("temporary directory should be created");
        let missing = data.path().join("secret-database-url.tmp");
        let error = durable_replace(&missing, &data.path().join("config.toml"), data.path())
            .expect_err("missing temporary file should fail");

        assert_eq!(error.operation, ConfigOperation::Sync);
        assert_eq!(error.source.kind(), io::ErrorKind::NotFound);
        assert!(!error.to_string().contains("secret-database-url"));
        assert!(
            !error
                .to_string()
                .contains(&data.path().display().to_string())
        );
    }

    #[test]
    fn cleanup_failure_error_is_redacted_and_records_both_operation_classes() {
        let data = tempdir().expect("temporary directory should be created");
        let temporary_path = data.path().join(".config.toml.database-url-sentinel.tmp");
        fs::create_dir(&temporary_path).expect("cleanup-blocking directory should be created");
        fs::write(temporary_path.join("child"), b"sentinel")
            .expect("cleanup-blocking child should be written");
        let mut temporary = TemporaryConfig::new(temporary_path.clone());

        let error = temporary.cleanup_after_failure(ConfigFailure::new(
            ConfigOperation::Write,
            io::Error::other("database-url-sentinel"),
        ));
        let message = match error {
            SetupError::WriteConfig(source) => source.to_string(),
            other => panic!("unexpected cleanup error: {other}"),
        };

        assert!(message.contains("write"));
        assert!(message.contains("cleanup"));
        assert!(!message.contains("database-url-sentinel"));
        assert!(!message.contains(&temporary_path.display().to_string()));
    }

    fn temporary_config_paths(data_dir: &Path) -> Vec<PathBuf> {
        fs::read_dir(data_dir)
            .expect("data directory should be readable")
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.starts_with(".config.toml.") && name.ends_with(".tmp"))
            })
            .collect()
    }
}
