use std::fmt;

use base64::Engine;
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, ConnectionTrait, DatabaseBackend, DbBackend,
    DbErr, EntityTrait, IntoActiveModel, QueryFilter, QueryResult, Statement, TransactionTrait,
    Value,
};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use uuid::Uuid;

use super::lifecycle::is_unique_violation;
use super::query::{RepositoryError, validate_uuid};
use super::repository::{
    find_active_run, find_run_by_idempotency, idempotent_result, lock_feed_for_queue,
    queue_run_statement, try_lock_feed_for_queue,
};
use super::{
    FeedRepository, ListSubscriptionsQuery, NormalizedFeedUrl, PatchValue, QueueRefreshRequest,
    QueueSubscriptionRefresh, RefreshClaim, RefreshDto, RefreshRepositoryError, RefreshTrigger,
    SubscribeOutcome, SubscriptionListItemDto, SubscriptionPage, UpdateSubscription,
};
use crate::db::entities::{category, subscription};

const SUBSCRIPTION_CURSOR_VERSION: u8 = 1;
const SUBSCRIPTION_CURSOR_ORDER: &str = "CREATED_DESC_ID_DESC";
const MAX_SUBSCRIPTION_CURSOR_BYTES: usize = 1_024;
const MAX_SUBSCRIPTION_LIMIT: u16 = 100;
const INITIAL_VISIBLE_ENTRY_COUNT: i64 = 100;
const MAX_SUBSCRIPTIONS_PER_USER: i64 = 1_000;
const MAX_ACTIVE_USER_REFRESH_RUNS: i64 = 20;

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SubscriptionCursorV1 {
    version: u8,
    user_hash: String,
    order: String,
    created_at_us: i64,
    subscription_id: String,
}

pub(super) struct RefreshContext {
    pub fetch_url: String,
    pub consecutive_failures: i64,
}

#[derive(thiserror::Error)]
pub(super) enum SubscriptionRepositoryError {
    #[error("subscription repository database operation failed")]
    Database(#[source] DbErr),
    #[error("subscription user is not authorized")]
    UserNotFound,
    #[error("subscription repository data is corrupt")]
    CorruptData,
    #[error("refresh run does not belong to the claimed feed")]
    RunMismatch,
}

impl fmt::Debug for SubscriptionRepositoryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Database(_) => "SubscriptionRepositoryError::Database([REDACTED])",
            Self::UserNotFound => "SubscriptionRepositoryError::UserNotFound",
            Self::CorruptData => "SubscriptionRepositoryError::CorruptData",
            Self::RunMismatch => "SubscriptionRepositoryError::RunMismatch",
        })
    }
}

impl From<DbErr> for SubscriptionRepositoryError {
    fn from(error: DbErr) -> Self {
        Self::Database(error)
    }
}

impl FeedRepository {
    pub async fn subscribe(
        &self,
        user_id: &str,
        source_url: &str,
        normalized: &NormalizedFeedUrl,
    ) -> Result<SubscribeOutcome, RefreshRepositoryError> {
        validate_uuid(user_id).map_err(|()| RefreshRepositoryError::InvalidRequest)?;
        if source_url.is_empty() || source_url.len() > 4_096 {
            return Err(RefreshRepositoryError::InvalidRequest);
        }
        self.subscribe_command(user_id, source_url, normalized, None)
            .await
    }

    #[cfg(debug_assertions)]
    #[doc(hidden)]
    pub async fn subscribe_after_feed_scan(
        &self,
        user_id: &str,
        source_url: &str,
        normalized: &NormalizedFeedUrl,
        scanned: std::sync::Arc<tokio::sync::Notify>,
        release: std::sync::Arc<tokio::sync::Notify>,
    ) -> Result<SubscribeOutcome, RefreshRepositoryError> {
        validate_uuid(user_id).map_err(|()| RefreshRepositoryError::InvalidRequest)?;
        if source_url.is_empty() || source_url.len() > 4_096 {
            return Err(RefreshRepositoryError::InvalidRequest);
        }
        self.subscribe_command(user_id, source_url, normalized, Some((scanned, release)))
            .await
    }

    async fn subscribe_command(
        &self,
        user_id: &str,
        source_url: &str,
        normalized: &NormalizedFeedUrl,
        mut scan_hook: Option<(
            std::sync::Arc<tokio::sync::Notify>,
            std::sync::Arc<tokio::sync::Notify>,
        )>,
    ) -> Result<SubscribeOutcome, RefreshRepositoryError> {
        for attempt in 0..3 {
            let candidate = find_feed_by_hash(
                self.connection(),
                self.connection().get_database_backend(),
                normalized.url_hash(),
            )
            .await
            .map_err(map_subscription_command_error)?;
            if let Some((scanned, release)) = scan_hook.take() {
                scanned.notify_one();
                release.notified().await;
            }
            match self
                .subscribe_command_once(user_id, source_url, normalized, candidate)
                .await
            {
                Ok(Some(outcome)) => return Ok(outcome),
                Ok(None) => {}
                Err(RefreshRepositoryError::Database(error))
                    if attempt < 2 && is_unique_violation(&error) => {}
                Err(error) => return Err(error),
            }
        }
        Err(RefreshRepositoryError::CorruptData)
    }

    async fn subscribe_command_once(
        &self,
        user_id: &str,
        source_url: &str,
        normalized: &NormalizedFeedUrl,
        candidate: Option<(String, String)>,
    ) -> Result<Option<SubscribeOutcome>, RefreshRepositoryError> {
        let backend = self.connection().get_database_backend();
        let transaction = self.connection().begin().await?;
        let result = async {
            lock_active_user(&transaction, backend, user_id).await?;
            let (feed_id, feed_is_new) = match candidate {
                Some((feed_id, stored_url)) => {
                    if stored_url != normalized.complete() {
                        return Err(RefreshRepositoryError::IdentityHashCollision);
                    }
                    (feed_id, false)
                }
                None => {
                    let feed_id = Uuid::new_v4().to_string();
                    transaction
                        .execute(insert_feed_statement(
                            backend, &feed_id, source_url, normalized,
                        ))
                        .await?;
                    (feed_id, true)
                }
            };
            let Some(feed) = try_lock_feed_for_queue(&transaction, backend, &feed_id).await? else {
                if feed_is_new {
                    return Err(RefreshRepositoryError::CorruptData);
                }
                return Ok(None);
            };
            if feed.normalized_url != normalized.complete() {
                return Err(RefreshRepositoryError::CorruptData);
            }
            if let Some(subscription_id) =
                find_subscription(&transaction, backend, user_id, &feed_id)
                    .await
                    .map_err(map_subscription_command_error)?
            {
                let subscription =
                    subscription_for_user_from(&transaction, backend, user_id, &subscription_id)
                        .await
                        .map_err(map_projection_command_error)?
                        .ok_or(RefreshRepositoryError::CorruptData)?;
                return Ok(Some(SubscribeOutcome {
                    created: false,
                    subscription,
                }));
            }
            if feed.is_disabled {
                return Err(RefreshRepositoryError::FeedDisabled);
            }
            if count_user_subscriptions(&transaction, backend, user_id).await?
                >= MAX_SUBSCRIPTIONS_PER_USER
            {
                return Err(RefreshRepositoryError::SubscriptionLimit);
            }

            let cleared = transaction
                .execute(clear_orphaned_statement(backend, &feed_id))
                .await?;
            if cleared.rows_affected() != 1 {
                return Err(RefreshRepositoryError::CorruptData);
            }
            let subscription_id = Uuid::new_v4().to_string();
            transaction
                .execute(insert_subscription_statement(
                    backend,
                    &subscription_id,
                    user_id,
                    &feed_id,
                    feed.entry_sequence_head,
                ))
                .await?;

            let database_now = database_now_in(&transaction, backend).await?;
            let needs_refresh =
                feed_is_new || feed.last_success_at.is_none() || feed.next_fetch_at <= database_now;
            if needs_refresh {
                let idempotency_key = format!("subscribe:{subscription_id}");
                let request = QueueRefreshRequest {
                    feed_id: feed_id.clone(),
                    requested_by_user_id: Some(user_id.to_owned()),
                    trigger: RefreshTrigger::Subscribe,
                    idempotency_key,
                };
                if let Some(existing) = find_run_by_idempotency(
                    &transaction,
                    backend,
                    &feed_id,
                    &request.idempotency_key,
                )
                .await?
                {
                    let _ = idempotent_result(existing, &request)?;
                } else if find_active_run(&transaction, backend, &feed_id)
                    .await?
                    .is_none()
                {
                    if count_active_user_refresh_runs(&transaction, backend, user_id).await?
                        >= MAX_ACTIVE_USER_REFRESH_RUNS
                    {
                        return Err(RefreshRepositoryError::ActiveRefreshLimit);
                    }
                    transaction
                        .execute(queue_run_statement(
                            backend,
                            &Uuid::new_v4().to_string(),
                            &request,
                        ))
                        .await?;
                }
            }
            let subscription =
                subscription_for_user_from(&transaction, backend, user_id, &subscription_id)
                    .await
                    .map_err(map_projection_command_error)?
                    .ok_or(RefreshRepositoryError::CorruptData)?;
            Ok(Some(SubscribeOutcome {
                created: true,
                subscription,
            }))
        }
        .await;
        match result {
            Ok(Some(record)) => {
                transaction.commit().await?;
                Ok(Some(record))
            }
            Ok(None) => {
                transaction.rollback().await?;
                Ok(None)
            }
            Err(error) => {
                transaction.rollback().await?;
                Err(error)
            }
        }
    }

    pub async fn queue_subscription_refresh(
        &self,
        user_id: &str,
        subscription_id: &str,
        request: QueueSubscriptionRefresh,
    ) -> Result<RefreshDto, RefreshRepositoryError> {
        validate_uuid(user_id).map_err(|()| RefreshRepositoryError::InvalidRequest)?;
        validate_uuid(subscription_id).map_err(|()| RefreshRepositoryError::InvalidRequest)?;
        validate_uuid(&request.request_id).map_err(|()| RefreshRepositoryError::InvalidRequest)?;
        let Some((_, feed_id)) = self
            .find_owned_subscription(user_id, subscription_id)
            .await
            .map_err(map_subscription_command_error)?
        else {
            return Err(RefreshRepositoryError::InvalidRequest);
        };
        let backend = self.connection().get_database_backend();
        let transaction = self.connection().begin().await?;
        let result = async {
            lock_active_user(&transaction, backend, user_id).await?;
            let feed = lock_feed_for_queue(&transaction, backend, &feed_id).await?;
            if !subscription_matches(&transaction, backend, user_id, subscription_id, &feed_id)
                .await?
            {
                return Err(RefreshRepositoryError::InvalidRequest);
            }
            let idempotency_key = manual_idempotency_key(user_id, &request.request_id);
            let queue_request = QueueRefreshRequest {
                feed_id: feed_id.clone(),
                requested_by_user_id: Some(user_id.to_owned()),
                trigger: RefreshTrigger::Manual,
                idempotency_key,
            };
            if let Some(existing) = find_run_by_idempotency(
                &transaction,
                backend,
                &feed_id,
                &queue_request.idempotency_key,
            )
            .await?
            {
                let existing = idempotent_result(existing, &queue_request)?;
                return load_refresh_dto_from(&transaction, backend, &existing.id).await;
            }
            if let Some(active) = find_active_run(&transaction, backend, &feed_id).await? {
                return Err(RefreshRepositoryError::RefreshInProgress {
                    operation_id: active.id,
                });
            }
            if feed.is_disabled {
                return Err(RefreshRepositoryError::FeedDisabled);
            }
            if feed.orphaned_at.is_some() {
                return Err(RefreshRepositoryError::CorruptData);
            }
            let retry_at = manual_retry_at(feed.last_attempt_at, feed.retry_after_at)?;
            let now = database_now_in(&transaction, backend).await?;
            if let Some(retry_at) = retry_at
                && now < retry_at
            {
                return Err(RefreshRepositoryError::RefreshCooldown {
                    retry_at,
                    retry_after_seconds: retry_after_seconds(now, retry_at),
                });
            }
            if count_active_user_refresh_runs(&transaction, backend, user_id).await?
                >= MAX_ACTIVE_USER_REFRESH_RUNS
            {
                return Err(RefreshRepositoryError::ActiveRefreshLimit);
            }
            let run_id = Uuid::new_v4().to_string();
            transaction
                .execute(queue_run_statement(backend, &run_id, &queue_request))
                .await?;
            load_refresh_dto_from(&transaction, backend, &run_id).await
        }
        .await;
        match result {
            Ok(refresh) => {
                transaction.commit().await?;
                Ok(refresh)
            }
            Err(error) => {
                transaction.rollback().await?;
                Err(error)
            }
        }
    }

    pub async fn unsubscribe(
        &self,
        user_id: &str,
        subscription_id: &str,
    ) -> Result<bool, RefreshRepositoryError> {
        validate_uuid(user_id).map_err(|()| RefreshRepositoryError::InvalidRequest)?;
        validate_uuid(subscription_id).map_err(|()| RefreshRepositoryError::InvalidRequest)?;
        let Some((_, feed_id)) = self
            .find_owned_subscription(user_id, subscription_id)
            .await
            .map_err(map_subscription_command_error)?
        else {
            return Ok(false);
        };
        let backend = self.connection().get_database_backend();
        let transaction = self.connection().begin().await?;
        let result = async {
            lock_active_user(&transaction, backend, user_id).await?;
            let _feed = lock_feed_for_queue(&transaction, backend, &feed_id).await?;
            if !subscription_matches(&transaction, backend, user_id, subscription_id, &feed_id)
                .await?
            {
                return Ok(false);
            }
            let deleted = transaction
                .execute(delete_subscription_statement(
                    backend,
                    user_id,
                    subscription_id,
                    &feed_id,
                ))
                .await?;
            if deleted.rows_affected() != 1 {
                return Err(RefreshRepositoryError::CorruptData);
            }
            if count_feed_subscriptions(&transaction, backend, &feed_id).await? == 0 {
                let orphaned = transaction
                    .execute(mark_feed_orphaned_statement(backend, &feed_id))
                    .await?;
                if orphaned.rows_affected() != 1 {
                    return Err(RefreshRepositoryError::CorruptData);
                }
            }
            Ok(true)
        }
        .await;
        match result {
            Ok(deleted) => {
                transaction.commit().await?;
                Ok(deleted)
            }
            Err(error) => {
                transaction.rollback().await?;
                Err(error)
            }
        }
    }

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
        subscription_for_user_from(self.connection(), backend, user_id, subscription_id).await
    }

    pub async fn update_subscription_for_user(
        &self,
        user_id: &str,
        subscription_id: &str,
        input: UpdateSubscription,
    ) -> Result<Option<SubscriptionListItemDto>, RefreshRepositoryError> {
        let backend = self.connection().get_database_backend();
        let transaction = self.connection().begin().await?;
        let result = async {
            lock_active_user(&transaction, backend, user_id).await?;
            let stored = subscription::Entity::find_by_id(subscription_id)
                .filter(subscription::Column::UserId.eq(user_id))
                .one(&transaction)
                .await?;
            let Some(stored) = stored else {
                return Ok(None);
            };
            if let PatchValue::Value(category_id) = &input.category_id
                && category::Entity::find_by_id(category_id)
                    .filter(category::Column::UserId.eq(user_id))
                    .one(&transaction)
                    .await?
                    .is_none()
            {
                return Ok(None);
            }
            let mut active = stored.into_active_model();
            match input.category_id {
                PatchValue::Missing => {}
                PatchValue::Null => active.category_id = Set(None),
                PatchValue::Value(category_id) => active.category_id = Set(Some(category_id)),
            }
            match input.title_override {
                PatchValue::Missing => {}
                PatchValue::Null => active.title_override = Set(None),
                PatchValue::Value(title_override) => {
                    active.title_override = Set(Some(title_override));
                }
            }
            if let Some(position) = input.position {
                active.position = Set(position);
            }
            active.updated_at = Set(database_now_in(&transaction, backend).await?);
            active.update(&transaction).await?;
            let projection =
                subscription_for_user_from(&transaction, backend, user_id, subscription_id)
                    .await
                    .map_err(map_projection_command_error)?
                    .ok_or(RefreshRepositoryError::CorruptData)?;
            Ok(Some(projection))
        }
        .await;
        match result {
            Ok(projection) => {
                transaction.commit().await?;
                Ok(projection)
            }
            Err(error) => {
                transaction.rollback().await?;
                Err(error)
            }
        }
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

    #[doc(hidden)]
    pub async fn explain_subscription_detail_for_user(
        &self,
        user_id: &str,
        subscription_id: &str,
    ) -> Result<Vec<String>, RepositoryError> {
        validate_uuid(user_id).map_err(|()| RepositoryError::InvalidUserId)?;
        let backend = self.connection().get_database_backend();
        self.connection()
            .query_all(subscription_explain_statement(
                subscription_detail_statement(backend, user_id, subscription_id),
            ))
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

    pub(super) async fn load_refresh_context(
        &self,
        claim: &RefreshClaim,
    ) -> Result<RefreshContext, SubscriptionRepositoryError> {
        let backend = self.connection().get_database_backend();
        let run_feed_id: String = self
            .connection()
            .query_one(Statement::from_sql_and_values(
                backend,
                match backend {
                    DatabaseBackend::Postgres => {
                        "SELECT feed_id FROM feed_refresh_runs WHERE id = $1"
                    }
                    DatabaseBackend::Sqlite | DatabaseBackend::MySql => {
                        "SELECT feed_id FROM feed_refresh_runs WHERE id = ?"
                    }
                },
                [claim.run_id.as_str().into()],
            ))
            .await?
            .ok_or(SubscriptionRepositoryError::RunMismatch)
            .and_then(|row| required(&row, "feed_id"))?;
        if run_feed_id != claim.feed_id {
            return Err(SubscriptionRepositoryError::RunMismatch);
        }
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
                [claim.feed_id.as_str().into()],
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
        load_refresh_dto_from(self.connection(), backend, run_id)
            .await
            .map_err(|error| match error {
                RefreshRepositoryError::Database(error) => {
                    SubscriptionRepositoryError::Database(error)
                }
                _ => SubscriptionRepositoryError::CorruptData,
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
    let mut selected = format!(
        "SELECT s.id, s.user_id, s.feed_id, s.category_id, s.title_override, s.position,
                s.start_sequence, s.read_through_sequence, s.created_at
         FROM subscriptions s
         WHERE s.user_id = {user}"
    );
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
        selected.push_str(&format!(
            " AND (s.created_at < {created_before}
                   OR (s.created_at = {created_tie} AND s.id < {id_before}))"
        ));
    }
    let bound_limit = sql.bind(i64::from(limit) + 1);
    selected.push_str(&format!(
        " ORDER BY s.created_at DESC, s.id DESC LIMIT {bound_limit}"
    ));
    let mut text = subscription_projection_sql(&selected);
    text.push_str(" ORDER BY s.created_at DESC, s.id DESC");
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
    let selected = format!(
        "SELECT s.id, s.user_id, s.feed_id, s.category_id, s.title_override, s.position,
                s.start_sequence, s.read_through_sequence, s.created_at
         FROM subscriptions s
         WHERE s.user_id = {user} AND s.id = {subscription}
         LIMIT 1"
    );
    sql.finish(subscription_projection_sql(&selected))
}

async fn subscription_for_user_from<C>(
    connection: &C,
    backend: DbBackend,
    user_id: &str,
    subscription_id: &str,
) -> Result<Option<SubscriptionListItemDto>, RepositoryError>
where
    C: ConnectionTrait,
{
    connection
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

fn subscription_projection_sql(selected_subscriptions: &str) -> String {
    format!(
        "WITH selected_subscriptions AS (
            {selected_subscriptions}
         ),
         user_feeds AS (
            SELECT DISTINCT feed_id FROM selected_subscriptions
         ),
         latest_runs AS (
            SELECT r.id, r.feed_id, r.status, r.http_status, r.new_count, r.updated_count,
                   r.dropped_count, r.commit_generation, r.error_code, r.retry_at,
                   r.queued_at, r.started_at, r.completed_at,
                   ROW_NUMBER() OVER (
                       PARTITION BY r.feed_id ORDER BY r.queued_at DESC, r.id DESC
                   ) AS refresh_rank
            FROM feed_refresh_runs r
            JOIN user_feeds uf ON uf.feed_id = r.feed_id
         )
         SELECT s.id AS subscription_id, s.feed_id AS feed_id, s.created_at AS created_at,
                s.category_id AS category_id, s.title_override AS title_override,
                s.position AS position, f.title AS feed_title,
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
                r.commit_generation AS refresh_generation,
                r.error_code AS refresh_error_code, r.retry_at AS refresh_retry_at,
                r.queued_at AS refresh_queued_at, r.started_at AS refresh_started_at,
                r.completed_at AS refresh_completed_at
         FROM selected_subscriptions s
         JOIN feeds f ON f.id = s.feed_id
         LEFT JOIN latest_runs r ON r.feed_id = s.feed_id AND r.refresh_rank = 1"
    )
}

fn decode_subscription_projection(
    row: QueryResult,
) -> Result<SubscriptionProjection, RepositoryError> {
    let normalized_url: String = projection_required(&row, "normalized_url")?;
    let title_override = projection_optional(&row, "title_override")?;
    let title = effective_subscription_title(
        title_override.clone(),
        projection_optional(&row, "feed_title")?,
        &normalized_url,
    )?;
    let category_id = projection_optional::<String>(&row, "category_id")?;
    if category_id
        .as_deref()
        .is_some_and(|id| validate_uuid(id).is_err())
    {
        return Err(RepositoryError::CorruptData);
    }
    let position: i64 = projection_required(&row, "position")?;
    if position < 0 {
        return Err(RepositoryError::CorruptData);
    }
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
            category_id,
            title_override,
            position,
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
        error_code: row.try_get("", &column("error_code")).map_err(|_| ())?,
        retry_at: row.try_get("", &column("retry_at")).map_err(|_| ())?,
        queued_at: row.try_get("", &column("queued_at")).map_err(|_| ())?,
        started_at: row.try_get("", &column("started_at")).map_err(|_| ())?,
        completed_at: row.try_get("", &column("completed_at")).map_err(|_| ())?,
    })
}

async fn load_refresh_dto_from<C>(
    connection: &C,
    backend: DbBackend,
    run_id: &str,
) -> Result<RefreshDto, RefreshRepositoryError>
where
    C: ConnectionTrait,
{
    let sql = match backend {
        DatabaseBackend::Postgres => {
            "SELECT id, status, http_status, new_count, updated_count, dropped_count,
                    commit_generation AS generation, error_code, retry_at, queued_at,
                    started_at, completed_at FROM feed_refresh_runs WHERE id = $1"
        }
        DatabaseBackend::Sqlite | DatabaseBackend::MySql => {
            "SELECT id, status, http_status, new_count, updated_count, dropped_count,
                    commit_generation AS generation, error_code, retry_at, queued_at,
                    started_at, completed_at FROM feed_refresh_runs WHERE id = ?"
        }
    };
    let row = connection
        .query_one(Statement::from_sql_and_values(
            backend,
            sql,
            [run_id.into()],
        ))
        .await?
        .ok_or(RefreshRepositoryError::CorruptData)?;
    decode_refresh_row(&row, "").map_err(|()| RefreshRepositoryError::CorruptData)
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

fn stable_framed_blake3(parts: &[&[u8]]) -> blake3::Hash {
    let mut hasher = blake3::Hasher::new();
    for part in parts {
        let length = u32::try_from(part.len()).expect("validated idempotency frame fits in u32");
        hasher.update(&length.to_be_bytes());
        hasher.update(part);
    }
    hasher.finalize()
}

fn manual_idempotency_key(user_id: &str, request_id: &str) -> String {
    let digest = stable_framed_blake3(&[
        b"manual-refresh-v1",
        user_id.as_bytes(),
        request_id.as_bytes(),
    ]);
    let key = format!(
        "m1:{}",
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(digest.as_bytes())
    );
    assert_eq!(key.len(), 46, "manual idempotency key length is stable");
    key
}

fn manual_retry_at(
    last_attempt_at: Option<OffsetDateTime>,
    retry_after_at: Option<OffsetDateTime>,
) -> Result<Option<OffsetDateTime>, RefreshRepositoryError> {
    let cooldown_at = match last_attempt_at {
        Some(attempt) => Some(
            attempt
                .checked_add(time::Duration::seconds(30))
                .ok_or(RefreshRepositoryError::CorruptData)?,
        ),
        None => None,
    };
    Ok(match (cooldown_at, retry_after_at) {
        (Some(cooldown), Some(retry_after)) => Some(cooldown.max(retry_after)),
        (Some(cooldown), None) => Some(cooldown),
        (None, retry_after) => retry_after,
    })
}

fn retry_after_seconds(now: OffsetDateTime, retry_at: OffsetDateTime) -> u64 {
    let remaining_ns = (retry_at - now).whole_nanoseconds().max(1);
    let seconds = ((remaining_ns - 1) / 1_000_000_000) + 1;
    u64::try_from(seconds).unwrap_or(u64::MAX).max(1)
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

async fn lock_active_user<C>(
    connection: &C,
    backend: DbBackend,
    user_id: &str,
) -> Result<(), RefreshRepositoryError>
where
    C: ConnectionTrait,
{
    match backend {
        DatabaseBackend::Sqlite => {
            let result = connection
                .execute(Statement::from_sql_and_values(
                    backend,
                    "UPDATE users SET is_disabled = is_disabled WHERE id = ? AND is_disabled = FALSE",
                    [user_id.into()],
                ))
                .await?;
            if result.rows_affected() != 1 {
                return Err(RefreshRepositoryError::InvalidRequest);
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
                .ok_or(RefreshRepositoryError::InvalidRequest)?;
            let disabled: bool = row
                .try_get("", "is_disabled")
                .map_err(|_| RefreshRepositoryError::CorruptData)?;
            if disabled {
                return Err(RefreshRepositoryError::InvalidRequest);
            }
        }
    }
    Ok(())
}

async fn count_user_subscriptions<C>(
    connection: &C,
    backend: DbBackend,
    user_id: &str,
) -> Result<i64, RefreshRepositoryError>
where
    C: ConnectionTrait,
{
    let sql = if backend == DatabaseBackend::Postgres {
        "SELECT COUNT(*) AS count FROM subscriptions WHERE user_id = $1"
    } else {
        "SELECT COUNT(*) AS count FROM subscriptions WHERE user_id = ?"
    };
    let row = connection
        .query_one(Statement::from_sql_and_values(
            backend,
            sql,
            [user_id.into()],
        ))
        .await?
        .ok_or(RefreshRepositoryError::CorruptData)?;
    let count: i64 = row
        .try_get("", "count")
        .map_err(|_| RefreshRepositoryError::CorruptData)?;
    if count < 0 {
        return Err(RefreshRepositoryError::CorruptData);
    }
    Ok(count)
}

async fn subscription_matches<C>(
    connection: &C,
    backend: DbBackend,
    user_id: &str,
    subscription_id: &str,
    feed_id: &str,
) -> Result<bool, RefreshRepositoryError>
where
    C: ConnectionTrait,
{
    let sql = if backend == DatabaseBackend::Postgres {
        "SELECT 1 AS present FROM subscriptions
         WHERE id = $1 AND user_id = $2 AND feed_id = $3"
    } else {
        "SELECT 1 AS present FROM subscriptions
         WHERE id = ? AND user_id = ? AND feed_id = ?"
    };
    Ok(connection
        .query_one(Statement::from_sql_and_values(
            backend,
            sql,
            [subscription_id.into(), user_id.into(), feed_id.into()],
        ))
        .await?
        .is_some())
}

async fn count_active_user_refresh_runs<C>(
    connection: &C,
    backend: DbBackend,
    user_id: &str,
) -> Result<i64, RefreshRepositoryError>
where
    C: ConnectionTrait,
{
    let sql = if backend == DatabaseBackend::Postgres {
        "SELECT COUNT(*) AS count FROM feed_refresh_runs
         WHERE requested_by_user_id = $1 AND status IN ('QUEUED','RUNNING')"
    } else {
        "SELECT COUNT(*) AS count FROM feed_refresh_runs
         WHERE requested_by_user_id = ? AND status IN ('QUEUED','RUNNING')"
    };
    let row = connection
        .query_one(Statement::from_sql_and_values(
            backend,
            sql,
            [user_id.into()],
        ))
        .await?
        .ok_or(RefreshRepositoryError::CorruptData)?;
    let count: i64 = row
        .try_get("", "count")
        .map_err(|_| RefreshRepositoryError::CorruptData)?;
    if count < 0 {
        return Err(RefreshRepositoryError::CorruptData);
    }
    Ok(count)
}

async fn count_feed_subscriptions<C>(
    connection: &C,
    backend: DbBackend,
    feed_id: &str,
) -> Result<i64, RefreshRepositoryError>
where
    C: ConnectionTrait,
{
    let sql = if backend == DatabaseBackend::Postgres {
        "SELECT COUNT(*) AS count FROM subscriptions WHERE feed_id = $1"
    } else {
        "SELECT COUNT(*) AS count FROM subscriptions WHERE feed_id = ?"
    };
    let row = connection
        .query_one(Statement::from_sql_and_values(
            backend,
            sql,
            [feed_id.into()],
        ))
        .await?
        .ok_or(RefreshRepositoryError::CorruptData)?;
    let count: i64 = row
        .try_get("", "count")
        .map_err(|_| RefreshRepositoryError::CorruptData)?;
    if count < 0 {
        return Err(RefreshRepositoryError::CorruptData);
    }
    Ok(count)
}

async fn database_now_in<C>(
    connection: &C,
    backend: DbBackend,
) -> Result<OffsetDateTime, RefreshRepositoryError>
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
        .ok_or(RefreshRepositoryError::CorruptData)?;
    row.try_get("", "database_now")
        .map_err(|_| RefreshRepositoryError::CorruptData)
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

fn delete_subscription_statement(
    backend: DbBackend,
    user_id: &str,
    subscription_id: &str,
    feed_id: &str,
) -> Statement {
    let sql = if backend == DatabaseBackend::Postgres {
        "DELETE FROM subscriptions WHERE id = $1 AND user_id = $2 AND feed_id = $3"
    } else {
        "DELETE FROM subscriptions WHERE id = ? AND user_id = ? AND feed_id = ?"
    };
    Statement::from_sql_and_values(
        backend,
        sql,
        [subscription_id.into(), user_id.into(), feed_id.into()],
    )
}

fn mark_feed_orphaned_statement(backend: DbBackend, feed_id: &str) -> Statement {
    let sql = match backend {
        DatabaseBackend::Sqlite => {
            "UPDATE feeds
             SET orphaned_at = strftime('%Y-%m-%dT%H:%M:%f000Z','now'),
                 updated_at = strftime('%Y-%m-%dT%H:%M:%f000Z','now')
             WHERE id = ?"
        }
        DatabaseBackend::Postgres => {
            "UPDATE feeds SET orphaned_at = clock_timestamp(), updated_at = clock_timestamp()
             WHERE id = $1"
        }
        DatabaseBackend::MySql => {
            "UPDATE feeds SET orphaned_at = UTC_TIMESTAMP(6), updated_at = UTC_TIMESTAMP(6)
             WHERE id = ?"
        }
    };
    Statement::from_sql_and_values(backend, sql, [feed_id.into()])
}

fn insert_subscription_statement(
    backend: DbBackend,
    subscription_id: &str,
    user_id: &str,
    feed_id: &str,
    entry_sequence_head: i64,
) -> Statement {
    let initial_frontier = entry_sequence_head
        .saturating_sub(INITIAL_VISIBLE_ENTRY_COUNT)
        .max(0);
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
            initial_frontier.into(),
            initial_frontier.into(),
        ],
    )
}

fn map_subscription_command_error(error: SubscriptionRepositoryError) -> RefreshRepositoryError {
    match error {
        SubscriptionRepositoryError::Database(error) => RefreshRepositoryError::Database(error),
        SubscriptionRepositoryError::UserNotFound => RefreshRepositoryError::InvalidRequest,
        SubscriptionRepositoryError::CorruptData | SubscriptionRepositoryError::RunMismatch => {
            RefreshRepositoryError::CorruptData
        }
    }
}

fn map_projection_command_error(error: RepositoryError) -> RefreshRepositoryError {
    match error {
        RepositoryError::Database(error) => RefreshRepositoryError::Database(error),
        _ => RefreshRepositoryError::CorruptData,
    }
}

fn required<T>(row: &QueryResult, column: &str) -> Result<T, SubscriptionRepositoryError>
where
    T: sea_orm::TryGetable,
{
    row.try_get("", column)
        .map_err(|_| SubscriptionRepositoryError::CorruptData)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mysql_projection_avoids_reserved_window_alias() {
        let statement = subscription_list_statement(
            DatabaseBackend::MySql,
            "00000000-0000-4000-8000-000000000001",
            50,
            None,
        );

        assert!(statement.sql.contains("AS refresh_rank"));
        assert!(statement.sql.contains("r.refresh_rank = 1"));
        assert!(!statement.sql.contains("AS row_number"));
    }
}
