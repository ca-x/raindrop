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
    let is_file_sqlite = is_sqlite && !sqlite_is_connection_local(url);
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

/// Opens the pool used by latency-sensitive reads.
///
/// File-backed SQLite keeps writes on the single-connection primary pool and serves reads from a
/// separate read-only pool. A queued writer can therefore never consume the connections needed by
/// authentication and Reader requests. Other databases already have concurrent primary pools, and
/// in-memory SQLite must keep using its original connection to see the same database.
pub async fn connect_reader(
    config: &DatabaseConfig,
    primary: &DatabaseConnection,
) -> Result<DatabaseConnection, DbError> {
    let url = config.url.expose_secret();
    if !url.starts_with("sqlite:") || sqlite_is_connection_local(url) {
        return Ok(primary.clone());
    }

    let mut options = ConnectOptions::new(url.to_owned());
    options
        .min_connections(1)
        .max_connections(4)
        .connect_timeout(Duration::from_secs(5))
        .acquire_timeout(Duration::from_secs(5))
        .idle_timeout(Duration::from_secs(300))
        .sqlx_logging(false)
        .map_sqlx_sqlite_opts(|options| {
            options
                .read_only(true)
                .create_if_missing(false)
                .busy_timeout(Duration::from_secs(5))
        });

    Database::connect(options).await.map_err(DbError::from)
}

fn sqlite_is_connection_local(url: &str) -> bool {
    let without_scheme = url
        .strip_prefix("sqlite://")
        .or_else(|| url.strip_prefix("sqlite:"))
        .unwrap_or(url);
    let mut database_and_params = without_scheme.splitn(2, '?');
    let database = database_and_params.next().unwrap_or_default();
    database.is_empty()
        || percent_decoded_eq(database, b":memory:")
        || percent_decoded_eq(database, b"file::memory:")
        || database_and_params.next().is_some_and(|params| {
            url::form_urlencoded::parse(params.as_bytes())
                .any(|(key, value)| key == "mode" && value == "memory")
        })
}

fn percent_decoded_eq(value: &str, expected: &[u8]) -> bool {
    let value = value.as_bytes();
    let mut value_index = 0;
    for expected_byte in expected {
        let Some(current) = value.get(value_index) else {
            return false;
        };
        let decoded = if *current == b'%' {
            let Some((high, low)) = value.get(value_index + 1).zip(value.get(value_index + 2))
            else {
                return false;
            };
            let Some(decoded) = decode_hex(*high).zip(decode_hex(*low)) else {
                return false;
            };
            value_index += 3;
            (decoded.0 << 4) | decoded.1
        } else {
            value_index += 1;
            *current
        };
        if decoded != *expected_byte {
            return false;
        }
    }
    value_index == value.len()
}

fn decode_hex(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        b'A'..=b'F' => Some(value - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::sqlite_is_connection_local;

    #[test]
    fn sqlite_memory_detection_matches_connection_url_fields() {
        for url in [
            "sqlite::memory:",
            "sqlite://:memory:",
            "sqlite://:%6demory:",
            "sqlite:file::memory:",
            "sqlite://file::memory:",
            "sqlite://%66ile%3A%3Amemory%3A",
            "sqlite://%3Amemory%3A",
            "sqlite:",
            "sqlite://",
            "sqlite://?mode=rwc",
            "sqlite://?mode=memory",
            "sqlite://named.db?mode=memory&cache=shared",
        ] {
            assert!(
                sqlite_is_connection_local(url),
                "expected connection-local database: {url}"
            );
        }
        for url in [
            "sqlite://data/:memory:/raindrop.db?mode=rwc",
            "sqlite://data/mode=memory.db?mode=rwc",
            "sqlite://raindrop.db?cache=shared",
        ] {
            assert!(
                !sqlite_is_connection_local(url),
                "expected file database: {url}"
            );
        }
    }
}
