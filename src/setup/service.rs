use std::{
    fs::{self, File, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
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
    ready: AtomicBool,
    completion: Mutex<()>,
    sessions: SessionService,
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
                ready: AtomicBool::new(false),
                completion: Mutex::new(()),
                sessions: SessionService::unavailable(),
            }),
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
                ready: AtomicBool::new(true),
                completion: Mutex::new(()),
                sessions: SessionService::new(database),
            }),
        }
    }

    #[must_use]
    pub fn is_ready(&self) -> bool {
        self.inner.ready.load(Ordering::Acquire)
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
        if self.is_ready() {
            return Err(SetupError::AlreadyComplete);
        }
        self.require_token(token)?;
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
        if self.is_ready() {
            return Err(SetupError::AlreadyComplete);
        }
        self.require_token(token)?;
        validate_database_url(input.database_url.expose_secret())?;

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
        let admin = create_admin(
            &database,
            &PasswordService::default(),
            CreateAdminInput {
                username: input.username,
                password: input.password,
                email: input.email,
            },
        )
        .await
        .map_err(|error| {
            let _ = fs::remove_file(&temporary_path);
            SetupError::CreateAdmin(error)
        })?;

        if let Err(source) = fs::rename(&temporary_path, &self.inner.config_path) {
            let _ = user::Entity::delete_by_id(&admin.id).exec(&database).await;
            let _ = fs::remove_file(&temporary_path);
            return Err(SetupError::WriteConfig(source));
        }
        if let Err(source) = sync_directory(&self.inner.data_dir) {
            let _ = fs::remove_file(&self.inner.config_path);
            let _ = user::Entity::delete_by_id(&admin.id).exec(&database).await;
            return Err(SetupError::WriteConfig(source));
        }

        self.inner.sessions.attach_database(database);
        self.inner.ready.store(true, Ordering::Release);
        Ok(admin)
    }

    fn write_temporary_config(&self, database_url: &SecretString) -> Result<PathBuf, SetupError> {
        fs::create_dir_all(&self.inner.data_dir).map_err(SetupError::WriteConfig)?;
        let mut config = if self.inner.config_path.exists() {
            let content =
                fs::read_to_string(&self.inner.config_path).map_err(SetupError::WriteConfig)?;
            toml::from_str::<toml::Table>(&content).map_err(SetupError::ParseConfig)?
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

#[derive(Debug, thiserror::Error)]
pub enum SetupError {
    #[error("setup token is invalid")]
    Unauthorized,
    #[error("setup is already complete")]
    AlreadyComplete,
    #[error("setup is not complete")]
    NotReady,
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
