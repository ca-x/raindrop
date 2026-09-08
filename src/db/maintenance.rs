use sea_orm::{
    DatabaseBackend, DatabaseConnection,
    sqlx::{self, Row, SqliteConnection},
};
use serde::Serialize;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DatabaseStorage {
    pub database_bytes: u64,
    pub wal_bytes: u64,
    pub free_bytes: u64,
    pub entry_count: i64,
    pub feed_count: i64,
    pub tables: Option<Vec<TableStorage>>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TableStorage {
    pub name: String,
    pub bytes: i64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CompactionResult {
    pub before: DatabaseStorage,
    pub after: DatabaseStorage,
    pub reclaimed_bytes: u64,
    pub wal_truncated: bool,
    pub removed_refresh_runs: usize,
}

#[derive(Debug, thiserror::Error)]
pub enum MaintenanceError {
    #[error("database compaction requires file-backed SQLite")]
    Unsupported,
    #[error("database maintenance failed")]
    Database(#[from] sqlx::Error),
    #[error("refresh history cleanup failed")]
    Retention(#[from] crate::feeds::FeedRetentionError),
}

pub async fn storage(database: &DatabaseConnection) -> Result<DatabaseStorage, MaintenanceError> {
    ensure_sqlite(database)?;
    let mut connection = database.get_sqlite_connection_pool().acquire().await?;
    storage_on(&mut connection).await
}

pub async fn compact(database: &DatabaseConnection) -> Result<CompactionResult, MaintenanceError> {
    ensure_sqlite(database)?;
    let before = storage(database).await?;
    let repository = crate::feeds::FeedRepository::new(database.clone());
    let mut removed_refresh_runs = 0;
    loop {
        let removed = repository.purge_refresh_history().await?;
        removed_refresh_runs += removed;
        if removed == 0 {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    while repository.purge_orphaned_refresh_events().await? > 0 {
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    // Hold the primary pool's sole connection for the entire operation. No transaction: VACUUM
    // must run in autocommit mode. Reader requests use the separate read-only pool.
    let mut connection = database.get_sqlite_connection_pool().acquire().await?;
    sqlx::query("PRAGMA wal_checkpoint(PASSIVE)")
        .execute(&mut *connection)
        .await?;
    sqlx::query("VACUUM").execute(&mut *connection).await?;
    // A live reader can prevent truncation. Report that separately from successful compaction.
    // Optional WAL reclamation must not spend the entire five-second writer acquire timeout
    // waiting for a reader. Restore the connection setting even if the checkpoint fails.
    sqlx::query("PRAGMA busy_timeout = 0")
        .execute(&mut *connection)
        .await?;
    let checkpoint = sqlx::query("PRAGMA wal_checkpoint(TRUNCATE)")
        .fetch_one(&mut *connection)
        .await;
    sqlx::query("PRAGMA busy_timeout = 5000")
        .execute(&mut *connection)
        .await?;
    let wal_truncated = checkpoint?.try_get::<i64, _>(0)? == 0;
    let after = storage_on(&mut connection).await?;
    let reclaimed_bytes = (before.database_bytes + before.wal_bytes)
        .saturating_sub(after.database_bytes + after.wal_bytes);
    Ok(CompactionResult {
        before,
        after,
        reclaimed_bytes,
        wal_truncated,
        removed_refresh_runs,
    })
}

fn ensure_sqlite(database: &DatabaseConnection) -> Result<(), MaintenanceError> {
    use sea_orm::ConnectionTrait;
    if database.get_database_backend() != DatabaseBackend::Sqlite {
        return Err(MaintenanceError::Unsupported);
    }
    Ok(())
}

async fn storage_on(
    connection: &mut SqliteConnection,
) -> Result<DatabaseStorage, MaintenanceError> {
    let databases = sqlx::query("PRAGMA database_list")
        .fetch_all(&mut *connection)
        .await?;
    let path = databases
        .iter()
        .find(|row| row.get::<String, _>("name") == "main")
        .map(|row| row.get::<String, _>("file"))
        .filter(|path| !path.is_empty())
        .ok_or(MaintenanceError::Unsupported)?;
    let page_size: i64 = sqlx::query_scalar("PRAGMA page_size")
        .fetch_one(&mut *connection)
        .await?;
    let free_pages: i64 = sqlx::query_scalar("PRAGMA freelist_count")
        .fetch_one(&mut *connection)
        .await?;
    let entry_count = sqlx::query_scalar("SELECT COUNT(*) FROM entries")
        .fetch_one(&mut *connection)
        .await?;
    let feed_count = sqlx::query_scalar("SELECT COUNT(*) FROM feeds")
        .fetch_one(&mut *connection)
        .await?;
    // dbstat is optional in system SQLite builds. The basic statistics work without it.
    let tables = match sqlx::query(
        "SELECT COALESCE(s.tbl_name, d.name) AS name, SUM(d.pgsize) AS bytes
         FROM dbstat d LEFT JOIN sqlite_schema s ON s.name = d.name
         GROUP BY COALESCE(s.tbl_name, d.name) ORDER BY bytes DESC",
    )
    .fetch_all(&mut *connection)
    .await
    {
        Ok(rows) => Some(
            rows.into_iter()
                .map(|row| TableStorage {
                    name: row.get("name"),
                    bytes: row.get("bytes"),
                })
                .collect(),
        ),
        Err(sqlx::Error::Database(error)) if error.message().contains("no such table: dbstat") => {
            None
        }
        Err(error) => return Err(error.into()),
    };
    let wal_bytes = match tokio::fs::metadata(format!("{path}-wal")).await {
        Ok(metadata) => metadata.len(),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => 0,
        Err(error) => return Err(sqlx::Error::Io(error).into()),
    };
    Ok(DatabaseStorage {
        database_bytes: tokio::fs::metadata(&path)
            .await
            .map_err(sqlx::Error::Io)?
            .len(),
        wal_bytes,
        free_bytes: (page_size * free_pages) as u64,
        entry_count,
        feed_count,
        tables,
    })
}
