use std::{fmt, time::Duration};

use sea_orm::{
    ConnectionTrait, DatabaseBackend, DbBackend, QueryResult, Statement, TransactionTrait, Value,
};
use time::OffsetDateTime;

use super::FeedRepository;

const MAX_RETENTION_LIMIT: u16 = 100;
pub(super) const HISTORY_BATCH_LIMIT: usize = 100;
const DEFAULT_ORPHAN_GRACE: Duration = Duration::from_secs(30 * 86_400);
const DEFAULT_SCAN_INTERVAL: Duration = Duration::from_secs(60 * 60);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FeedRetentionPolicy {
    pub orphan_grace: Option<Duration>,
    pub(super) scan_interval: Duration,
    pub(super) batch_limit: u16,
}

impl FeedRetentionPolicy {
    #[must_use]
    pub const fn new(orphan_grace: Option<Duration>) -> Self {
        Self {
            orphan_grace,
            scan_interval: DEFAULT_SCAN_INTERVAL,
            batch_limit: MAX_RETENTION_LIMIT,
        }
    }

    #[cfg(debug_assertions)]
    #[doc(hidden)]
    #[must_use]
    pub const fn with_scan_interval(mut self, scan_interval: Duration) -> Self {
        self.scan_interval = scan_interval;
        self
    }
}

impl Default for FeedRetentionPolicy {
    fn default() -> Self {
        Self::new(Some(DEFAULT_ORPHAN_GRACE))
    }
}

#[derive(thiserror::Error)]
pub enum FeedRetentionError {
    #[error("feed retention database operation failed")]
    Database(#[source] sea_orm::DbErr),
    #[error("feed retention request is invalid")]
    InvalidRequest,
    #[error("feed retention cutoff is outside the supported range")]
    InvalidTime,
    #[error("feed retention data is corrupt")]
    CorruptData,
}

impl fmt::Debug for FeedRetentionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Database(_) => "FeedRetentionError::Database([REDACTED])",
            Self::InvalidRequest => "FeedRetentionError::InvalidRequest",
            Self::InvalidTime => "FeedRetentionError::InvalidTime",
            Self::CorruptData => "FeedRetentionError::CorruptData",
        })
    }
}

impl From<sea_orm::DbErr> for FeedRetentionError {
    fn from(error: sea_orm::DbErr) -> Self {
        Self::Database(error)
    }
}

#[derive(Debug)]
struct RetentionCandidate {
    feed_id: String,
}

#[derive(Debug)]
struct LockedRetentionFeed {
    orphaned_at: Option<OffsetDateTime>,
}

impl FeedRepository {
    pub async fn purge_orphaned_feeds(
        &self,
        grace: Duration,
        limit: u16,
    ) -> Result<usize, FeedRetentionError> {
        if grace.is_zero() || !(1..=MAX_RETENTION_LIMIT).contains(&limit) {
            return Err(FeedRetentionError::InvalidRequest);
        }
        let backend = self.connection().get_database_backend();
        let now = database_now(self.connection(), backend).await?;
        let grace = time::Duration::try_from(grace).map_err(|_| FeedRetentionError::InvalidTime)?;
        let cutoff = now
            .checked_sub(grace)
            .ok_or(FeedRetentionError::InvalidTime)?;
        let candidates = find_candidates(self.connection(), backend, cutoff, limit).await?;
        let mut deleted = 0;

        for candidate in candidates {
            let transaction = self.connection().begin().await?;
            let result = async {
                let Some(feed) = lock_candidate(&transaction, backend, &candidate.feed_id).await?
                else {
                    return Ok(false);
                };
                if feed
                    .orphaned_at
                    .is_none_or(|orphaned_at| orphaned_at > cutoff)
                    || has_subscription(&transaction, backend, &candidate.feed_id).await?
                    || has_active_run(&transaction, backend, &candidate.feed_id).await?
                {
                    return Ok(false);
                }
                let result = transaction
                    .execute(delete_feed_statement(backend, &candidate.feed_id))
                    .await?;
                if result.rows_affected() != 1 {
                    return Err(FeedRetentionError::CorruptData);
                }
                Ok(true)
            }
            .await;
            match result {
                Ok(was_deleted) => {
                    transaction.commit().await?;
                    deleted += usize::from(was_deleted);
                }
                Err(error) => {
                    transaction.rollback().await?;
                    return Err(error);
                }
            }
        }

        Ok(deleted)
    }
}

async fn database_now<C>(
    connection: &C,
    backend: DbBackend,
) -> Result<OffsetDateTime, FeedRetentionError>
where
    C: ConnectionTrait,
{
    let sql = match backend {
        DatabaseBackend::Sqlite => "SELECT strftime('%Y-%m-%dT%H:%M:%f000Z','now') AS database_now",
        DatabaseBackend::Postgres => "SELECT clock_timestamp() AS database_now",
        DatabaseBackend::MySql => "SELECT UTC_TIMESTAMP(6) AS database_now",
    };
    let row = connection
        .query_one(Statement::from_string(backend, sql.to_owned()))
        .await?
        .ok_or(FeedRetentionError::CorruptData)?;
    required(&row, "database_now")
}

async fn find_candidates<C>(
    connection: &C,
    backend: DbBackend,
    cutoff: OffsetDateTime,
    limit: u16,
) -> Result<Vec<RetentionCandidate>, FeedRetentionError>
where
    C: ConnectionTrait,
{
    let sql = if backend == DatabaseBackend::Postgres {
        "SELECT id FROM feeds
         WHERE orphaned_at IS NOT NULL AND orphaned_at <= $1
         ORDER BY orphaned_at, id LIMIT $2"
    } else {
        "SELECT id FROM feeds
         WHERE orphaned_at IS NOT NULL AND orphaned_at <= ?
         ORDER BY orphaned_at, id LIMIT ?"
    };
    connection
        .query_all(Statement::from_sql_and_values(
            backend,
            sql,
            [Value::from(cutoff), i64::from(limit).into()],
        ))
        .await?
        .into_iter()
        .map(|row| {
            Ok(RetentionCandidate {
                feed_id: required(&row, "id")?,
            })
        })
        .collect()
}

async fn lock_candidate<C>(
    connection: &C,
    backend: DbBackend,
    feed_id: &str,
) -> Result<Option<LockedRetentionFeed>, FeedRetentionError>
where
    C: ConnectionTrait,
{
    if backend == DatabaseBackend::Sqlite {
        let result = connection
            .execute(Statement::from_sql_and_values(
                backend,
                "UPDATE feeds SET lease_token = lease_token WHERE id = ?",
                [feed_id.into()],
            ))
            .await?;
        if result.rows_affected() == 0 {
            return Ok(None);
        }
    }
    let sql = match backend {
        DatabaseBackend::Sqlite => "SELECT orphaned_at FROM feeds WHERE id = ?",
        DatabaseBackend::Postgres => "SELECT orphaned_at FROM feeds WHERE id = $1 FOR UPDATE",
        DatabaseBackend::MySql => "SELECT orphaned_at FROM feeds WHERE id = ? FOR UPDATE",
    };
    connection
        .query_one(Statement::from_sql_and_values(
            backend,
            sql,
            [feed_id.into()],
        ))
        .await?
        .map(|row| {
            Ok(LockedRetentionFeed {
                orphaned_at: optional(&row, "orphaned_at")?,
            })
        })
        .transpose()
}

async fn has_subscription<C>(
    connection: &C,
    backend: DbBackend,
    feed_id: &str,
) -> Result<bool, FeedRetentionError>
where
    C: ConnectionTrait,
{
    let sql = if backend == DatabaseBackend::Postgres {
        "SELECT 1 AS present FROM subscriptions WHERE feed_id = $1 LIMIT 1"
    } else {
        "SELECT 1 AS present FROM subscriptions WHERE feed_id = ? LIMIT 1"
    };
    Ok(connection
        .query_one(Statement::from_sql_and_values(
            backend,
            sql,
            [feed_id.into()],
        ))
        .await?
        .is_some())
}

async fn has_active_run<C>(
    connection: &C,
    backend: DbBackend,
    feed_id: &str,
) -> Result<bool, FeedRetentionError>
where
    C: ConnectionTrait,
{
    let sql = if backend == DatabaseBackend::Postgres {
        "SELECT 1 AS present FROM feed_refresh_runs
         WHERE feed_id = $1 AND status IN ('QUEUED','RUNNING') LIMIT 1"
    } else {
        "SELECT 1 AS present FROM feed_refresh_runs
         WHERE feed_id = ? AND status IN ('QUEUED','RUNNING') LIMIT 1"
    };
    Ok(connection
        .query_one(Statement::from_sql_and_values(
            backend,
            sql,
            [feed_id.into()],
        ))
        .await?
        .is_some())
}

fn delete_feed_statement(backend: DbBackend, feed_id: &str) -> Statement {
    let sql = if backend == DatabaseBackend::Postgres {
        "DELETE FROM feeds WHERE id = $1"
    } else {
        "DELETE FROM feeds WHERE id = ?"
    };
    Statement::from_sql_and_values(backend, sql, [feed_id.into()])
}

fn required<T>(row: &QueryResult, column: &str) -> Result<T, FeedRetentionError>
where
    T: sea_orm::TryGetable,
{
    row.try_get("", column)
        .map_err(|_| FeedRetentionError::CorruptData)
}

fn optional<T>(row: &QueryResult, column: &str) -> Result<Option<T>, FeedRetentionError>
where
    T: sea_orm::TryGetable,
{
    row.try_get("", column)
        .map_err(|_| FeedRetentionError::CorruptData)
}

/// Refresh diagnostics are operational history, not article data. The current lifecycle outbox
/// has no dispatcher; retain its untouched refresh notifications for the same seven-day window.
/// Never remove leased, retried, unknown, or externally consumed events.
impl FeedRepository {
    pub async fn purge_refresh_history(&self) -> Result<usize, FeedRetentionError> {
        const BATCH: usize = HISTORY_BATCH_LIMIT;
        let backend = self.connection().get_database_backend();
        let now = database_now(self.connection(), backend).await?;
        let cutoff = now - time::Duration::days(7);
        let transaction = self.connection().begin().await?;
        let parameter = if backend == DatabaseBackend::Postgres {
            "$1"
        } else {
            "?"
        };
        let rows = transaction
            .query_all(Statement::from_sql_and_values(
                backend,
                format!(
                    "SELECT id FROM feed_refresh_runs r
             WHERE status IN ('SUCCESS','NOT_MODIFIED','PARTIAL','ERROR','LEASE_LOST','CANCELLED')
               AND queued_at < {parameter} AND completed_at < {parameter2}
               AND NOT EXISTS (SELECT 1 FROM lifecycle_outbox o WHERE o.refresh_id = r.id
                 AND (o.event_type NOT IN ('feed.refresh.persisted','feed.refresh.completed')
                   OR o.status <> 'PENDING' OR o.attempts <> 0
                   OR o.lease_owner IS NOT NULL OR o.lease_until IS NOT NULL))
             ORDER BY queued_at, id LIMIT {BATCH}",
                    parameter2 = if backend == DatabaseBackend::Postgres {
                        "$2"
                    } else {
                        "?"
                    }
                ),
                [Value::from(cutoff), Value::from(cutoff)],
            ))
            .await?;
        let ids = rows
            .iter()
            .map(|row| required::<String>(row, "id"))
            .collect::<Result<Vec<_>, _>>()?;
        let mut deleted = 0;
        if !ids.is_empty() {
            let bindings = retention_bindings(backend, ids.len());
            let values: Vec<Value> = ids.into_iter().map(Value::from).collect();
            // Active runs are never candidates. Recheck every event's eligibility at deletion;
            // a concurrently claimed notification must still protect its refresh run.
            transaction
                .execute(refresh_event_delete_statement(backend, values.clone()))
                .await?;
            let result = transaction.execute(Statement::from_sql_and_values(backend, format!(
                "DELETE FROM feed_refresh_runs WHERE id IN ({bindings})
                 AND NOT EXISTS (SELECT 1 FROM lifecycle_outbox WHERE refresh_id = feed_refresh_runs.id)"
            ), values)).await?;
            deleted = usize::try_from(result.rows_affected())
                .map_err(|_| FeedRetentionError::CorruptData)?;
        }
        transaction.commit().await?;
        Ok(deleted)
    }
}

impl FeedRepository {
    /// Feed deletion cascades to refresh runs, but outbox records deliberately have no FK.
    /// Sweep the resulting untouched notifications too, without dropping dispatchable leases.
    pub async fn purge_orphaned_refresh_events(&self) -> Result<usize, FeedRetentionError> {
        let backend = self.connection().get_database_backend();
        let cutoff = database_now(self.connection(), backend).await? - time::Duration::days(7);
        let transaction = self.connection().begin().await?;
        let p1 = if backend == DatabaseBackend::Postgres {
            "$1"
        } else {
            "?"
        };
        let p2 = if backend == DatabaseBackend::Postgres {
            "$2"
        } else {
            "?"
        };
        let rows = transaction
            .query_all(Statement::from_sql_and_values(
                backend,
                format!(
                    "SELECT id FROM lifecycle_outbox o
             WHERE status = 'PENDING' AND available_at < {p1} AND created_at < {p2}
               AND event_type IN ('feed.refresh.persisted','feed.refresh.completed')
               AND attempts = 0 AND lease_owner IS NULL AND lease_until IS NULL
               AND NOT EXISTS (SELECT 1 FROM feed_refresh_runs r WHERE r.id = o.refresh_id)
             ORDER BY available_at, id LIMIT {HISTORY_BATCH_LIMIT}"
                ),
                [Value::from(cutoff), Value::from(cutoff)],
            ))
            .await?;
        let ids = rows
            .iter()
            .map(|row| required::<String>(row, "id"))
            .collect::<Result<Vec<_>, _>>()?;
        let mut deleted = 0;
        if !ids.is_empty() {
            let bindings = retention_bindings(backend, ids.len());
            let result = transaction.execute(Statement::from_sql_and_values(backend, format!(
                "DELETE FROM lifecycle_outbox WHERE id IN ({bindings})
                 AND event_type IN ('feed.refresh.persisted','feed.refresh.completed')
                 AND status = 'PENDING' AND attempts = 0 AND lease_owner IS NULL AND lease_until IS NULL
                 AND NOT EXISTS (SELECT 1 FROM feed_refresh_runs r WHERE r.id = lifecycle_outbox.refresh_id)"
            ), ids.into_iter().map(Value::from))).await?;
            deleted = usize::try_from(result.rows_affected())
                .map_err(|_| FeedRetentionError::CorruptData)?;
        }
        transaction.commit().await?;
        Ok(deleted)
    }
}

fn retention_bindings(backend: DbBackend, count: usize) -> String {
    (1..=count)
        .map(|index| match backend {
            DatabaseBackend::Postgres => format!("${index}"),
            DatabaseBackend::Sqlite | DatabaseBackend::MySql => "?".to_owned(),
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn refresh_event_delete_statement(backend: DbBackend, values: Vec<Value>) -> Statement {
    let bindings = retention_bindings(backend, values.len());
    // Without statistics SQLite favors the status index for IN batches and scans the pending
    // queue. Require the existing refresh index so work remains bounded by the selected runs.
    let table = if backend == DatabaseBackend::Sqlite {
        "lifecycle_outbox INDEXED BY uq_lifecycle_outbox_order"
    } else {
        "lifecycle_outbox"
    };
    Statement::from_sql_and_values(
        backend,
        format!(
            "DELETE FROM {table} WHERE refresh_id IN ({bindings})
             AND event_type IN ('feed.refresh.persisted','feed.refresh.completed')
             AND status = 'PENDING' AND attempts = 0 AND lease_owner IS NULL AND lease_until IS NULL"
        ),
        values,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{DatabaseConfig, connect, migrate};
    use secrecy::SecretString;

    #[tokio::test]
    async fn sqlite_refresh_event_batch_looks_up_refresh_ids_instead_of_pending_queue() {
        let database = connect(&DatabaseConfig::new(SecretString::from("sqlite::memory:")))
            .await
            .expect("test database connects");
        migrate(&database).await.expect("test schema migrates");
        let values = (0..HISTORY_BATCH_LIMIT)
            .map(|index| Value::from(index.to_string()))
            .collect();
        let mut statement = refresh_event_delete_statement(DatabaseBackend::Sqlite, values);
        statement.sql.insert_str(0, "EXPLAIN QUERY PLAN ");
        let details = database
            .query_all(statement)
            .await
            .expect("production delete plan is valid")
            .iter()
            .map(|row| row.try_get::<String>("", "detail").unwrap())
            .collect::<Vec<_>>();
        assert!(
            details
                .iter()
                .any(|detail| detail.contains("(refresh_id=?)")),
            "a bounded batch must look up refresh IDs: {details:?}"
        );
        assert!(
            details
                .iter()
                .all(|detail| !detail.contains("SCAN ") && !detail.contains("(status=?)")),
            "retention must not scan the unrelated pending queue: {details:?}"
        );
    }
}
