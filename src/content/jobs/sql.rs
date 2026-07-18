use sea_orm::{ConnectionTrait, DatabaseBackend, QueryResult, Statement};
use time::OffsetDateTime;

use super::model::{ContentRepositoryError, ContentRepositoryErrorKind};

pub(super) struct CandidateJob {
    pub(super) id: String,
    pub(super) user_id: String,
}

pub(super) struct HeartbeatWrite<'a> {
    pub(super) job_id: &'a str,
    pub(super) user_id: &'a str,
    pub(super) attempt: i32,
    pub(super) owner: &'a str,
    pub(super) lease_token: i64,
    pub(super) lease_until: OffsetDateTime,
}

pub(super) async fn database_now<C>(
    connection: &C,
    backend: DatabaseBackend,
) -> Result<OffsetDateTime, ContentRepositoryError>
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
        .await
        .map_err(database_error)?
        .ok_or_else(corrupt_data)?;
    row.try_get("", "database_now").map_err(|_| corrupt_data())
}

pub(super) async fn lock_active_user<C>(
    connection: &C,
    backend: DatabaseBackend,
    user_id: &str,
) -> Result<bool, ContentRepositoryError>
where
    C: ConnectionTrait,
{
    if backend == DatabaseBackend::Sqlite {
        let result = connection
            .execute(Statement::from_sql_and_values(
                backend,
                "UPDATE users SET id = id WHERE id = ?",
                [user_id.into()],
            ))
            .await
            .map_err(database_error)?;
        if result.rows_affected() == 0 {
            return Ok(false);
        }
    }
    let sql = match backend {
        DatabaseBackend::Sqlite => "SELECT id, is_disabled FROM users WHERE id = ?",
        DatabaseBackend::Postgres => "SELECT id, is_disabled FROM users WHERE id = $1 FOR UPDATE",
        DatabaseBackend::MySql => "SELECT id, is_disabled FROM users WHERE id = ? FOR UPDATE",
    };
    connection
        .query_one(Statement::from_sql_and_values(
            backend,
            sql,
            [user_id.into()],
        ))
        .await
        .map_err(database_error)?
        .map(active_user)
        .transpose()
        .map(|user| user.unwrap_or(false))
}

pub(super) async fn visible_entry_content_hash<C>(
    connection: &C,
    backend: DatabaseBackend,
    user_id: &str,
    entry_id: &str,
) -> Result<Option<String>, ContentRepositoryError>
where
    C: ConnectionTrait,
{
    let sql = match backend {
        DatabaseBackend::Sqlite | DatabaseBackend::MySql => {
            "SELECT e.content_hash AS content_hash
             FROM entries e
             WHERE e.id = ?
               AND EXISTS (
                    SELECT 1 FROM subscriptions s
                    WHERE s.user_id = ? AND s.feed_id = e.feed_id
               )"
        }
        DatabaseBackend::Postgres => {
            "SELECT e.content_hash AS content_hash
             FROM entries e
             WHERE e.id = $1
               AND EXISTS (
                    SELECT 1 FROM subscriptions s
                    WHERE s.user_id = $2 AND s.feed_id = e.feed_id
               )"
        }
    };
    connection
        .query_one(Statement::from_sql_and_values(
            backend,
            sql,
            [entry_id.into(), user_id.into()],
        ))
        .await
        .map_err(database_error)?
        .map(|row| row.try_get("", "content_hash").map_err(|_| corrupt_data()))
        .transpose()
}

pub(super) async fn due_candidates<C>(
    connection: &C,
    backend: DatabaseBackend,
) -> Result<Vec<CandidateJob>, ContentRepositoryError>
where
    C: ConnectionTrait,
{
    let sql = match backend {
        DatabaseBackend::Sqlite => {
            "SELECT j.id AS id, j.user_id AS user_id
             FROM content_jobs j
             WHERE (
                    j.status IN ('QUEUED','RETRY_WAIT')
                    AND julianday(j.next_attempt_at) <= julianday('now')
                    AND (
                         SELECT COUNT(*) FROM content_jobs r
                         WHERE r.user_id = j.user_id
                           AND r.status = 'RUNNING'
                           AND r.lease_until IS NOT NULL
                           AND r.attempt_deadline_at IS NOT NULL
                           AND julianday(r.lease_until) > julianday('now')
                           AND julianday(r.attempt_deadline_at) > julianday('now')
                    ) < 2
                 ) OR (
                    j.status = 'RUNNING'
                    AND (
                         j.lease_until IS NULL
                         OR j.attempt_deadline_at IS NULL
                         OR julianday(j.lease_until) <= julianday('now')
                         OR julianday(j.attempt_deadline_at) <= julianday('now')
                    )
                    AND (
                         SELECT COUNT(*) FROM content_jobs r
                         WHERE r.user_id = j.user_id
                           AND r.status = 'RUNNING'
                           AND r.lease_until IS NOT NULL
                           AND r.attempt_deadline_at IS NOT NULL
                           AND julianday(r.lease_until) > julianday('now')
                           AND julianday(r.attempt_deadline_at) > julianday('now')
                    ) < 2
                 )
             ORDER BY j.next_attempt_at ASC, j.created_at ASC, j.id ASC
             LIMIT 16"
        }
        DatabaseBackend::Postgres => {
            "SELECT j.id AS id, j.user_id AS user_id
             FROM content_jobs j
             WHERE (
                    j.status IN ('QUEUED','RETRY_WAIT')
                    AND j.next_attempt_at <= clock_timestamp()
                    AND (
                         SELECT COUNT(*) FROM content_jobs r
                         WHERE r.user_id = j.user_id
                           AND r.status = 'RUNNING'
                           AND r.lease_until > clock_timestamp()
                           AND r.attempt_deadline_at > clock_timestamp()
                    ) < 2
                 ) OR (
                    j.status = 'RUNNING'
                    AND (
                         j.lease_until IS NULL
                         OR j.attempt_deadline_at IS NULL
                         OR j.lease_until <= clock_timestamp()
                         OR j.attempt_deadline_at <= clock_timestamp()
                    )
                    AND (
                         SELECT COUNT(*) FROM content_jobs r
                         WHERE r.user_id = j.user_id
                           AND r.status = 'RUNNING'
                           AND r.lease_until > clock_timestamp()
                           AND r.attempt_deadline_at > clock_timestamp()
                    ) < 2
                 )
             ORDER BY j.next_attempt_at ASC, j.created_at ASC, j.id ASC
             LIMIT 16"
        }
        DatabaseBackend::MySql => {
            "SELECT j.id AS id, j.user_id AS user_id
             FROM content_jobs j
             WHERE (
                    j.status IN ('QUEUED','RETRY_WAIT')
                    AND j.next_attempt_at <= UTC_TIMESTAMP(6)
                    AND (
                         SELECT COUNT(*) FROM content_jobs r
                         WHERE r.user_id = j.user_id
                           AND r.status = 'RUNNING'
                           AND r.lease_until > UTC_TIMESTAMP(6)
                           AND r.attempt_deadline_at > UTC_TIMESTAMP(6)
                    ) < 2
                 ) OR (
                    j.status = 'RUNNING'
                    AND (
                         j.lease_until IS NULL
                         OR j.attempt_deadline_at IS NULL
                         OR j.lease_until <= UTC_TIMESTAMP(6)
                         OR j.attempt_deadline_at <= UTC_TIMESTAMP(6)
                    )
                    AND (
                         SELECT COUNT(*) FROM content_jobs r
                         WHERE r.user_id = j.user_id
                           AND r.status = 'RUNNING'
                           AND r.lease_until > UTC_TIMESTAMP(6)
                           AND r.attempt_deadline_at > UTC_TIMESTAMP(6)
                    ) < 2
                 )
             ORDER BY j.next_attempt_at ASC, j.created_at ASC, j.id ASC
             LIMIT 16"
        }
    };
    connection
        .query_all(Statement::from_string(backend, sql.to_owned()))
        .await
        .map_err(database_error)?
        .into_iter()
        .map(candidate_job)
        .collect()
}

pub(super) async fn lock_job<C>(
    connection: &C,
    backend: DatabaseBackend,
    job_id: &str,
) -> Result<bool, ContentRepositoryError>
where
    C: ConnectionTrait,
{
    if backend == DatabaseBackend::Sqlite {
        return connection
            .execute(Statement::from_sql_and_values(
                backend,
                "UPDATE content_jobs SET lease_token = lease_token WHERE id = ?",
                [job_id.into()],
            ))
            .await
            .map(|result| result.rows_affected() == 1)
            .map_err(database_error);
    }
    let sql = if backend == DatabaseBackend::Postgres {
        "SELECT id FROM content_jobs WHERE id = $1 FOR UPDATE"
    } else {
        "SELECT id FROM content_jobs WHERE id = ? FOR UPDATE"
    };
    connection
        .query_one(Statement::from_sql_and_values(
            backend,
            sql,
            [job_id.into()],
        ))
        .await
        .map(|row| row.is_some())
        .map_err(database_error)
}

pub(super) async fn active_running_count<C>(
    connection: &C,
    backend: DatabaseBackend,
    user_id: &str,
) -> Result<i64, ContentRepositoryError>
where
    C: ConnectionTrait,
{
    let sql = match backend {
        DatabaseBackend::Sqlite => {
            "SELECT COUNT(*) AS count FROM content_jobs
             WHERE user_id = ? AND status = 'RUNNING'
               AND lease_until IS NOT NULL AND attempt_deadline_at IS NOT NULL
               AND julianday(lease_until) > julianday('now')
               AND julianday(attempt_deadline_at) > julianday('now')"
        }
        DatabaseBackend::Postgres => {
            "SELECT COUNT(*) AS count FROM content_jobs
             WHERE user_id = $1 AND status = 'RUNNING'
               AND lease_until > clock_timestamp()
               AND attempt_deadline_at > clock_timestamp()"
        }
        DatabaseBackend::MySql => {
            "SELECT COUNT(*) AS count FROM content_jobs
             WHERE user_id = ? AND status = 'RUNNING'
               AND lease_until > UTC_TIMESTAMP(6)
               AND attempt_deadline_at > UTC_TIMESTAMP(6)"
        }
    };
    let row = connection
        .query_one(Statement::from_sql_and_values(
            backend,
            sql,
            [user_id.into()],
        ))
        .await
        .map_err(database_error)?
        .ok_or_else(corrupt_data)?;
    let count: i64 = row.try_get("", "count").map_err(|_| corrupt_data())?;
    if count < 0 {
        Err(corrupt_data())
    } else {
        Ok(count)
    }
}

pub(super) async fn heartbeat<C>(
    connection: &C,
    backend: DatabaseBackend,
    write: &HeartbeatWrite<'_>,
) -> Result<bool, ContentRepositoryError>
where
    C: ConnectionTrait,
{
    let sql = match backend {
        DatabaseBackend::Sqlite => {
            "UPDATE content_jobs SET lease_until = ?
             WHERE id = ? AND user_id = ? AND status = 'RUNNING'
               AND attempts = ? AND lease_owner = ? AND lease_token = ?
               AND lease_until IS NOT NULL AND attempt_deadline_at IS NOT NULL
               AND julianday(lease_until) > julianday('now')
               AND julianday(attempt_deadline_at) > julianday('now')"
        }
        DatabaseBackend::Postgres => {
            "UPDATE content_jobs SET lease_until = $1
             WHERE id = $2 AND user_id = $3 AND status = 'RUNNING'
               AND attempts = $4 AND lease_owner = $5 AND lease_token = $6
               AND lease_until > clock_timestamp()
               AND attempt_deadline_at > clock_timestamp()"
        }
        DatabaseBackend::MySql => {
            "UPDATE content_jobs SET lease_until = ?
             WHERE id = ? AND user_id = ? AND status = 'RUNNING'
               AND attempts = ? AND lease_owner = ? AND lease_token = ?
               AND lease_until > UTC_TIMESTAMP(6)
               AND attempt_deadline_at > UTC_TIMESTAMP(6)"
        }
    };
    connection
        .execute(Statement::from_sql_and_values(
            backend,
            sql,
            [
                write.lease_until.into(),
                write.job_id.into(),
                write.user_id.into(),
                write.attempt.into(),
                write.owner.into(),
                write.lease_token.into(),
            ],
        ))
        .await
        .map(|result| result.rows_affected() == 1)
        .map_err(database_error)
}

fn active_user(row: QueryResult) -> Result<bool, ContentRepositoryError> {
    let disabled: bool = row.try_get("", "is_disabled").map_err(|_| corrupt_data())?;
    Ok(!disabled)
}

fn candidate_job(row: QueryResult) -> Result<CandidateJob, ContentRepositoryError> {
    Ok(CandidateJob {
        id: row.try_get("", "id").map_err(|_| corrupt_data())?,
        user_id: row.try_get("", "user_id").map_err(|_| corrupt_data())?,
    })
}

pub(super) fn database_error(_error: sea_orm::DbErr) -> ContentRepositoryError {
    ContentRepositoryError::new(ContentRepositoryErrorKind::Database)
}

pub(super) const fn corrupt_data() -> ContentRepositoryError {
    ContentRepositoryError::new(ContentRepositoryErrorKind::CorruptData)
}
