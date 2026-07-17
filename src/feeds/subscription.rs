use std::fmt;

use sea_orm::{
    ConnectionTrait, DatabaseBackend, DbBackend, DbErr, QueryResult, Statement, TransactionTrait,
};
use time::OffsetDateTime;
use uuid::Uuid;

use super::lifecycle::is_unique_violation;
use super::{
    FeedRepository, NormalizedFeedUrl, RefreshDto, RefreshStatus, RefreshTrigger, SubscriptionDto,
};

#[derive(Clone)]
pub(super) struct SubscribeRecord {
    pub subscription_id: String,
    pub feed_id: String,
    pub run_id: String,
    pub run_status: RefreshStatus,
}

pub(super) struct RefreshContext {
    pub fetch_url: String,
    pub consecutive_failures: i64,
}

#[derive(thiserror::Error)]
pub(super) enum SubscriptionRepositoryError {
    #[error("subscription repository database operation failed")]
    Database(#[source] DbErr),
    #[error("subscription request is invalid")]
    InvalidRequest,
    #[error("subscription user is not authorized")]
    UserNotFound,
    #[error("normalized feed URL hash collision detected")]
    FeedUrlHashCollision,
    #[error("subscription repository data is corrupt")]
    CorruptData,
    #[error("stable subscribe refresh has conflicting semantics")]
    RunConflict,
}

impl fmt::Debug for SubscriptionRepositoryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Database(_) => "SubscriptionRepositoryError::Database([REDACTED])",
            Self::InvalidRequest => "SubscriptionRepositoryError::InvalidRequest",
            Self::UserNotFound => "SubscriptionRepositoryError::UserNotFound",
            Self::FeedUrlHashCollision => "SubscriptionRepositoryError::FeedUrlHashCollision",
            Self::CorruptData => "SubscriptionRepositoryError::CorruptData",
            Self::RunConflict => "SubscriptionRepositoryError::RunConflict",
        })
    }
}

impl From<DbErr> for SubscriptionRepositoryError {
    fn from(error: DbErr) -> Self {
        Self::Database(error)
    }
}

impl FeedRepository {
    pub(super) async fn database_now(&self) -> Result<OffsetDateTime, SubscriptionRepositoryError> {
        let backend = self.connection().get_database_backend();
        let row = self
            .connection()
            .query_one(Statement::from_string(
                backend,
                match backend {
                    DatabaseBackend::Sqlite => {
                        "SELECT strftime('%Y-%m-%dT%H:%M:%f000Z','now') AS database_now"
                    }
                    DatabaseBackend::Postgres => "SELECT clock_timestamp() AS database_now",
                    DatabaseBackend::MySql => "SELECT UTC_TIMESTAMP(6) AS database_now",
                }
                .to_owned(),
            ))
            .await?
            .ok_or(SubscriptionRepositoryError::CorruptData)?;
        required(&row, "database_now")
    }

    pub(super) async fn subscribe_transaction(
        &self,
        user_id: &str,
        source_url: &str,
        normalized: &NormalizedFeedUrl,
    ) -> Result<SubscribeRecord, SubscriptionRepositoryError> {
        for attempt in 0..3 {
            match self
                .subscribe_transaction_once(user_id, source_url, normalized)
                .await
            {
                Err(SubscriptionRepositoryError::Database(error))
                    if attempt < 2 && is_unique_violation(&error) => {}
                result => return result,
            }
        }
        Err(SubscriptionRepositoryError::CorruptData)
    }

    async fn subscribe_transaction_once(
        &self,
        user_id: &str,
        source_url: &str,
        normalized: &NormalizedFeedUrl,
    ) -> Result<SubscribeRecord, SubscriptionRepositoryError> {
        if source_url.is_empty() || source_url.len() > 4_096 {
            return Err(SubscriptionRepositoryError::InvalidRequest);
        }
        let backend = self.connection().get_database_backend();
        let transaction = self.connection().begin().await?;
        let result =
            async {
                ensure_active_user(&transaction, backend, user_id).await?;
                let feed_id =
                    match find_feed_by_hash(&transaction, backend, normalized.url_hash()).await? {
                        Some((feed_id, stored_url)) => {
                            if stored_url != normalized.complete() {
                                return Err(SubscriptionRepositoryError::FeedUrlHashCollision);
                            }
                            feed_id
                        }
                        None => {
                            let feed_id = Uuid::new_v4().to_string();
                            transaction
                                .execute(insert_feed_statement(
                                    backend, &feed_id, source_url, normalized,
                                ))
                                .await?;
                            feed_id
                        }
                    };

                let cleared = transaction
                    .execute(clear_orphaned_statement(backend, &feed_id))
                    .await?;
                if cleared.rows_affected() != 1 {
                    return Err(SubscriptionRepositoryError::CorruptData);
                }
                let (stored_url, entry_sequence_head) =
                    lock_feed_head(&transaction, backend, &feed_id).await?;
                if stored_url != normalized.complete() {
                    return Err(SubscriptionRepositoryError::FeedUrlHashCollision);
                }

                let subscription_id =
                    match find_subscription(&transaction, backend, user_id, &feed_id).await? {
                        Some(subscription_id) => subscription_id,
                        None => {
                            let subscription_id = Uuid::new_v4().to_string();
                            transaction
                                .execute(insert_subscription_statement(
                                    backend,
                                    &subscription_id,
                                    user_id,
                                    &feed_id,
                                    entry_sequence_head,
                                ))
                                .await?;
                            subscription_id
                        }
                    };

                let idempotency_key = format!("subscribe:{subscription_id}");
                let (run_id, run_status) =
                    match find_subscribe_run(&transaction, backend, &feed_id, &idempotency_key)
                        .await?
                    {
                        Some((run_id, requested_by, trigger, status)) => {
                            if requested_by.as_deref() != Some(user_id)
                                || trigger != RefreshTrigger::Subscribe
                            {
                                return Err(SubscriptionRepositoryError::RunConflict);
                            }
                            (run_id, status)
                        }
                        None => {
                            let run_id = Uuid::new_v4().to_string();
                            transaction
                                .execute(insert_subscribe_run_statement(
                                    backend,
                                    &run_id,
                                    &feed_id,
                                    user_id,
                                    &idempotency_key,
                                ))
                                .await?;
                            (run_id, RefreshStatus::Queued)
                        }
                    };

                Ok(SubscribeRecord {
                    subscription_id,
                    feed_id,
                    run_id,
                    run_status,
                })
            }
            .await;

        match result {
            Ok(record) => {
                transaction.commit().await?;
                Ok(record)
            }
            Err(error) => {
                transaction.rollback().await?;
                Err(error)
            }
        }
    }

    pub(super) async fn load_refresh_context(
        &self,
        feed_id: &str,
    ) -> Result<RefreshContext, SubscriptionRepositoryError> {
        let backend = self.connection().get_database_backend();
        let row = self
            .connection()
            .query_one(Statement::from_sql_and_values(
                backend,
                match backend {
                    DatabaseBackend::Postgres => {
                        "SELECT fetch_url, consecutive_failures FROM feeds WHERE id = $1 AND is_disabled = FALSE"
                    }
                    DatabaseBackend::Sqlite | DatabaseBackend::MySql => {
                        "SELECT fetch_url, consecutive_failures FROM feeds WHERE id = ? AND is_disabled = FALSE"
                    }
                },
                [feed_id.into()],
            ))
            .await?
            .ok_or(SubscriptionRepositoryError::UserNotFound)?;
        let consecutive_failures: i64 = required(&row, "consecutive_failures")?;
        if consecutive_failures < 0 {
            return Err(SubscriptionRepositoryError::CorruptData);
        }
        Ok(RefreshContext {
            fetch_url: required(&row, "fetch_url")?,
            consecutive_failures,
        })
    }

    pub(super) async fn find_owned_subscription(
        &self,
        user_id: &str,
        subscription_id: &str,
    ) -> Result<Option<(String, String)>, SubscriptionRepositoryError> {
        let backend = self.connection().get_database_backend();
        self.connection()
            .query_one(Statement::from_sql_and_values(
                backend,
                match backend {
                    DatabaseBackend::Postgres => {
                        "SELECT id, feed_id FROM subscriptions WHERE id = $1 AND user_id = $2"
                    }
                    DatabaseBackend::Sqlite | DatabaseBackend::MySql => {
                        "SELECT id, feed_id FROM subscriptions WHERE id = ? AND user_id = ?"
                    }
                },
                [subscription_id.into(), user_id.into()],
            ))
            .await?
            .map(|row| Ok((required(&row, "id")?, required(&row, "feed_id")?)))
            .transpose()
    }

    pub(super) async fn load_refresh_dto(
        &self,
        run_id: &str,
    ) -> Result<RefreshDto, SubscriptionRepositoryError> {
        let backend = self.connection().get_database_backend();
        let row = self
            .connection()
            .query_one(Statement::from_sql_and_values(
                backend,
                match backend {
                    DatabaseBackend::Postgres => {
                        "SELECT id, status, http_status, new_count, updated_count, dropped_count,
                                commit_generation FROM feed_refresh_runs WHERE id = $1"
                    }
                    DatabaseBackend::Sqlite | DatabaseBackend::MySql => {
                        "SELECT id, status, http_status, new_count, updated_count, dropped_count,
                                commit_generation FROM feed_refresh_runs WHERE id = ?"
                    }
                },
                [run_id.into()],
            ))
            .await?
            .ok_or(SubscriptionRepositoryError::CorruptData)?;
        let status: String = required(&row, "status")?;
        Ok(RefreshDto {
            run_id: required(&row, "id")?,
            status: status
                .parse()
                .map_err(|_| SubscriptionRepositoryError::CorruptData)?,
            http_status: optional(&row, "http_status")?,
            new_count: required(&row, "new_count")?,
            updated_count: required(&row, "updated_count")?,
            dropped_count: required(&row, "dropped_count")?,
            generation: optional(&row, "commit_generation")?,
        })
    }

    pub(super) async fn subscription_dto(
        &self,
        user_id: &str,
        subscription_id: &str,
        refresh: RefreshDto,
    ) -> Result<SubscriptionDto, SubscriptionRepositoryError> {
        let backend = self.connection().get_database_backend();
        let row = self
            .connection()
            .query_one(Statement::from_sql_and_values(
                backend,
                match backend {
                    DatabaseBackend::Postgres => {
                        "SELECT s.id AS subscription_id, s.feed_id AS feed_id,
                                s.title_override AS title_override, s.start_sequence AS start_sequence,
                                s.read_through_sequence AS read_through_sequence, f.title AS feed_title,
                                f.site_url AS site_url, f.normalized_url AS normalized_url
                         FROM subscriptions s JOIN feeds f ON f.id = s.feed_id
                         WHERE s.id = $1 AND s.user_id = $2"
                    }
                    DatabaseBackend::Sqlite | DatabaseBackend::MySql => {
                        "SELECT s.id AS subscription_id, s.feed_id AS feed_id,
                                s.title_override AS title_override, s.start_sequence AS start_sequence,
                                s.read_through_sequence AS read_through_sequence, f.title AS feed_title,
                                f.site_url AS site_url, f.normalized_url AS normalized_url
                         FROM subscriptions s JOIN feeds f ON f.id = s.feed_id
                         WHERE s.id = ? AND s.user_id = ?"
                    }
                },
                [subscription_id.into(), user_id.into()],
            ))
            .await?
            .ok_or(SubscriptionRepositoryError::UserNotFound)?;
        let normalized_url: String = required(&row, "normalized_url")?;
        let title = optional::<String>(&row, "title_override")?
            .filter(|title| !title.trim().is_empty())
            .or(optional::<String>(&row, "feed_title")?.filter(|title| !title.trim().is_empty()))
            .or_else(|| {
                url::Url::parse(&normalized_url)
                    .ok()
                    .and_then(|url| url.host_str().map(str::to_owned))
            })
            .ok_or(SubscriptionRepositoryError::CorruptData)?;
        Ok(SubscriptionDto {
            subscription_id: required(&row, "subscription_id")?,
            feed_id: required(&row, "feed_id")?,
            title,
            site_url: optional(&row, "site_url")?,
            start_sequence: required(&row, "start_sequence")?,
            read_through_sequence: required(&row, "read_through_sequence")?,
            refresh,
        })
    }
}

async fn ensure_active_user<C>(
    connection: &C,
    backend: DbBackend,
    user_id: &str,
) -> Result<(), SubscriptionRepositoryError>
where
    C: ConnectionTrait,
{
    let row = connection
        .query_one(Statement::from_sql_and_values(
            backend,
            match backend {
                DatabaseBackend::Postgres => "SELECT is_disabled FROM users WHERE id = $1",
                DatabaseBackend::Sqlite | DatabaseBackend::MySql => {
                    "SELECT is_disabled FROM users WHERE id = ?"
                }
            },
            [user_id.into()],
        ))
        .await?
        .ok_or(SubscriptionRepositoryError::UserNotFound)?;
    let disabled: bool = required(&row, "is_disabled")?;
    if disabled {
        Err(SubscriptionRepositoryError::UserNotFound)
    } else {
        Ok(())
    }
}

async fn find_feed_by_hash<C>(
    connection: &C,
    backend: DbBackend,
    hash: &str,
) -> Result<Option<(String, String)>, SubscriptionRepositoryError>
where
    C: ConnectionTrait,
{
    connection
        .query_one(Statement::from_sql_and_values(
            backend,
            match backend {
                DatabaseBackend::Postgres => {
                    "SELECT id, normalized_url FROM feeds WHERE normalized_url_hash = $1"
                }
                DatabaseBackend::Sqlite | DatabaseBackend::MySql => {
                    "SELECT id, normalized_url FROM feeds WHERE normalized_url_hash = ?"
                }
            },
            [hash.into()],
        ))
        .await?
        .map(|row| Ok((required(&row, "id")?, required(&row, "normalized_url")?)))
        .transpose()
}

async fn lock_feed_head<C>(
    connection: &C,
    backend: DbBackend,
    feed_id: &str,
) -> Result<(String, i64), SubscriptionRepositoryError>
where
    C: ConnectionTrait,
{
    let sql = match backend {
        DatabaseBackend::Sqlite => {
            "SELECT normalized_url, entry_sequence_head FROM feeds WHERE id = ?"
        }
        DatabaseBackend::Postgres => {
            "SELECT normalized_url, entry_sequence_head FROM feeds WHERE id = $1 FOR UPDATE"
        }
        DatabaseBackend::MySql => {
            "SELECT normalized_url, entry_sequence_head FROM feeds WHERE id = ? FOR UPDATE"
        }
    };
    let row = connection
        .query_one(Statement::from_sql_and_values(
            backend,
            sql,
            [feed_id.into()],
        ))
        .await?
        .ok_or(SubscriptionRepositoryError::CorruptData)?;
    let head: i64 = required(&row, "entry_sequence_head")?;
    if head < 0 {
        return Err(SubscriptionRepositoryError::CorruptData);
    }
    Ok((required(&row, "normalized_url")?, head))
}

async fn find_subscription<C>(
    connection: &C,
    backend: DbBackend,
    user_id: &str,
    feed_id: &str,
) -> Result<Option<String>, SubscriptionRepositoryError>
where
    C: ConnectionTrait,
{
    connection
        .query_one(Statement::from_sql_and_values(
            backend,
            match backend {
                DatabaseBackend::Postgres => {
                    "SELECT id FROM subscriptions WHERE user_id = $1 AND feed_id = $2"
                }
                DatabaseBackend::Sqlite | DatabaseBackend::MySql => {
                    "SELECT id FROM subscriptions WHERE user_id = ? AND feed_id = ?"
                }
            },
            [user_id.into(), feed_id.into()],
        ))
        .await?
        .map(|row| required(&row, "id"))
        .transpose()
}

async fn find_subscribe_run<C>(
    connection: &C,
    backend: DbBackend,
    feed_id: &str,
    idempotency_key: &str,
) -> Result<
    Option<(String, Option<String>, RefreshTrigger, RefreshStatus)>,
    SubscriptionRepositoryError,
>
where
    C: ConnectionTrait,
{
    connection
        .query_one(Statement::from_sql_and_values(
            backend,
            match backend {
                DatabaseBackend::Postgres => {
                    "SELECT id, requested_by_user_id, trigger_kind, status
                     FROM feed_refresh_runs WHERE feed_id = $1 AND idempotency_key = $2"
                }
                DatabaseBackend::Sqlite | DatabaseBackend::MySql => {
                    "SELECT id, requested_by_user_id, trigger_kind, status
                     FROM feed_refresh_runs WHERE feed_id = ? AND idempotency_key = ?"
                }
            },
            [feed_id.into(), idempotency_key.into()],
        ))
        .await?
        .map(|row| {
            let trigger: String = required(&row, "trigger_kind")?;
            let status: String = required(&row, "status")?;
            Ok((
                required(&row, "id")?,
                optional(&row, "requested_by_user_id")?,
                trigger
                    .parse()
                    .map_err(|_| SubscriptionRepositoryError::CorruptData)?,
                status
                    .parse()
                    .map_err(|_| SubscriptionRepositoryError::CorruptData)?,
            ))
        })
        .transpose()
}

fn insert_feed_statement(
    backend: DbBackend,
    feed_id: &str,
    source_url: &str,
    normalized: &NormalizedFeedUrl,
) -> Statement {
    let clock = match backend {
        DatabaseBackend::Sqlite => "strftime('%Y-%m-%dT%H:%M:%f000Z','now')",
        DatabaseBackend::Postgres => "clock_timestamp()",
        DatabaseBackend::MySql => "UTC_TIMESTAMP(6)",
    };
    let placeholders = if backend == DatabaseBackend::Postgres {
        ["$1", "$2", "$3", "$4", "$5"]
    } else {
        ["?", "?", "?", "?", "?"]
    };
    Statement::from_sql_and_values(
        backend,
        format!(
            "INSERT INTO feeds (
                id, source_url, normalized_url, normalized_url_hash, fetch_url,
                entry_sequence_head, next_fetch_at, consecutive_failures, is_disabled,
                lease_token, created_at, updated_at
             ) VALUES ({}, {}, {}, {}, {}, 0, {clock}, 0, FALSE, 0, {clock}, {clock})",
            placeholders[0], placeholders[1], placeholders[2], placeholders[3], placeholders[4]
        ),
        [
            feed_id.into(),
            source_url.into(),
            normalized.complete().into(),
            normalized.url_hash().into(),
            normalized.complete().into(),
        ],
    )
}

fn clear_orphaned_statement(backend: DbBackend, feed_id: &str) -> Statement {
    let sql = match backend {
        DatabaseBackend::Sqlite => {
            "UPDATE feeds SET orphaned_at = NULL,
                    updated_at = strftime('%Y-%m-%dT%H:%M:%f000Z','now') WHERE id = ?"
        }
        DatabaseBackend::Postgres => {
            "UPDATE feeds SET orphaned_at = NULL, updated_at = clock_timestamp() WHERE id = $1"
        }
        DatabaseBackend::MySql => {
            "UPDATE feeds SET orphaned_at = NULL, updated_at = UTC_TIMESTAMP(6) WHERE id = ?"
        }
    };
    Statement::from_sql_and_values(backend, sql, [feed_id.into()])
}

fn insert_subscription_statement(
    backend: DbBackend,
    subscription_id: &str,
    user_id: &str,
    feed_id: &str,
    head: i64,
) -> Statement {
    let clock = match backend {
        DatabaseBackend::Sqlite => "strftime('%Y-%m-%dT%H:%M:%f000Z','now')",
        DatabaseBackend::Postgres => "clock_timestamp()",
        DatabaseBackend::MySql => "UTC_TIMESTAMP(6)",
    };
    let placeholders = if backend == DatabaseBackend::Postgres {
        ["$1", "$2", "$3", "$4", "$5"]
    } else {
        ["?", "?", "?", "?", "?"]
    };
    Statement::from_sql_and_values(
        backend,
        format!(
            "INSERT INTO subscriptions (
                id, user_id, feed_id, position, start_sequence, read_through_sequence,
                state_revision, created_at, updated_at
             ) VALUES ({}, {}, {}, 0, {}, {}, 0, {clock}, {clock})",
            placeholders[0], placeholders[1], placeholders[2], placeholders[3], placeholders[4]
        ),
        [
            subscription_id.into(),
            user_id.into(),
            feed_id.into(),
            head.into(),
            head.into(),
        ],
    )
}

fn insert_subscribe_run_statement(
    backend: DbBackend,
    run_id: &str,
    feed_id: &str,
    user_id: &str,
    idempotency_key: &str,
) -> Statement {
    let clock = match backend {
        DatabaseBackend::Sqlite => "strftime('%Y-%m-%dT%H:%M:%f000Z','now')",
        DatabaseBackend::Postgres => "clock_timestamp()",
        DatabaseBackend::MySql => "UTC_TIMESTAMP(6)",
    };
    let placeholders = if backend == DatabaseBackend::Postgres {
        ["$1", "$2", "$3", "$4"]
    } else {
        ["?", "?", "?", "?"]
    };
    Statement::from_sql_and_values(
        backend,
        format!(
            "INSERT INTO feed_refresh_runs (
                id, feed_id, requested_by_user_id, trigger_kind, status, idempotency_key, queued_at
             ) VALUES ({}, {}, {}, 'SUBSCRIBE', 'QUEUED', {}, {clock})",
            placeholders[0], placeholders[1], placeholders[2], placeholders[3]
        ),
        [
            run_id.into(),
            feed_id.into(),
            user_id.into(),
            idempotency_key.into(),
        ],
    )
}

fn required<T>(row: &QueryResult, column: &str) -> Result<T, SubscriptionRepositoryError>
where
    T: sea_orm::TryGetable,
{
    row.try_get("", column)
        .map_err(|_| SubscriptionRepositoryError::CorruptData)
}

fn optional<T>(row: &QueryResult, column: &str) -> Result<Option<T>, SubscriptionRepositoryError>
where
    T: sea_orm::TryGetable,
{
    row.try_get("", column)
        .map_err(|_| SubscriptionRepositoryError::CorruptData)
}
