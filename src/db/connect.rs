use std::time::Duration;

use sea_orm::sqlx::sqlite::{SqliteJournalMode, SqliteSynchronous};
use sea_orm::{ConnectOptions, Database, DatabaseConnection};
use secrecy::{ExposeSecret, SecretString};

#[derive(Debug)]
pub struct DatabaseConfig {
    url: SecretString,
}

impl DatabaseConfig {
    #[must_use]
    pub fn new(url: SecretString) -> Self {
        Self { url }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum DbError {
    #[error("unsupported database URL scheme")]
    UnsupportedScheme,
    #[error("database operation failed")]
    Operation(#[source] sea_orm::DbErr),
}

impl From<sea_orm::DbErr> for DbError {
    fn from(value: sea_orm::DbErr) -> Self {
        Self::Operation(value)
    }
}

pub async fn connect(config: &DatabaseConfig) -> Result<DatabaseConnection, DbError> {
    let url = config.url.expose_secret();
    let is_sqlite = url.starts_with("sqlite:");
    let is_file_sqlite = is_sqlite && !url.contains(":memory:") && !url.contains("mode=memory");
    if !is_sqlite
        && !url.starts_with("postgres:")
        && !url.starts_with("postgresql:")
        && !url.starts_with("mysql:")
    {
        return Err(DbError::UnsupportedScheme);
    }

    let mut options = ConnectOptions::new(url.to_owned());
    options
        .min_connections(1)
        .max_connections(if is_sqlite { 1 } else { 8 })
        .connect_timeout(Duration::from_secs(5))
        .acquire_timeout(Duration::from_secs(5))
        .idle_timeout(Duration::from_secs(300))
        .sqlx_logging(false);

    if is_sqlite {
        options.map_sqlx_sqlite_opts(move |options| {
            let options = options
                .foreign_keys(true)
                .busy_timeout(Duration::from_secs(5))
                .synchronous(SqliteSynchronous::Normal);
            if is_file_sqlite {
                options.journal_mode(SqliteJournalMode::Wal)
            } else {
                options
            }
        });
    } else if url.starts_with("postgres:") || url.starts_with("postgresql:") {
        options.map_sqlx_postgres_opts(|options| options.options([("timezone", "UTC")]));
    } else {
        options.map_sqlx_mysql_opts(|options| options.timezone(Some("+00:00".to_owned())));
    }

    Database::connect(options).await.map_err(DbError::from)
}
