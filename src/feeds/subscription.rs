use std::fmt;

use base64::Engine;
use sea_orm::{
    ConnectionTrait, DatabaseBackend, DbBackend, DbErr, QueryResult, Statement, TransactionTrait,
    Value,
};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use uuid::Uuid;

use super::lifecycle::is_unique_violation;
use super::query::{RepositoryError, validate_uuid};
use super::{
    FeedRepository, ListSubscriptionsQuery, NormalizedFeedUrl, RefreshDto, RefreshStatus,
    RefreshTrigger, SubscriptionDto, SubscriptionListItemDto, SubscriptionPage,
};

const SUBSCRIPTION_CURSOR_VERSION: u8 = 1;
const SUBSCRIPTION_CURSOR_ORDER: &str = "CREATED_DESC_ID_DESC";
const MAX_SUBSCRIPTION_CURSOR_BYTES: usize = 1_024;
const MAX_SUBSCRIPTION_LIMIT: u16 = 100;

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SubscriptionCursorV1 {
    version: u8,
    user_hash: String,
    order: String,
    created_at_us: i64,
    subscription_id: String,
}

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
    pub async fn list_subscriptions_for_user(
        &self,
        user_id: &str,
        query: ListSubscriptionsQuery,
    ) -> Result<SubscriptionPage, RepositoryError> {
        validate_uuid(user_id).map_err(|()| RepositoryError::InvalidUserId)?;
        validate_subscription_query(&query)?;
        let expected_user_hash = subscription_user_hash(user_id);
        let cursor = query
            .cursor
            .as_deref()
            .map(decode_subscription_cursor)
            .transpose()?;
        if cursor
            .as_ref()
            .is_some_and(|cursor| cursor.user_hash != expected_user_hash)
        {
            return Err(RepositoryError::InvalidCursor);
        }
        let backend = self.connection().get_database_backend();
        let rows = self
            .connection()
            .query_all(subscription_list_statement(
                backend,
                user_id,
                query.limit,
                cursor.as_ref(),
            ))
            .await?;
        let mut projected = rows
            .into_iter()
            .map(decode_subscription_projection)
            .collect::<Result<Vec<_>, _>>()?;
        let has_more = projected.len() > usize::from(query.limit);
        if has_more {
            projected.pop();
        }
        let next_cursor = if has_more {
            let last = projected.last().ok_or(RepositoryError::CorruptData)?;
            Some(encode_subscription_cursor(&SubscriptionCursorV1 {
                version: SUBSCRIPTION_CURSOR_VERSION,
                user_hash: expected_user_hash,
                order: SUBSCRIPTION_CURSOR_ORDER.to_owned(),
                created_at_us: timestamp_to_micros(last.created_at)?,
                subscription_id: last.item.subscription_id.clone(),
            })?)
        } else {
            None
        };

        Ok(SubscriptionPage {
            items: projected.into_iter().map(|row| row.item).collect(),
            next_cursor,
        })
    }

    pub async fn get_subscription_for_user(
        &self,
        user_id: &str,
        subscription_id: &str,
    ) -> Result<Option<SubscriptionListItemDto>, RepositoryError> {
        validate_uuid(user_id).map_err(|()| RepositoryError::InvalidUserId)?;
        let backend = self.connection().get_database_backend();
        self.connection()
            .query_one(subscription_detail_statement(
                backend,
                user_id,
                subscription_id,
            ))
            .await?
            .map(decode_subscription_projection)
            .transpose()
            .map(|projection| projection.map(|row| row.item))
    }

    #[doc(hidden)]
    pub async fn explain_list_subscriptions_for_user(
        &self,
        user_id: &str,
        query: ListSubscriptionsQuery,
    ) -> Result<Vec<String>, RepositoryError> {
        validate_uuid(user_id).map_err(|()| RepositoryError::InvalidUserId)?;
        validate_subscription_query(&query)?;
        let expected_user_hash = subscription_user_hash(user_id);
        let cursor = query
            .cursor
            .as_deref()
            .map(decode_subscription_cursor)
            .transpose()?;
        if cursor
            .as_ref()
            .is_some_and(|cursor| cursor.user_hash != expected_user_hash)
        {
            return Err(RepositoryError::InvalidCursor);
        }
        let backend = self.connection().get_database_backend();
        let statement = subscription_list_statement(backend, user_id, query.limit, cursor.as_ref());
        self.connection()
            .query_all(subscription_explain_statement(statement))
            .await?
            .into_iter()
            .map(|row| subscription_explain_line(backend, &row))
            .collect()
    }

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
                                commit_generation AS generation FROM feed_refresh_runs WHERE id = $1"
                    }
                    DatabaseBackend::Sqlite | DatabaseBackend::MySql => {
                        "SELECT id, status, http_status, new_count, updated_count, dropped_count,
                                commit_generation AS generation FROM feed_refresh_runs WHERE id = ?"
                    }
                },
                [run_id.into()],
            ))
            .await?
            .ok_or(SubscriptionRepositoryError::CorruptData)?;
        decode_refresh_row(&row, "").map_err(|()| SubscriptionRepositoryError::CorruptData)
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

struct SubscriptionProjection {
    item: SubscriptionListItemDto,
    created_at: OffsetDateTime,
}

fn validate_subscription_query(query: &ListSubscriptionsQuery) -> Result<(), RepositoryError> {
    if !(1..=MAX_SUBSCRIPTION_LIMIT).contains(&query.limit) {
        return Err(RepositoryError::InvalidLimit);
    }
    Ok(())
}

fn subscription_list_statement(
    backend: DbBackend,
    user_id: &str,
    limit: u16,
    cursor: Option<&SubscriptionCursorV1>,
) -> Statement {
    let mut sql = SubscriptionSql::new(backend);
    let user = sql.bind(user_id);
    let mut text = subscription_projection_sql(&user);
    if let Some(cursor) = cursor {
        let created_before = sql.bind(
            micros_to_timestamp(cursor.created_at_us)
                .expect("validated subscription cursor timestamp is representable"),
        );
        let created_tie = sql.bind(
            micros_to_timestamp(cursor.created_at_us)
                .expect("validated subscription cursor timestamp is representable"),
        );
        let id_before = sql.bind(cursor.subscription_id.as_str());
        text.push_str(&format!(
            " WHERE (s.created_at < {created_before}
                     OR (s.created_at = {created_tie} AND s.id < {id_before}))"
        ));
    }
    let bound_limit = sql.bind(i64::from(limit) + 1);
    text.push_str(&format!(
        " ORDER BY s.created_at DESC, s.id DESC LIMIT {bound_limit}"
    ));
    sql.finish(text)
}

fn subscription_detail_statement(
    backend: DbBackend,
    user_id: &str,
    subscription_id: &str,
) -> Statement {
    let mut sql = SubscriptionSql::new(backend);
    let user = sql.bind(user_id);
    let subscription = sql.bind(subscription_id);
    let mut text = subscription_projection_sql(&user);
    text.push_str(&format!(" WHERE s.id = {subscription} LIMIT 1"));
    sql.finish(text)
}

fn subscription_projection_sql(user: &str) -> String {
    format!(
        "WITH user_subscriptions AS (
            SELECT id, user_id, feed_id, title_override, start_sequence,
                   read_through_sequence, created_at
            FROM subscriptions
            WHERE user_id = {user}
         ),
         user_feeds AS (
            SELECT DISTINCT feed_id FROM user_subscriptions
         ),
         latest_runs AS (
            SELECT r.id, r.feed_id, r.status, r.http_status, r.new_count, r.updated_count,
                   r.dropped_count, r.commit_generation,
                   ROW_NUMBER() OVER (
                       PARTITION BY r.feed_id ORDER BY r.queued_at DESC, r.id DESC
                   ) AS row_number
            FROM feed_refresh_runs r
            JOIN user_feeds uf ON uf.feed_id = r.feed_id
         )
         SELECT s.id AS subscription_id, s.feed_id AS feed_id, s.created_at AS created_at,
                s.title_override AS title_override, f.title AS feed_title,
                f.site_url AS site_url, f.normalized_url AS normalized_url,
                (SELECT COUNT(*)
                 FROM entries e
                 LEFT JOIN entry_states es ON es.user_id = s.user_id AND es.entry_id = e.id
                                          AND es.feed_id = e.feed_id
                                          AND es.feed_sequence = e.feed_sequence
                 WHERE e.feed_id = s.feed_id AND e.feed_sequence > s.start_sequence
                   AND (es.read_override = FALSE
                        OR (es.read_override IS NULL
                            AND e.feed_sequence > s.read_through_sequence))) AS unread_count,
                r.id AS refresh_id, r.status AS refresh_status,
                r.http_status AS refresh_http_status, r.new_count AS refresh_new_count,
                r.updated_count AS refresh_updated_count,
                r.dropped_count AS refresh_dropped_count,
                r.commit_generation AS refresh_generation
         FROM user_subscriptions s
         JOIN feeds f ON f.id = s.feed_id
         LEFT JOIN latest_runs r ON r.feed_id = s.feed_id AND r.row_number = 1"
    )
}

fn decode_subscription_projection(
    row: QueryResult,
) -> Result<SubscriptionProjection, RepositoryError> {
    let normalized_url: String = projection_required(&row, "normalized_url")?;
    let title = effective_subscription_title(
        projection_optional(&row, "title_override")?,
        projection_optional(&row, "feed_title")?,
        &normalized_url,
    )?;
    let unread_count: i64 = projection_required(&row, "unread_count")?;
    if unread_count < 0 {
        return Err(RepositoryError::CorruptData);
    }
    let refresh = projection_optional::<String>(&row, "refresh_id")?
        .map(|_| decode_refresh_row(&row, "refresh_").map_err(|()| RepositoryError::CorruptData))
        .transpose()?;
    Ok(SubscriptionProjection {
        item: SubscriptionListItemDto {
            subscription_id: projection_required(&row, "subscription_id")?,
            feed_id: projection_required(&row, "feed_id")?,
            title,
            site_url: projection_optional(&row, "site_url")?,
            unread_count,
            refresh,
        },
        created_at: projection_required(&row, "created_at")?,
    })
}

fn decode_refresh_row(row: &QueryResult, prefix: &str) -> Result<RefreshDto, ()> {
    let column = |name: &str| format!("{prefix}{name}");
    let status: String = row.try_get("", &column("status")).map_err(|_| ())?;
    Ok(RefreshDto {
        run_id: row.try_get("", &column("id")).map_err(|_| ())?,
        status: status.parse().map_err(|_| ())?,
        http_status: row.try_get("", &column("http_status")).map_err(|_| ())?,
        new_count: row.try_get("", &column("new_count")).map_err(|_| ())?,
        updated_count: row.try_get("", &column("updated_count")).map_err(|_| ())?,
        dropped_count: row.try_get("", &column("dropped_count")).map_err(|_| ())?,
        generation: row.try_get("", &column("generation")).map_err(|_| ())?,
    })
}

fn effective_subscription_title(
    title_override: Option<String>,
    feed_title: Option<String>,
    normalized_url: &str,
) -> Result<String, RepositoryError> {
    title_override
        .filter(|title| !title.trim().is_empty())
        .or(feed_title.filter(|title| !title.trim().is_empty()))
        .or_else(|| {
            url::Url::parse(normalized_url)
                .ok()
                .and_then(|url| url.host_str().map(str::to_owned))
        })
        .filter(|title| !title.is_empty())
        .ok_or(RepositoryError::CorruptData)
}

fn subscription_user_hash(user_id: &str) -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"raindrop-subscription-user\0v1");
    subscription_hash_frame(&mut hasher, user_id.as_bytes());
    subscription_hash_frame(&mut hasher, SUBSCRIPTION_CURSOR_ORDER.as_bytes());
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(hasher.finalize().as_bytes())
}

fn subscription_hash_frame(hasher: &mut blake3::Hasher, value: &[u8]) {
    let length = u32::try_from(value.len()).expect("validated user identifier fits in u32");
    hasher.update(&length.to_be_bytes());
    hasher.update(value);
}

fn encode_subscription_cursor(cursor: &SubscriptionCursorV1) -> Result<String, RepositoryError> {
    let json = serde_json::to_vec(cursor).map_err(|_| RepositoryError::CorruptData)?;
    Ok(base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(json))
}

fn decode_subscription_cursor(encoded: &str) -> Result<SubscriptionCursorV1, RepositoryError> {
    if encoded.is_empty()
        || encoded.len() > MAX_SUBSCRIPTION_CURSOR_BYTES
        || !encoded
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
    {
        return Err(RepositoryError::InvalidCursor);
    }
    let json = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(encoded)
        .map_err(|_| RepositoryError::InvalidCursor)?;
    if json.len() > 768 {
        return Err(RepositoryError::InvalidCursor);
    }
    let cursor: SubscriptionCursorV1 =
        serde_json::from_slice(&json).map_err(|_| RepositoryError::InvalidCursor)?;
    let decoded_user_hash = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(&cursor.user_hash)
        .map_err(|_| RepositoryError::InvalidCursor)?;
    if cursor.version != SUBSCRIPTION_CURSOR_VERSION
        || cursor.order != SUBSCRIPTION_CURSOR_ORDER
        || cursor.user_hash.len() != 43
        || decoded_user_hash.len() != 32
        || base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(&decoded_user_hash)
            != cursor.user_hash
        || validate_uuid(&cursor.subscription_id).is_err()
        || micros_to_timestamp(cursor.created_at_us).is_none()
        || serde_json::to_vec(&cursor).map_err(|_| RepositoryError::InvalidCursor)? != json
        || base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(&json) != encoded
    {
        return Err(RepositoryError::InvalidCursor);
    }
    Ok(cursor)
}

fn timestamp_to_micros(timestamp: OffsetDateTime) -> Result<i64, RepositoryError> {
    i64::try_from(timestamp.unix_timestamp_nanos() / 1_000)
        .map_err(|_| RepositoryError::CorruptData)
}

fn micros_to_timestamp(micros: i64) -> Option<OffsetDateTime> {
    OffsetDateTime::from_unix_timestamp_nanos(i128::from(micros) * 1_000).ok()
}

fn projection_required<T>(row: &QueryResult, column: &str) -> Result<T, RepositoryError>
where
    T: sea_orm::TryGetable,
{
    row.try_get("", column)
        .map_err(|_| RepositoryError::CorruptData)
}

fn projection_optional<T>(row: &QueryResult, column: &str) -> Result<Option<T>, RepositoryError>
where
    T: sea_orm::TryGetable,
{
    row.try_get("", column)
        .map_err(|_| RepositoryError::CorruptData)
}

struct SubscriptionSql {
    backend: DbBackend,
    values: Vec<Value>,
}

impl SubscriptionSql {
    fn new(backend: DbBackend) -> Self {
        Self {
            backend,
            values: Vec::new(),
        }
    }

    fn bind(&mut self, value: impl Into<Value>) -> String {
        self.values.push(value.into());
        if self.backend == DatabaseBackend::Postgres {
            format!("${}", self.values.len())
        } else {
            "?".to_owned()
        }
    }

    fn finish(self, sql: String) -> Statement {
        Statement::from_sql_and_values(self.backend, sql, self.values)
    }
}

fn subscription_explain_statement(statement: Statement) -> Statement {
    let prefix = match statement.db_backend {
        DatabaseBackend::Sqlite => "EXPLAIN QUERY PLAN ",
        DatabaseBackend::Postgres => "EXPLAIN (FORMAT TEXT) ",
        DatabaseBackend::MySql => "EXPLAIN ",
    };
    Statement::from_sql_and_values(
        statement.db_backend,
        format!("{prefix}{}", statement.sql),
        statement.values.map(|values| values.0).unwrap_or_default(),
    )
}

fn subscription_explain_line(
    backend: DbBackend,
    row: &QueryResult,
) -> Result<String, RepositoryError> {
    match backend {
        DatabaseBackend::Sqlite => projection_required(row, "detail"),
        DatabaseBackend::Postgres => projection_required(row, "QUERY PLAN"),
        DatabaseBackend::MySql => {
            let table: Option<String> = projection_optional(row, "table")?;
            let key: Option<String> = projection_optional(row, "key")?;
            let access: Option<String> = projection_optional(row, "type")?;
            let extra: Option<String> = projection_optional(row, "Extra")?;
            Ok(format!(
                "table={} key={} type={} extra={}",
                table.as_deref().unwrap_or(""),
                key.as_deref().unwrap_or(""),
                access.as_deref().unwrap_or(""),
                extra.as_deref().unwrap_or("")
            ))
        }
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
