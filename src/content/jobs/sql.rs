use sea_orm::{ConnectionTrait, DatabaseBackend, QueryResult, Statement};
use time::OffsetDateTime;

use super::model::{ContentRepositoryError, ContentRepositoryErrorKind};

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

fn active_user(row: QueryResult) -> Result<bool, ContentRepositoryError> {
    let disabled: bool = row.try_get("", "is_disabled").map_err(|_| corrupt_data())?;
    Ok(!disabled)
}

pub(super) fn database_error(_error: sea_orm::DbErr) -> ContentRepositoryError {
    ContentRepositoryError::new(ContentRepositoryErrorKind::Database)
}

pub(super) const fn corrupt_data() -> ContentRepositoryError {
    ContentRepositoryError::new(ContentRepositoryErrorKind::CorruptData)
}
