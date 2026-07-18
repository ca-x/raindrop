use sea_orm::{
    ConnectionTrait, DatabaseBackend, DatabaseTransaction, DbBackend, QueryResult, Statement,
    TransactionTrait, Value,
};

use super::{
    EntryStateDto, FeedRepository, RepositoryError, UpdateEntryState, query::validate_uuid,
};

struct LockedSubscription {
    subscription_id: String,
}

struct CurrentState {
    user_id: String,
    entry_id: String,
    feed_id: String,
    feed_sequence: i64,
    read_through_sequence: i64,
    read_override: Option<bool>,
    is_starred: bool,
    revision: i64,
    state_exists: bool,
}

struct NextState {
    entry_id: String,
    read_override: Option<bool>,
    is_read: bool,
    is_starred: bool,
}

impl CurrentState {
    fn apply(&self, patch: UpdateEntryState) -> NextState {
        let base_read = self.feed_sequence <= self.read_through_sequence;
        let read_override = match patch.is_read {
            Some(requested) if requested == base_read => None,
            Some(requested) => Some(requested),
            None => self.read_override,
        };
        let is_starred = patch.is_starred.unwrap_or(self.is_starred);
        let is_read = read_override.unwrap_or(base_read);
        NextState {
            entry_id: self.entry_id.clone(),
            read_override,
            is_read,
            is_starred,
        }
    }
}

impl NextState {
    fn is_neutral(&self) -> bool {
        self.read_override.is_none() && !self.is_starred
    }

    fn same_storage(&self, current: &CurrentState) -> bool {
        self.read_override == current.read_override && self.is_starred == current.is_starred
    }

    fn dto(&self) -> EntryStateDto {
        EntryStateDto {
            entry_id: self.entry_id.clone(),
            is_read: self.is_read,
            is_starred: self.is_starred,
        }
    }
}

impl FeedRepository {
    pub async fn update_state_for_user(
        &self,
        user_id: &str,
        entry_id: &str,
        patch: UpdateEntryState,
    ) -> Result<Option<EntryStateDto>, RepositoryError> {
        validate_uuid(user_id).map_err(|()| RepositoryError::InvalidUserId)?;
        validate_uuid(entry_id).map_err(|()| RepositoryError::InvalidEntryId)?;
        if patch.is_read.is_none() && patch.is_starred.is_none() {
            return Err(RepositoryError::InvalidStatePatch);
        }

        let backend = self.connection().get_database_backend();
        let transaction = self.connection().begin().await?;
        let Some(locked) =
            lock_visible_subscription(&transaction, backend, user_id, entry_id).await?
        else {
            transaction.rollback().await?;
            return Ok(None);
        };
        let current =
            load_current_state_after_lock(&transaction, backend, &locked, user_id, entry_id)
                .await?;

        let next = current.apply(patch);
        if next.is_neutral() {
            if current.state_exists {
                delete_state_row(&transaction, backend, &current).await?;
            }
            transaction.commit().await?;
            return Ok(Some(next.dto()));
        }
        if next.same_storage(&current) {
            transaction.commit().await?;
            return Ok(Some(next.dto()));
        }

        if current.state_exists {
            update_state_row(&transaction, backend, &current, &next).await?;
        } else {
            insert_state_row(&transaction, backend, &current, &next).await?;
        }
        transaction.commit().await?;
        Ok(Some(next.dto()))
    }
}

async fn lock_visible_subscription(
    transaction: &DatabaseTransaction,
    backend: DbBackend,
    user_id: &str,
    entry_id: &str,
) -> Result<Option<LockedSubscription>, RepositoryError> {
    if backend == DatabaseBackend::Sqlite {
        transaction
            .execute(Statement::from_sql_and_values(
                backend,
                "UPDATE subscriptions
                 SET state_revision = state_revision
                 WHERE user_id = ?
                   AND feed_id = (
                       SELECT e.feed_id
                       FROM entries e
                       WHERE e.id = ?
                         AND e.feed_id = subscriptions.feed_id
                         AND e.feed_sequence > subscriptions.start_sequence
                   )",
                [user_id.into(), entry_id.into()],
            ))
            .await?;
    }

    let mut sql = Sql::new(backend);
    let user = sql.bind(user_id);
    let entry = sql.bind(entry_id);
    let lock = match backend {
        DatabaseBackend::Postgres => " LIMIT 1 FOR UPDATE OF s",
        DatabaseBackend::MySql => " LIMIT 1 FOR UPDATE",
        DatabaseBackend::Sqlite => " LIMIT 1",
    };
    transaction
        .query_one(sql.finish(format!(
            "SELECT s.id AS subscription_id
             FROM subscriptions s
             JOIN entries e ON e.feed_id = s.feed_id
             WHERE s.user_id = {user}
               AND e.id = {entry}
               AND e.feed_sequence > s.start_sequence{lock}"
        )))
        .await?
        .map(|row| {
            Ok(LockedSubscription {
                subscription_id: required(&row, "subscription_id")?,
            })
        })
        .transpose()
}

async fn load_current_state_after_lock(
    transaction: &DatabaseTransaction,
    backend: DbBackend,
    locked: &LockedSubscription,
    user_id: &str,
    entry_id: &str,
) -> Result<CurrentState, RepositoryError> {
    let mut sql = Sql::new(backend);
    let user = sql.bind(user_id);
    let subscription = sql.bind(locked.subscription_id.as_str());
    let entry = sql.bind(entry_id);
    let lock = if backend == DatabaseBackend::MySql {
        " FOR UPDATE"
    } else {
        ""
    };
    let row = transaction
        .query_one(sql.finish(format!(
            "SELECT s.user_id AS user_id, e.id AS entry_id, e.feed_id AS feed_id,
                    e.feed_sequence AS feed_sequence,
                    s.read_through_sequence AS read_through_sequence,
                    es.read_override AS read_override,
                    COALESCE(es.is_starred, FALSE) AS is_starred,
                    COALESCE(es.revision, 0) AS revision,
                    CASE WHEN es.user_id IS NULL THEN FALSE ELSE TRUE END AS state_exists
             FROM subscriptions s
             JOIN entries e ON e.feed_id = s.feed_id
             LEFT JOIN entry_states es ON es.user_id = s.user_id
                                      AND es.entry_id = e.id
                                      AND es.feed_id = e.feed_id
                                      AND es.feed_sequence = e.feed_sequence
             WHERE s.user_id = {user}
               AND s.id = {subscription}
               AND e.id = {entry}
               AND e.feed_id = s.feed_id
               AND e.feed_sequence > s.start_sequence
             LIMIT 1{lock}"
        )))
        .await?
        .ok_or(RepositoryError::CorruptData)?;
    Ok(CurrentState {
        user_id: required(&row, "user_id")?,
        entry_id: required(&row, "entry_id")?,
        feed_id: required(&row, "feed_id")?,
        feed_sequence: required(&row, "feed_sequence")?,
        read_through_sequence: required(&row, "read_through_sequence")?,
        read_override: optional(&row, "read_override")?,
        is_starred: required(&row, "is_starred")?,
        revision: required(&row, "revision")?,
        state_exists: required(&row, "state_exists")?,
    })
}

async fn delete_state_row(
    transaction: &DatabaseTransaction,
    backend: DbBackend,
    current: &CurrentState,
) -> Result<(), RepositoryError> {
    let placeholders = Placeholders::new(backend);
    let statement = Statement::from_sql_and_values(
        backend,
        format!(
            "DELETE FROM entry_states
             WHERE user_id = {} AND entry_id = {} AND revision = {}",
            placeholders.at(1),
            placeholders.at(2),
            placeholders.at(3),
        ),
        vec![
            Value::from(current.user_id.as_str()),
            Value::from(current.entry_id.as_str()),
            current.revision.into(),
        ],
    );
    if transaction.execute(statement).await?.rows_affected() != 1 {
        return Err(RepositoryError::CorruptData);
    }
    Ok(())
}

async fn update_state_row(
    transaction: &DatabaseTransaction,
    backend: DbBackend,
    current: &CurrentState,
    next: &NextState,
) -> Result<(), RepositoryError> {
    let starred_at = match (current.is_starred, next.is_starred) {
        (false, true) => database_clock(backend),
        (true, false) => "NULL",
        _ => "starred_at",
    };
    let clock = database_clock(backend);
    let placeholders = Placeholders::new(backend);
    let statement = Statement::from_sql_and_values(
        backend,
        format!(
            "UPDATE entry_states
             SET read_override = {}, is_starred = {}, starred_at = {starred_at},
                 revision = revision + 1, updated_at = {clock}
             WHERE user_id = {} AND entry_id = {} AND revision = {}",
            placeholders.at(1),
            placeholders.at(2),
            placeholders.at(3),
            placeholders.at(4),
            placeholders.at(5),
        ),
        vec![
            next.read_override.into(),
            next.is_starred.into(),
            Value::from(current.user_id.as_str()),
            Value::from(current.entry_id.as_str()),
            current.revision.into(),
        ],
    );
    if transaction.execute(statement).await?.rows_affected() != 1 {
        return Err(RepositoryError::CorruptData);
    }
    Ok(())
}

async fn insert_state_row(
    transaction: &DatabaseTransaction,
    backend: DbBackend,
    current: &CurrentState,
    next: &NextState,
) -> Result<(), RepositoryError> {
    let clock = database_clock(backend);
    let starred_at = if next.is_starred { clock } else { "NULL" };
    let placeholders = Placeholders::new(backend);
    let statement = Statement::from_sql_and_values(
        backend,
        format!(
            "INSERT INTO entry_states (
                 user_id, entry_id, feed_id, feed_sequence, read_override,
                 is_starred, starred_at, revision, updated_at
             ) VALUES ({}, {}, {}, {}, {}, {}, {starred_at}, 1, {clock})",
            placeholders.at(1),
            placeholders.at(2),
            placeholders.at(3),
            placeholders.at(4),
            placeholders.at(5),
            placeholders.at(6),
        ),
        vec![
            Value::from(current.user_id.as_str()),
            Value::from(current.entry_id.as_str()),
            Value::from(current.feed_id.as_str()),
            current.feed_sequence.into(),
            next.read_override.into(),
            next.is_starred.into(),
        ],
    );
    if transaction.execute(statement).await?.rows_affected() != 1 {
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
