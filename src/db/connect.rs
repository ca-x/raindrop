use std::time::Duration;

use sea_orm::{
    ConnectOptions, ConnectionTrait, Database, DatabaseBackend, DatabaseConnection, Statement,
};
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

    let database = Database::connect(options).await?;
    if is_sqlite {
        configure_sqlite(&database, url).await?;
    }
    Ok(database)
}

async fn configure_sqlite(database: &DatabaseConnection, url: &str) -> Result<(), DbError> {
    for pragma in [
        "PRAGMA foreign_keys = ON",
        "PRAGMA busy_timeout = 5000",
        "PRAGMA synchronous = NORMAL",
    ] {
        database
            .execute(Statement::from_string(
                DatabaseBackend::Sqlite,
                pragma.to_owned(),
            ))
            .await?;
    }

    if !url.contains(":memory:") && !url.contains("mode=memory") {
        database
            .execute(Statement::from_string(
                DatabaseBackend::Sqlite,
                "PRAGMA journal_mode = WAL".to_owned(),
            ))
            .await?;
    }
    Ok(())
}
