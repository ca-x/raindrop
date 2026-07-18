use std::{fmt, time::Duration};

use sea_orm::{
    ConnectionTrait, DatabaseBackend, DbBackend, QueryResult, Statement, TransactionTrait, Value,
};
use time::OffsetDateTime;

use super::FeedRepository;

const MAX_RETENTION_LIMIT: u16 = 100;

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
