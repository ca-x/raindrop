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
use sea_orm::{DatabaseConnection, EntityTrait, PaginatorTrait};
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
    db::{DatabaseConfig, DbError, connect, entities::user, migrate},
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
        let users = user::Entity::find()
            .count(&database)
            .await
            .map_err(DbError::from)
            .map_err(SetupError::Database)?;
        if users == 0 {
            Ok(Self::admin_only(data_dir, token, public_url, database))
        } else {
            Ok(Self::ready(data_dir, public_url, database))
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
        if user::Entity::find()
            .count(&database)
            .await
            .map_err(DbError::from)
            .map_err(SetupError::Database)?
            > 0
        {
            return Err(SetupError::AlreadyComplete);
        }

        let temporary_path = self.write_temporary_config(&input.database_url)?;

        if let Err(source) = fs::rename(&temporary_path, &self.inner.config_path) {
            let _ = fs::remove_file(&temporary_path);
            return Err(SetupError::WriteConfig(source));
        }

        if self.inner.fail_directory_sync.swap(false, Ordering::AcqRel) {
            return Err(SetupError::WriteConfig(io::Error::other(
                "injected directory synchronization failure",
            )));
        }
        if let Err(source) = sync_directory(&self.inner.data_dir) {
            return Err(SetupError::WriteConfig(source));
        }
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
                self.inner.mode.store(MODE_READY, Ordering::Release);
                Err(SetupError::AlreadyComplete)
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

    fn write_temporary_config(&self, database_url: &SecretString) -> Result<PathBuf, SetupError> {
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
        file.write_all(encoded.as_bytes())
            .and_then(|()| file.sync_all())
            .map_err(SetupError::WriteConfig)?;
        ensure_private_permissions(&temporary_path).map_err(SetupError::WriteConfig)?;
        Ok(temporary_path)
    }
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

fn sync_directory(path: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        File::open(path)?.sync_all()?;
    }
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

#[cfg(test)]
mod tests {
    use sea_orm::{EntityTrait, PaginatorTrait};
    use tempfile::tempdir;

    use super::*;

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
}
