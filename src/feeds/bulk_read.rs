use sea_orm::{
    ConnectionTrait, DatabaseBackend, DatabaseTransaction, DbBackend, QueryResult, Statement,
    TransactionTrait, Value,
};

use super::{FeedRepository, RepositoryError, query::validate_uuid};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MarkReadScope {
    All,
    Feed(String),
    Category(String),
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct MarkReadResult {
    pub changed_subscriptions: u16,
}

struct LockedSubscription {
    id: String,
    feed_id: String,
    start_sequence: i64,
    read_through_sequence: i64,
    state_revision: i64,
}

impl FeedRepository {
    pub async fn mark_read_for_user(
        &self,
        user_id: &str,
        scope: MarkReadScope,
        snapshot_generation: i64,
    ) -> Result<MarkReadResult, RepositoryError> {
        validate_uuid(user_id).map_err(|()| RepositoryError::InvalidUserId)?;
        validate_mark_read_request(&scope, snapshot_generation)?;

        let backend = self.connection().get_database_backend();
        let transaction = self.connection().begin().await?;
        let result =
            mark_read_in_transaction(&transaction, backend, user_id, &scope, snapshot_generation)
                .await;
        match result {
            Ok(result) => {
                transaction.commit().await?;
                Ok(result)
            }
            Err(error) => {
                transaction.rollback().await?;
                Err(error)
            }
        }
    }
}

fn validate_mark_read_request(
    scope: &MarkReadScope,
    snapshot_generation: i64,
) -> Result<(), RepositoryError> {
    if snapshot_generation < 0 {
        return Err(RepositoryError::InvalidSnapshotGeneration);
    }
    match scope {
        MarkReadScope::All => Ok(()),
        MarkReadScope::Feed(feed_id) => {
            validate_uuid(feed_id).map_err(|()| RepositoryError::InvalidFeedId)
        }
        MarkReadScope::Category(category_id) => {
            validate_uuid(category_id).map_err(|()| RepositoryError::InvalidCategoryId)
        }
    }
}

async fn mark_read_in_transaction(
    transaction: &DatabaseTransaction,
    backend: DbBackend,
    user_id: &str,
    scope: &MarkReadScope,
    snapshot_generation: i64,
) -> Result<MarkReadResult, RepositoryError> {
    lock_active_user(transaction, backend, user_id).await?;
    let current_generation = read_current_generation(transaction, backend).await?;
    if snapshot_generation > current_generation {
        return Err(RepositoryError::InvalidSnapshotGeneration);
    }
    let subscriptions = lock_target_subscriptions(transaction, backend, user_id, scope).await?;
    let mut changed_subscriptions = 0_u16;
    for subscription in subscriptions {
        let Some(target_sequence) =
            target_sequence(transaction, backend, &subscription, snapshot_generation).await?
        else {
            continue;
        };
        let starred_updates = clear_starred_overrides(
            transaction,
            backend,
            user_id,
            &subscription,
            target_sequence,
        )
        .await?;
        let neutral_deletes = delete_neutral_overrides(
            transaction,
            backend,
            user_id,
            &subscription,
            target_sequence,
        )
        .await?;
        let next_frontier = subscription.read_through_sequence.max(target_sequence);
        if next_frontier == subscription.read_through_sequence
            && starred_updates == 0
            && neutral_deletes == 0
        {
            continue;
        }
        update_subscription_frontier(transaction, backend, user_id, &subscription, next_frontier)
            .await?;
        changed_subscriptions = changed_subscriptions
            .checked_add(1)
            .ok_or(RepositoryError::CorruptData)?;
    }
    Ok(MarkReadResult {
        changed_subscriptions,
    })
}

async fn lock_active_user<C>(
    connection: &C,
    backend: DbBackend,
    user_id: &str,
) -> Result<(), RepositoryError>
where
    C: ConnectionTrait,
{
    match backend {
        DatabaseBackend::Sqlite => {
            let result = connection
                .execute(Statement::from_sql_and_values(
                    backend,
                    "UPDATE users SET is_disabled = is_disabled
                     WHERE id = ? AND is_disabled = FALSE",
                    [user_id.into()],
                ))
                .await?;
            if result.rows_affected() != 1 {
                return Err(RepositoryError::InvalidUserId);
            }
        }
        DatabaseBackend::Postgres | DatabaseBackend::MySql => {
            let sql = if backend == DatabaseBackend::Postgres {
                "SELECT is_disabled FROM users WHERE id = $1 FOR UPDATE"
            } else {
                "SELECT is_disabled FROM users WHERE id = ? FOR UPDATE"
            };
            let row = connection
                .query_one(Statement::from_sql_and_values(
                    backend,
                    sql,
                    [user_id.into()],
                ))
                .await?
                .ok_or(RepositoryError::InvalidUserId)?;
            let disabled: bool = required(&row, "is_disabled")?;
            if disabled {
                return Err(RepositoryError::InvalidUserId);
            }
        }
    }
    Ok(())
}

async fn read_current_generation<C>(
    connection: &C,
    backend: DbBackend,
) -> Result<i64, RepositoryError>
where
    C: ConnectionTrait,
{
    let sql = match backend {
        DatabaseBackend::Postgres => "SELECT value FROM rss_counters WHERE key = $1",
        DatabaseBackend::Sqlite => "SELECT value FROM rss_counters WHERE key = ?",
        DatabaseBackend::MySql => "SELECT value FROM rss_counters WHERE `key` = ?",
    };
    let row = connection
        .query_one(Statement::from_sql_and_values(
            backend,
            sql,
            ["INGEST_GENERATION".into()],
        ))
        .await?
        .ok_or(RepositoryError::CorruptData)?;
    let generation: i64 = required(&row, "value")?;
    if generation < 0 {
        return Err(RepositoryError::CorruptData);
    }
    Ok(generation)
}

async fn lock_target_subscriptions<C>(
    connection: &C,
    backend: DbBackend,
    user_id: &str,
    scope: &MarkReadScope,
) -> Result<Vec<LockedSubscription>, RepositoryError>
where
    C: ConnectionTrait,
{
    let mut sql = Sql::new(backend);
    let user = sql.bind(user_id);
    let mut text = format!(
        "SELECT id, feed_id, start_sequence, read_through_sequence, state_revision
         FROM subscriptions WHERE user_id = {user}"
    );
    match scope {
        MarkReadScope::All => {}
        MarkReadScope::Feed(feed_id) => {
            let feed = sql.bind(feed_id.as_str());
            text.push_str(&format!(" AND feed_id = {feed}"));
        }
        MarkReadScope::Category(category_id) => {
            let category = sql.bind(category_id.as_str());
            text.push_str(&format!(" AND category_id = {category}"));
        }
    }
    text.push_str(" ORDER BY id");
    if backend != DatabaseBackend::Sqlite {
        text.push_str(" FOR UPDATE");
    }
    connection
        .query_all(sql.finish(text))
        .await?
        .into_iter()
        .map(|row| {
            let start_sequence = required(&row, "start_sequence")?;
            let read_through_sequence = required(&row, "read_through_sequence")?;
            let state_revision = required(&row, "state_revision")?;
            if start_sequence < 0 || read_through_sequence < 0 || state_revision < 0 {
                return Err(RepositoryError::CorruptData);
            }
            Ok(LockedSubscription {
                id: required(&row, "id")?,
                feed_id: required(&row, "feed_id")?,
                start_sequence,
                read_through_sequence,
                state_revision,
            })
        })
        .collect()
}

async fn target_sequence<C>(
    connection: &C,
    backend: DbBackend,
    subscription: &LockedSubscription,
    snapshot_generation: i64,
) -> Result<Option<i64>, RepositoryError>
where
    C: ConnectionTrait,
{
    let placeholders = Placeholders::new(backend);
    let row = connection
        .query_one(Statement::from_sql_and_values(
            backend,
            format!(
                "SELECT MAX(feed_sequence) AS target_sequence FROM entries
                 WHERE feed_id = {} AND feed_sequence > {} AND ingest_generation <= {}",
                placeholders.at(1),
                placeholders.at(2),
                placeholders.at(3),
            ),
            [
                Value::from(subscription.feed_id.as_str()),
                subscription.start_sequence.into(),
                snapshot_generation.into(),
            ],
        ))
        .await?
        .ok_or(RepositoryError::CorruptData)?;
    let target: Option<i64> = optional(&row, "target_sequence")?;
    if target.is_some_and(|value| value < 0) {
        return Err(RepositoryError::CorruptData);
    }
    Ok(target)
}

async fn clear_starred_overrides<C>(
    connection: &C,
    backend: DbBackend,
    user_id: &str,
    subscription: &LockedSubscription,
    target_sequence: i64,
) -> Result<u64, RepositoryError>
where
    C: ConnectionTrait,
{
    let placeholders = Placeholders::new(backend);
    let clock = database_clock(backend);
    Ok(connection
        .execute(Statement::from_sql_and_values(
            backend,
            format!(
                "UPDATE entry_states SET read_override = NULL, revision = revision + 1,
                    updated_at = {clock}
                 WHERE user_id = {} AND feed_id = {} AND feed_sequence > {}
                   AND feed_sequence <= {} AND read_override IS NOT NULL
                   AND is_starred = TRUE",
                placeholders.at(1),
                placeholders.at(2),
                placeholders.at(3),
                placeholders.at(4),
            ),
            [
                Value::from(user_id),
                Value::from(subscription.feed_id.as_str()),
                subscription.start_sequence.into(),
                target_sequence.into(),
            ],
        ))
        .await?
        .rows_affected())
}

async fn delete_neutral_overrides<C>(
    connection: &C,
    backend: DbBackend,
    user_id: &str,
    subscription: &LockedSubscription,
    target_sequence: i64,
) -> Result<u64, RepositoryError>
where
    C: ConnectionTrait,
{
    let placeholders = Placeholders::new(backend);
    Ok(connection
        .execute(Statement::from_sql_and_values(
            backend,
            format!(
                "DELETE FROM entry_states
                 WHERE user_id = {} AND feed_id = {} AND feed_sequence > {}
                   AND feed_sequence <= {} AND read_override IS NOT NULL
                   AND is_starred = FALSE",
                placeholders.at(1),
                placeholders.at(2),
                placeholders.at(3),
                placeholders.at(4),
            ),
            [
                Value::from(user_id),
                Value::from(subscription.feed_id.as_str()),
                subscription.start_sequence.into(),
                target_sequence.into(),
            ],
        ))
        .await?
        .rows_affected())
}

async fn update_subscription_frontier<C>(
    connection: &C,
    backend: DbBackend,
    user_id: &str,
    subscription: &LockedSubscription,
    next_frontier: i64,
) -> Result<(), RepositoryError>
where
    C: ConnectionTrait,
{
    let placeholders = Placeholders::new(backend);
    let clock = database_clock(backend);
    let result = connection
        .execute(Statement::from_sql_and_values(
            backend,
            format!(
                "UPDATE subscriptions
                 SET read_through_sequence = {}, state_revision = state_revision + 1,
                     updated_at = {clock}
                 WHERE id = {} AND user_id = {} AND state_revision = {}",
                placeholders.at(1),
                placeholders.at(2),
                placeholders.at(3),
                placeholders.at(4),
            ),
            [
                next_frontier.into(),
                Value::from(subscription.id.as_str()),
                Value::from(user_id),
                subscription.state_revision.into(),
            ],
        ))
        .await?;
    if result.rows_affected() != 1 {
        return Err(RepositoryError::CorruptData);
    }
    Ok(())
}

fn database_clock(backend: DbBackend) -> &'static str {
    match backend {
        DatabaseBackend::Sqlite => "strftime('%Y-%m-%dT%H:%M:%f000Z','now')",
        DatabaseBackend::Postgres => "clock_timestamp()",
        DatabaseBackend::MySql => "UTC_TIMESTAMP(6)",
    }
}

fn required<T>(row: &QueryResult, column: &str) -> Result<T, RepositoryError>
where
    T: sea_orm::TryGetable,
{
    row.try_get("", column)
        .map_err(|_| RepositoryError::CorruptData)
}

fn optional<T>(row: &QueryResult, column: &str) -> Result<Option<T>, RepositoryError>
where
    T: sea_orm::TryGetable,
{
    row.try_get("", column)
        .map_err(|_| RepositoryError::CorruptData)
}

struct Sql {
    backend: DbBackend,
    values: Vec<Value>,
}

impl Sql {
    fn new(backend: DbBackend) -> Self {
        Self {
            backend,
            values: Vec::new(),
        }
    }

    fn bind<T>(&mut self, value: T) -> String
    where
        T: Into<Value>,
    {
        self.values.push(value.into());
        Placeholders::new(self.backend).at(self.values.len())
    }

    fn finish(self, text: String) -> Statement {
        Statement::from_sql_and_values(self.backend, text, self.values)
    }
}

struct Placeholders {
    backend: DbBackend,
}

impl Placeholders {
    fn new(backend: DbBackend) -> Self {
        Self { backend }
    }

    fn at(&self, position: usize) -> String {
        if self.backend == DatabaseBackend::Postgres {
            format!("${position}")
        } else {
            "?".to_owned()
        }
    }
}
