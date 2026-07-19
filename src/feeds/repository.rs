use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use sea_orm::{
    ConnectionTrait, DatabaseBackend, DatabaseConnection, DbBackend, QueryResult, Statement,
    TransactionTrait,
};
use time::OffsetDateTime;
use uuid::Uuid;

use super::lifecycle::{record_completed_event, valid_error_code};
use super::{
    ClaimRequest, ExactClaimResult, NormalizedFeedUrl, OpaqueValidator, QueueRefreshRequest,
    RefreshClaim, RefreshCounts, RefreshFailure, RefreshRepositoryError, RefreshRun, RefreshStatus,
    RefreshTrigger, ScheduleOutcome,
};

const MAX_LEASE_TOKEN: i64 = i64::MAX;

#[derive(Debug)]
struct ClaimCandidate {
    run_id: String,
    feed_id: String,
    lease_token: i64,
}

#[derive(Debug)]
struct ExpiredRunCandidate {
    run_id: String,
    feed_id: String,
}

#[derive(Debug)]
struct RecoverableRun {
    requested_by_user_id: Option<String>,
    lease_token: i64,
}

#[derive(Debug)]
struct ScheduledFeedCandidate {
    feed_id: String,
    next_fetch_at: OffsetDateTime,
}

#[derive(Debug)]
struct TerminalUpdate<'a> {
    status: RefreshStatus,
    http_status: Option<i32>,
    counts: RefreshCounts,
    error_code: Option<&'a str>,
    retry_at: Option<OffsetDateTime>,
}

pub(super) struct LockedFeed {
    pub normalized_url: String,
    pub entry_sequence_head: i64,
    pub last_attempt_at: Option<OffsetDateTime>,
    pub last_success_at: Option<OffsetDateTime>,
    pub next_fetch_at: OffsetDateTime,
    pub retry_after_at: Option<OffsetDateTime>,
    pub is_disabled: bool,
    pub orphaned_at: Option<OffsetDateTime>,
    pub lease_owner: Option<String>,
    pub lease_token: i64,
    pub lease_until: Option<OffsetDateTime>,
}

struct NotModifiedUpdate<'a> {
    final_url: &'a str,
    etag: Option<&'a str>,
    last_modified: Option<&'a str>,
    same_validator_scope: bool,
}

#[derive(Clone, Debug)]
pub struct FeedRepository {
    database: DatabaseConnection,
}

impl FeedRepository {
    #[must_use]
    pub fn new(database: DatabaseConnection) -> Self {
        Self { database }
    }

    pub(super) const fn connection(&self) -> &DatabaseConnection {
        &self.database
    }

    /// Low-level exact-key queue seam retained for executor and persistence contracts.
    /// User-facing manual refresh admission must use `queue_subscription_refresh`.
    #[doc(hidden)]
    pub async fn queue_refresh(
        &self,
        request: QueueRefreshRequest,
    ) -> Result<RefreshRun, RefreshRepositoryError> {
        validate_queue_request(&request)?;
        let backend = self.database.get_database_backend();
        let transaction = self.database.begin().await?;
        let result = async {
            let feed = lock_feed_for_queue(&transaction, backend, &request.feed_id).await?;
            if let Some(existing) = find_run_by_idempotency(
                &transaction,
                backend,
                &request.feed_id,
                &request.idempotency_key,
            )
            .await?
            {
                return idempotent_result(existing, &request);
            }
            if let Some(active) = find_active_run(&transaction, backend, &request.feed_id).await? {
                return Err(RefreshRepositoryError::RefreshInProgress {
                    operation_id: active.id,
                });
            }
            if feed.is_disabled {
                return Err(RefreshRepositoryError::FeedDisabled);
            }
            if feed.orphaned_at.is_some() {
                return Err(RefreshRepositoryError::InvalidRequest);
            }
            if request.trigger == RefreshTrigger::Scheduled
                && feed.next_fetch_at > queue_database_now(&transaction, backend).await?
            {
                return Err(RefreshRepositoryError::InvalidRequest);
            }
            if request.requested_by_user_id.is_some() {
                return Err(RefreshRepositoryError::InvalidRequest);
            }

            let run_id = Uuid::new_v4().to_string();
            transaction
                .execute(queue_run_statement(backend, &run_id, &request))
                .await?;
            find_run_by_idempotency(
                &transaction,
                backend,
                &request.feed_id,
                &request.idempotency_key,
            )
            .await?
            .ok_or(RefreshRepositoryError::CorruptData)
        }
        .await;
        match result {
            Ok(run) => {
                transaction.commit().await?;
                Ok(run)
            }
            Err(error) => {
                transaction.rollback().await?;
                Err(error)
            }
        }
    }

    pub async fn claim_due(
        &self,
        request: ClaimRequest,
    ) -> Result<Option<RefreshClaim>, RefreshRepositoryError> {
        validate_owner(&request.owner)?;
        let lease_micros = lease_micros(request.lease_duration)?;
        let backend = self.database.get_database_backend();
        let Some(candidate) = find_claim_candidate(&self.database, backend).await? else {
            return Ok(None);
        };
        validate_claim_token(candidate.lease_token)?;

        let transaction = self.database.begin().await?;
        let locked_token = if backend == DatabaseBackend::MySql {
            lock_feed_for_mysql(&transaction, &candidate.feed_id).await?
        } else {
            candidate.lease_token
        };
        validate_claim_token(locked_token)?;

        let result = transaction
            .execute(claim_statement(
                backend,
                &request.owner,
                lease_micros,
                &candidate.feed_id,
                &candidate.run_id,
            ))
            .await?;
        if result.rows_affected() != 1 {
            transaction.rollback().await?;
            return Ok(None);
        }

        let (lease_token, deadline) =
            read_lease_state(&transaction, backend, &candidate.feed_id).await?;
        validate_active_token(lease_token)?;
        let run_result = transaction
            .execute(start_run_statement(
                backend,
                &candidate.run_id,
                &candidate.feed_id,
                lease_token,
            ))
            .await?;
        if run_result.rows_affected() != 1 {
            transaction.rollback().await?;
            return Err(RefreshRepositoryError::CorruptData);
        }

        transaction.commit().await?;

        Ok(Some(RefreshClaim {
            run_id: candidate.run_id,
            feed_id: candidate.feed_id,
            owner: request.owner,
            lease_token,
            lease_deadline: deadline,
        }))
    }

    pub async fn claim_run(
        &self,
        run_id: &str,
        request: ClaimRequest,
    ) -> Result<ExactClaimResult, RefreshRepositoryError> {
        if run_id.is_empty() || run_id.len() > 36 {
            return Err(RefreshRepositoryError::InvalidRequest);
        }
        validate_owner(&request.owner)?;
        let lease_micros = lease_micros(request.lease_duration)?;
        let backend = self.database.get_database_backend();
        let (status, disabled) = read_exact_run_state(&self.database, backend, run_id)
            .await?
            .ok_or(RefreshRepositoryError::RunNotFound)?;
        if status != RefreshStatus::Queued {
            return Ok(ExactClaimResult::Existing(status));
        }
        if disabled {
            return Ok(ExactClaimResult::FeedDisabled);
        }
        let Some(candidate) = find_exact_claim_candidate(&self.database, backend, run_id).await?
        else {
            return Ok(ExactClaimResult::TemporarilyBlocked);
        };
        validate_claim_token(candidate.lease_token)?;

        let transaction = self.database.begin().await?;
        let locked_token = if backend == DatabaseBackend::MySql {
            lock_feed_for_mysql(&transaction, &candidate.feed_id).await?
        } else {
            candidate.lease_token
        };
        validate_claim_token(locked_token)?;
        let result = transaction
            .execute(claim_statement(
                backend,
                &request.owner,
                lease_micros,
                &candidate.feed_id,
                &candidate.run_id,
            ))
            .await?;
        if result.rows_affected() != 1 {
            transaction.rollback().await?;
            let status = read_run_status_optional(&self.database, backend, run_id)
                .await?
                .ok_or(RefreshRepositoryError::RunNotFound)?;
            return if status == RefreshStatus::Queued {
                Ok(ExactClaimResult::TemporarilyBlocked)
            } else {
                Ok(ExactClaimResult::Existing(status))
            };
        }
        let (lease_token, deadline) =
            read_lease_state(&transaction, backend, &candidate.feed_id).await?;
        validate_active_token(lease_token)?;
        let run_result = transaction
            .execute(start_run_statement(
                backend,
                &candidate.run_id,
                &candidate.feed_id,
                lease_token,
            ))
            .await?;
        if run_result.rows_affected() != 1 {
            transaction.rollback().await?;
            return Err(RefreshRepositoryError::CorruptData);
        }
        transaction.commit().await?;
        Ok(ExactClaimResult::Claimed(RefreshClaim {
            run_id: candidate.run_id,
            feed_id: candidate.feed_id,
            owner: request.owner,
            lease_token,
            lease_deadline: deadline,
        }))
    }

    pub async fn extend_lease(
        &self,
        claim: &RefreshClaim,
        lease_duration: std::time::Duration,
    ) -> Result<RefreshClaim, RefreshRepositoryError> {
        validate_owner(&claim.owner)?;
        validate_active_token(claim.lease_token)?;
        let lease_micros = lease_micros(lease_duration)?;
        let backend = self.database.get_database_backend();
        let transaction = self.database.begin().await?;
        if let Some(current_token) =
            lock_feed_before_deadline_check(&transaction, backend, &claim.feed_id).await?
        {
            validate_active_token(current_token)?;
        }

        let result = transaction
            .execute(extend_statement(backend, claim, lease_micros))
            .await?;
        if result.rows_affected() != 1 {
            transaction.rollback().await?;
            return Err(RefreshRepositoryError::LeaseLost);
        }
        let (_, deadline) = read_lease_state(&transaction, backend, &claim.feed_id).await?;
        transaction.commit().await?;

        Ok(RefreshClaim {
            lease_deadline: deadline,
            ..claim.clone()
        })
    }

    pub async fn recover_expired_runs(
        &self,
        limit: u16,
    ) -> Result<Vec<String>, RefreshRepositoryError> {
        if limit == 0 {
            return Err(RefreshRepositoryError::InvalidRequest);
        }
        let backend = self.database.get_database_backend();
        let candidates = find_expired_run_candidates(&self.database, backend, limit).await?;
        let mut queued = Vec::new();
        for candidate in candidates {
            let transaction = self.database.begin().await?;
            let result = async {
                let feed = lock_feed_for_queue(&transaction, backend, &candidate.feed_id).await?;
                let Some(run) = lock_recoverable_run(
                    &transaction,
                    backend,
                    &candidate.run_id,
                    &candidate.feed_id,
                )
                .await?
                else {
                    return Ok(None);
                };
                let now = queue_database_now(&transaction, backend).await?;
                let stale = feed.lease_token != run.lease_token
                    || feed.lease_owner.is_none()
                    || feed.lease_until.is_none_or(|deadline| deadline <= now);
                if !stale {
                    return Ok(None);
                }
                let terminalized = transaction
                    .execute(recover_run_statement(
                        backend,
                        &candidate.run_id,
                        &candidate.feed_id,
                        run.lease_token,
                    ))
                    .await?;
                if terminalized.rows_affected() != 1 {
                    return Err(RefreshRepositoryError::InvalidTransition);
                }
                if find_active_run(&transaction, backend, &candidate.feed_id)
                    .await?
                    .is_some()
                {
                    return Ok(None);
                }

                let request = QueueRefreshRequest {
                    feed_id: candidate.feed_id.clone(),
                    requested_by_user_id: run.requested_by_user_id,
                    trigger: RefreshTrigger::Retry,
                    idempotency_key: format!("r1:{}", candidate.run_id),
                };
                if let Some(existing) = find_run_by_idempotency(
                    &transaction,
                    backend,
                    &candidate.feed_id,
                    &request.idempotency_key,
                )
                .await?
                {
                    idempotent_result(existing, &request)?;
                    return Ok(None);
                }
                let run_id = Uuid::new_v4().to_string();
                transaction
                    .execute(queue_run_statement(backend, &run_id, &request))
                    .await?;
                Ok(Some(run_id))
            }
            .await;
            match result {
                Ok(run_id) => {
                    transaction.commit().await?;
                    if let Some(run_id) = run_id {
                        queued.push(run_id);
                    }
                }
                Err(error) => {
                    transaction.rollback().await?;
                    return Err(error);
                }
            }
        }
        Ok(queued)
    }

    pub async fn enqueue_due_scheduled(&self, limit: u16) -> Result<usize, RefreshRepositoryError> {
        self.enqueue_due_scheduled_inner(limit, || {}).await
    }

    #[cfg(debug_assertions)]
    #[doc(hidden)]
    pub async fn enqueue_due_scheduled_after_scan(
        &self,
        limit: u16,
        scanned: std::sync::Arc<tokio::sync::Notify>,
    ) -> Result<usize, RefreshRepositoryError> {
        self.enqueue_due_scheduled_inner(limit, move || scanned.notify_one())
            .await
    }

    async fn enqueue_due_scheduled_inner<F>(
        &self,
        limit: u16,
        after_scan: F,
    ) -> Result<usize, RefreshRepositoryError>
    where
        F: FnOnce(),
    {
        if limit == 0 {
            return Err(RefreshRepositoryError::InvalidRequest);
        }
        let backend = self.database.get_database_backend();
        let candidates =
            find_due_scheduled_candidates(&self.database, backend, limit.min(100)).await?;
        after_scan();
        let mut queued = 0;
        for candidate in candidates {
            let transaction = self.database.begin().await?;
            let result = async {
                let feed = lock_feed_for_queue(&transaction, backend, &candidate.feed_id).await?;
                let now = queue_database_now(&transaction, backend).await?;
                if feed.is_disabled
                    || feed.orphaned_at.is_some()
                    || feed.next_fetch_at != candidate.next_fetch_at
                    || feed.next_fetch_at > now
                    || !has_feed_subscription(&transaction, backend, &candidate.feed_id).await?
                    || find_active_run(&transaction, backend, &candidate.feed_id)
                        .await?
                        .is_some()
                {
                    return Ok(false);
                }

                let request = QueueRefreshRequest {
                    feed_id: candidate.feed_id.clone(),
                    requested_by_user_id: None,
                    trigger: RefreshTrigger::Scheduled,
                    idempotency_key: scheduled_idempotency_key(
                        &candidate.feed_id,
                        candidate.next_fetch_at,
                    )?,
                };
                if let Some(existing) = find_run_by_idempotency(
                    &transaction,
                    backend,
                    &candidate.feed_id,
                    &request.idempotency_key,
                )
                .await?
                {
                    idempotent_result(existing, &request)?;
                    return Ok(false);
                }
                let run_id = Uuid::new_v4().to_string();
                transaction
                    .execute(queue_run_statement(backend, &run_id, &request))
                    .await?;
                Ok(true)
            }
            .await;
            match result {
                Ok(inserted) => {
                    transaction.commit().await?;
                    queued += usize::from(inserted);
                }
                Err(error) => {
                    transaction.rollback().await?;
                    return Err(error);
                }
            }
        }
        Ok(queued)
    }

    #[cfg(debug_assertions)]
    pub async fn complete_success(
        &self,
        claim: &RefreshClaim,
        http_status: i32,
        counts: RefreshCounts,
    ) -> Result<(), RefreshRepositoryError> {
        validate_http_status(http_status)?;
        validate_counts(counts)?;
        self.complete_owned(
            claim,
            TerminalUpdate {
                status: RefreshStatus::Success,
                http_status: Some(http_status),
                counts,
                error_code: None,
                retry_at: None,
            },
        )
        .await
    }

    #[cfg(debug_assertions)]
    pub async fn complete_partial(
        &self,
        claim: &RefreshClaim,
        http_status: i32,
        counts: RefreshCounts,
    ) -> Result<(), RefreshRepositoryError> {
        validate_http_status(http_status)?;
        validate_counts(counts)?;
        self.complete_owned(
            claim,
            TerminalUpdate {
                status: RefreshStatus::Partial,
                http_status: Some(http_status),
                counts,
                error_code: None,
                retry_at: None,
            },
        )
        .await
    }

    #[cfg(debug_assertions)]
    pub async fn complete_not_modified(
        &self,
        claim: &RefreshClaim,
    ) -> Result<(), RefreshRepositoryError> {
        self.complete_owned(
            claim,
            TerminalUpdate {
                status: RefreshStatus::NotModified,
                http_status: Some(304),
                counts: RefreshCounts {
                    new_count: 0,
                    updated_count: 0,
                    dropped_count: 0,
                },
                error_code: None,
                retry_at: None,
            },
        )
        .await
    }

    #[cfg(debug_assertions)]
    pub async fn complete_failure(
        &self,
        claim: &RefreshClaim,
        failure: RefreshFailure,
    ) -> Result<(), RefreshRepositoryError> {
        if !valid_error_code(&failure.error_code) {
            return Err(RefreshRepositoryError::InvalidRequest);
        }
        if let Some(http_status) = failure.http_status {
            validate_http_status(http_status)?;
        }
        self.complete_owned(
            claim,
            TerminalUpdate {
                status: RefreshStatus::Error,
                http_status: failure.http_status,
                counts: RefreshCounts {
                    new_count: 0,
                    updated_count: 0,
                    dropped_count: 0,
                },
                error_code: Some(&failure.error_code),
                retry_at: failure.retry_at,
            },
        )
        .await
    }

    pub async fn complete_not_modified_scheduled(
        &self,
        claim: &RefreshClaim,
        final_url: &NormalizedFeedUrl,
        etag: Option<&OpaqueValidator>,
        last_modified: Option<&OpaqueValidator>,
        schedule: ScheduleOutcome,
    ) -> Result<(), RefreshRepositoryError> {
        if schedule.consecutive_failures() != 0 || schedule.retry_after_at().is_some() {
            return Err(RefreshRepositoryError::InvalidRequest);
        }
        self.complete_owned_scheduled(
            claim,
            TerminalUpdate {
                status: RefreshStatus::NotModified,
                http_status: Some(304),
                counts: RefreshCounts {
                    new_count: 0,
                    updated_count: 0,
                    dropped_count: 0,
                },
                error_code: None,
                retry_at: None,
            },
            schedule,
            Some(NotModifiedUpdate {
                final_url: final_url.complete(),
                etag: etag.map(OpaqueValidator::storage_value),
                last_modified: last_modified.map(OpaqueValidator::storage_value),
                same_validator_scope: false,
            }),
        )
        .await
    }

    pub async fn complete_failure_scheduled(
        &self,
        claim: &RefreshClaim,
        failure: RefreshFailure,
        schedule: ScheduleOutcome,
    ) -> Result<(), RefreshRepositoryError> {
        if !valid_error_code(&failure.error_code)
            || schedule.consecutive_failures() <= 0
            || failure.retry_at != Some(schedule.next_at())
        {
            return Err(RefreshRepositoryError::InvalidRequest);
        }
        if let Some(http_status) = failure.http_status {
            validate_http_status(http_status)?;
        }
        self.complete_owned_scheduled(
            claim,
            TerminalUpdate {
                status: RefreshStatus::Error,
                http_status: failure.http_status,
                counts: RefreshCounts {
                    new_count: 0,
                    updated_count: 0,
                    dropped_count: 0,
                },
                error_code: Some(&failure.error_code),
                retry_at: failure.retry_at,
            },
            schedule,
            None,
        )
        .await
    }

    pub async fn cancel_running(&self, claim: &RefreshClaim) -> Result<(), RefreshRepositoryError> {
        self.complete_owned(
            claim,
            TerminalUpdate {
                status: RefreshStatus::Cancelled,
                http_status: None,
                counts: RefreshCounts {
                    new_count: 0,
                    updated_count: 0,
                    dropped_count: 0,
                },
                error_code: None,
                retry_at: None,
            },
        )
        .await
    }

    pub async fn cancel_queued(&self, run_id: &str) -> Result<(), RefreshRepositoryError> {
        if run_id.is_empty() || run_id.len() > 36 {
            return Err(RefreshRepositoryError::InvalidRequest);
        }
        let backend = self.database.get_database_backend();
        let result = self
            .database
            .execute(cancel_queued_statement(backend, run_id))
            .await?;
        if result.rows_affected() == 1 {
            Ok(())
        } else {
            let _ = read_run_status(&self.database, backend, run_id).await?;
            Err(RefreshRepositoryError::InvalidTransition)
        }
    }

    pub async fn record_lease_lost(&self, run_id: &str) -> Result<(), RefreshRepositoryError> {
        if run_id.is_empty() || run_id.len() > 36 {
            return Err(RefreshRepositoryError::InvalidRequest);
        }
        let backend = self.database.get_database_backend();
        if backend == DatabaseBackend::MySql {
            let feed_id = read_run_feed_id(&self.database, backend, run_id).await?;
            let transaction = self.database.begin().await?;
            let current_token = lock_feed_for_mysql(&transaction, &feed_id).await?;
            validate_active_token(current_token)?;
            let result = transaction
                .execute(record_lease_lost_statement(backend, run_id))
                .await?;
            if result.rows_affected() != 1 {
                let _ = read_run_status(&transaction, backend, run_id).await?;
                transaction.rollback().await?;
                return Err(RefreshRepositoryError::InvalidTransition);
            }
            transaction.commit().await?;
            return Ok(());
        }

        let result = self
            .database
            .execute(record_lease_lost_statement(backend, run_id))
            .await?;
        if result.rows_affected() == 1 {
            Ok(())
        } else {
            let _ = read_run_status(&self.database, backend, run_id).await?;
            Err(RefreshRepositoryError::InvalidTransition)
        }
    }

    async fn complete_owned(
        &self,
        claim: &RefreshClaim,
        terminal: TerminalUpdate<'_>,
    ) -> Result<(), RefreshRepositoryError> {
        validate_owner(&claim.owner)?;
        validate_active_token(claim.lease_token)?;
        let records_lifecycle_event = matches!(
            terminal.status,
            RefreshStatus::Success
                | RefreshStatus::Partial
                | RefreshStatus::NotModified
                | RefreshStatus::Error
        );
        let backend = self.database.get_database_backend();
        let transaction = self.database.begin().await?;
        if let Some(current_token) =
            lock_feed_before_deadline_check(&transaction, backend, &claim.feed_id).await?
        {
            validate_active_token(current_token)?;
        }

        let authorization = transaction
            .execute(terminal_authorization_statement(backend, claim))
            .await?;
        if authorization.rows_affected() != 1 {
            let status = read_run_status(&transaction, backend, &claim.run_id).await?;
            transaction.rollback().await?;
            return if status == RefreshStatus::Running {
                Err(RefreshRepositoryError::LeaseLost)
            } else {
                Err(RefreshRepositoryError::InvalidTransition)
            };
        }

        let run_result = transaction
            .execute(terminal_run_statement(backend, claim, &terminal))
            .await?;
        if run_result.rows_affected() != 1 {
            transaction.rollback().await?;
            return Err(RefreshRepositoryError::InvalidTransition);
        }
        if records_lifecycle_event {
            record_completed_event(
                &transaction,
                backend,
                claim,
                terminal.status,
                terminal.http_status,
                terminal.counts,
                terminal.error_code,
            )
            .await?;
        }
        transaction.commit().await?;
        Ok(())
    }

    async fn complete_owned_scheduled(
        &self,
        claim: &RefreshClaim,
        terminal: TerminalUpdate<'_>,
        schedule: ScheduleOutcome,
        mut not_modified: Option<NotModifiedUpdate<'_>>,
    ) -> Result<(), RefreshRepositoryError> {
        validate_owner(&claim.owner)?;
        validate_active_token(claim.lease_token)?;
        let backend = self.database.get_database_backend();
        let transaction = self.database.begin().await?;
        if let Some(current_token) =
            lock_feed_before_deadline_check(&transaction, backend, &claim.feed_id).await?
        {
            validate_active_token(current_token)?;
        }
        if let Some(update) = not_modified.as_mut() {
            let stored_scope = read_validator_scope(&transaction, backend, &claim.feed_id).await?;
            update.same_validator_scope = stored_scope.as_deref() == Some(update.final_url);
        }
        let authorization = transaction
            .execute(scheduled_terminal_authorization_statement(
                backend,
                claim,
                &terminal,
                schedule,
                not_modified.as_ref(),
            ))
            .await?;
        if authorization.rows_affected() != 1 {
            let status = read_run_status(&transaction, backend, &claim.run_id).await?;
            transaction.rollback().await?;
            return if status == RefreshStatus::Running {
                Err(RefreshRepositoryError::LeaseLost)
            } else {
                Err(RefreshRepositoryError::InvalidTransition)
            };
        }
        let run_result = transaction
            .execute(terminal_run_statement(backend, claim, &terminal))
            .await?;
        if run_result.rows_affected() != 1 {
            transaction.rollback().await?;
            return Err(RefreshRepositoryError::InvalidTransition);
        }
        record_completed_event(
            &transaction,
            backend,
            claim,
            terminal.status,
            terminal.http_status,
            terminal.counts,
            terminal.error_code,
        )
        .await?;
        transaction.commit().await?;
        Ok(())
    }
}

pub(super) async fn lock_feed_for_queue<C>(
    connection: &C,
    backend: DbBackend,
    feed_id: &str,
) -> Result<LockedFeed, RefreshRepositoryError>
where
    C: ConnectionTrait,
{
    try_lock_feed_for_queue(connection, backend, feed_id)
        .await?
        .ok_or(RefreshRepositoryError::CorruptData)
}

pub(super) async fn try_lock_feed_for_queue<C>(
    connection: &C,
    backend: DbBackend,
    feed_id: &str,
) -> Result<Option<LockedFeed>, RefreshRepositoryError>
where
    C: ConnectionTrait,
{
    if backend == DatabaseBackend::Sqlite {
        let locked = connection
            .execute(Statement::from_sql_and_values(
                backend,
                "UPDATE feeds SET lease_token = lease_token WHERE id = ?",
                [feed_id.into()],
            ))
            .await?;
        if locked.rows_affected() == 0 {
            return Ok(None);
        }
    }
    let sql = match backend {
        DatabaseBackend::Sqlite => {
            "SELECT normalized_url, entry_sequence_head, last_attempt_at, last_success_at,
                    next_fetch_at, retry_after_at, is_disabled, orphaned_at,
                    lease_owner, lease_token, lease_until
             FROM feeds WHERE id = ?"
        }
        DatabaseBackend::Postgres => {
            "SELECT normalized_url, entry_sequence_head, last_attempt_at, last_success_at,
                    next_fetch_at, retry_after_at, is_disabled, orphaned_at,
                    lease_owner, lease_token, lease_until
             FROM feeds WHERE id = $1 FOR UPDATE"
        }
        DatabaseBackend::MySql => {
            "SELECT normalized_url, entry_sequence_head, last_attempt_at, last_success_at,
                    next_fetch_at, retry_after_at, is_disabled, orphaned_at,
                    lease_owner, lease_token, lease_until
             FROM feeds WHERE id = ? FOR UPDATE"
        }
    };
    connection
        .query_one(Statement::from_sql_and_values(
            backend,
            sql,
            [feed_id.into()],
        ))
        .await?
        .map(decode_locked_feed)
        .transpose()
}

fn decode_locked_feed(row: QueryResult) -> Result<LockedFeed, RefreshRepositoryError> {
    let entry_sequence_head = row
        .try_get("", "entry_sequence_head")
        .map_err(|_| RefreshRepositoryError::CorruptData)?;
    if entry_sequence_head < 0 {
        return Err(RefreshRepositoryError::CorruptData);
    }
    Ok(LockedFeed {
        normalized_url: row
            .try_get("", "normalized_url")
            .map_err(|_| RefreshRepositoryError::CorruptData)?,
        entry_sequence_head,
        last_attempt_at: row
            .try_get("", "last_attempt_at")
            .map_err(|_| RefreshRepositoryError::CorruptData)?,
        last_success_at: row
            .try_get("", "last_success_at")
            .map_err(|_| RefreshRepositoryError::CorruptData)?,
        next_fetch_at: row
            .try_get("", "next_fetch_at")
            .map_err(|_| RefreshRepositoryError::CorruptData)?,
        retry_after_at: row
            .try_get("", "retry_after_at")
            .map_err(|_| RefreshRepositoryError::CorruptData)?,
        is_disabled: row
            .try_get("", "is_disabled")
            .map_err(|_| RefreshRepositoryError::CorruptData)?,
        orphaned_at: row
            .try_get("", "orphaned_at")
            .map_err(|_| RefreshRepositoryError::CorruptData)?,
        lease_owner: row
            .try_get("", "lease_owner")
            .map_err(|_| RefreshRepositoryError::CorruptData)?,
        lease_token: row
            .try_get("", "lease_token")
            .map_err(|_| RefreshRepositoryError::CorruptData)?,
        lease_until: row
            .try_get("", "lease_until")
            .map_err(|_| RefreshRepositoryError::CorruptData)?,
    })
}

async fn queue_database_now<C>(
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

fn validate_queue_request(request: &QueueRefreshRequest) -> Result<(), RefreshRepositoryError> {
    if request.feed_id.is_empty()
        || request.feed_id.len() > 36
        || request.idempotency_key.is_empty()
        || request.idempotency_key.len() > 64
        || request
            .requested_by_user_id
            .as_ref()
            .is_some_and(|id| id.is_empty() || id.len() > 36)
    {
        return Err(RefreshRepositoryError::InvalidRequest);
    }
    Ok(())
}

pub(super) fn idempotent_result(
    existing: RefreshRun,
    request: &QueueRefreshRequest,
) -> Result<RefreshRun, RefreshRepositoryError> {
    if existing.trigger == request.trigger
        && existing.requested_by_user_id == request.requested_by_user_id
    {
        Ok(existing)
    } else {
        Err(RefreshRepositoryError::IdempotencyConflict)
    }
}

fn validate_owner(owner: &str) -> Result<(), RefreshRepositoryError> {
    if owner.is_empty() || owner.len() > 128 {
        return Err(RefreshRepositoryError::InvalidRequest);
    }
    Ok(())
}

pub(super) fn queue_run_statement(
    backend: DbBackend,
    run_id: &str,
    request: &QueueRefreshRequest,
) -> Statement {
    let sql = match backend {
        DatabaseBackend::Sqlite => {
            "INSERT INTO feed_refresh_runs (
                 id, feed_id, requested_by_user_id, trigger_kind, status, idempotency_key, queued_at
             ) VALUES (?, ?, ?, ?, 'QUEUED', ?, strftime('%Y-%m-%dT%H:%M:%f000Z', 'now'))"
        }
        DatabaseBackend::Postgres => {
            "INSERT INTO feed_refresh_runs (
                 id, feed_id, requested_by_user_id, trigger_kind, status, idempotency_key, queued_at
             ) VALUES ($1, $2, $3, $4, 'QUEUED', $5, clock_timestamp())"
        }
        DatabaseBackend::MySql => {
            "INSERT INTO feed_refresh_runs (
                 id, feed_id, requested_by_user_id, trigger_kind, status, idempotency_key, queued_at
             ) VALUES (?, ?, ?, ?, 'QUEUED', ?, UTC_TIMESTAMP(6))"
        }
    };
    Statement::from_sql_and_values(
        backend,
        sql,
        [
            run_id.into(),
            request.feed_id.as_str().into(),
            request.requested_by_user_id.as_deref().into(),
            request.trigger.as_str().into(),
            request.idempotency_key.as_str().into(),
        ],
    )
}

pub(super) async fn find_run_by_idempotency<C>(
    connection: &C,
    backend: DbBackend,
    feed_id: &str,
    idempotency_key: &str,
) -> Result<Option<RefreshRun>, RefreshRepositoryError>
where
    C: ConnectionTrait,
{
    let sql = match backend {
        DatabaseBackend::Postgres => {
            "SELECT id, feed_id, requested_by_user_id, trigger_kind, status, idempotency_key,
                    lease_token, queued_at
             FROM feed_refresh_runs
             WHERE feed_id = $1 AND idempotency_key = $2"
        }
        DatabaseBackend::Sqlite | DatabaseBackend::MySql => {
            "SELECT id, feed_id, requested_by_user_id, trigger_kind, status, idempotency_key,
                    lease_token, queued_at
             FROM feed_refresh_runs
             WHERE feed_id = ? AND idempotency_key = ?"
        }
    };
    connection
        .query_one(Statement::from_sql_and_values(
            backend,
            sql,
            [feed_id.into(), idempotency_key.into()],
        ))
        .await?
        .map(decode_refresh_run)
        .transpose()
}

pub(super) async fn find_active_run<C>(
    connection: &C,
    backend: DbBackend,
    feed_id: &str,
) -> Result<Option<RefreshRun>, RefreshRepositoryError>
where
    C: ConnectionTrait,
{
    let sql = match backend {
        DatabaseBackend::Postgres => {
            "SELECT id, feed_id, requested_by_user_id, trigger_kind, status, idempotency_key,
                    lease_token, queued_at
             FROM feed_refresh_runs
             WHERE feed_id = $1 AND status IN ('QUEUED','RUNNING')
             ORDER BY queued_at, id LIMIT 1"
        }
        DatabaseBackend::Sqlite | DatabaseBackend::MySql => {
            "SELECT id, feed_id, requested_by_user_id, trigger_kind, status, idempotency_key,
                    lease_token, queued_at
             FROM feed_refresh_runs
             WHERE feed_id = ? AND status IN ('QUEUED','RUNNING')
             ORDER BY queued_at, id LIMIT 1"
        }
    };
    connection
        .query_one(Statement::from_sql_and_values(
            backend,
            sql,
            [feed_id.into()],
        ))
        .await?
        .map(decode_refresh_run)
        .transpose()
}

fn decode_refresh_run(row: QueryResult) -> Result<RefreshRun, RefreshRepositoryError> {
    let trigger: String = row
        .try_get("", "trigger_kind")
        .map_err(|_| RefreshRepositoryError::CorruptData)?;
    let status: String = row
        .try_get("", "status")
        .map_err(|_| RefreshRepositoryError::CorruptData)?;
    let lease_token: Option<i64> = row
        .try_get("", "lease_token")
        .map_err(|_| RefreshRepositoryError::CorruptData)?;
    if lease_token.is_some_and(|token| token < 0) {
        return Err(RefreshRepositoryError::CorruptData);
    }
    Ok(RefreshRun {
        id: row
            .try_get("", "id")
            .map_err(|_| RefreshRepositoryError::CorruptData)?,
        feed_id: row
            .try_get("", "feed_id")
            .map_err(|_| RefreshRepositoryError::CorruptData)?,
        requested_by_user_id: row
            .try_get("", "requested_by_user_id")
            .map_err(|_| RefreshRepositoryError::CorruptData)?,
        trigger: trigger
            .parse()
            .map_err(|_| RefreshRepositoryError::CorruptData)?,
        status: status
            .parse()
            .map_err(|_| RefreshRepositoryError::CorruptData)?,
        idempotency_key: row
            .try_get("", "idempotency_key")
            .map_err(|_| RefreshRepositoryError::CorruptData)?,
        lease_token,
        queued_at: row
            .try_get("", "queued_at")
            .map_err(|_| RefreshRepositoryError::CorruptData)?,
    })
}

fn lease_micros(duration: std::time::Duration) -> Result<i64, RefreshRepositoryError> {
    if duration.is_zero() {
        return Err(RefreshRepositoryError::InvalidRequest);
    }
    i64::try_from(duration.as_micros()).map_err(|_| RefreshRepositoryError::InvalidRequest)
}

fn validate_claim_token(token: i64) -> Result<(), RefreshRepositoryError> {
    if token < 0 {
        Err(RefreshRepositoryError::CorruptData)
    } else if token == MAX_LEASE_TOKEN {
        Err(RefreshRepositoryError::TokenExhausted)
    } else {
        Ok(())
    }
}

fn validate_active_token(token: i64) -> Result<(), RefreshRepositoryError> {
    if token < 0 {
        Err(RefreshRepositoryError::CorruptData)
    } else {
        Ok(())
    }
}

fn validate_http_status(http_status: i32) -> Result<(), RefreshRepositoryError> {
    if (100..=599).contains(&http_status) {
        Ok(())
    } else {
        Err(RefreshRepositoryError::InvalidRequest)
    }
}

#[cfg(debug_assertions)]
fn validate_counts(counts: RefreshCounts) -> Result<(), RefreshRepositoryError> {
    if counts.new_count < 0 || counts.updated_count < 0 || counts.dropped_count < 0 {
        Err(RefreshRepositoryError::InvalidRequest)
    } else {
        Ok(())
    }
}

async fn find_claim_candidate<C>(
    connection: &C,
    backend: DbBackend,
) -> Result<Option<ClaimCandidate>, RefreshRepositoryError>
where
    C: ConnectionTrait,
{
    let sql = match backend {
        DatabaseBackend::Sqlite => {
            "SELECT r.id AS run_id, r.feed_id AS feed_id, f.lease_token AS lease_token
             FROM feed_refresh_runs r
             JOIN feeds f ON f.id = r.feed_id
             WHERE r.status = 'QUEUED'
               AND f.is_disabled = FALSE
               AND (
                    (r.trigger_kind = 'SCHEDULED' AND julianday(f.next_fetch_at) <= julianday('now'))
                    OR r.trigger_kind IN ('MANUAL', 'SUBSCRIBE', 'IMPORT', 'RETRY')
               )
               AND (
                    (f.lease_owner IS NULL AND f.lease_until IS NULL)
                    OR (f.lease_until IS NOT NULL AND julianday(f.lease_until) <= julianday('now'))
               )
             ORDER BY r.queued_at, r.id
             LIMIT 1"
        }
        DatabaseBackend::Postgres => {
            "SELECT r.id AS run_id, r.feed_id AS feed_id, f.lease_token AS lease_token
             FROM feed_refresh_runs r
             JOIN feeds f ON f.id = r.feed_id
             WHERE r.status = 'QUEUED'
               AND f.is_disabled = FALSE
               AND (
                    (r.trigger_kind = 'SCHEDULED' AND f.next_fetch_at <= clock_timestamp())
                    OR r.trigger_kind IN ('MANUAL', 'SUBSCRIBE', 'IMPORT', 'RETRY')
               )
               AND (
                    (f.lease_owner IS NULL AND f.lease_until IS NULL)
                    OR (f.lease_until IS NOT NULL AND f.lease_until <= clock_timestamp())
               )
             ORDER BY r.queued_at, r.id
             LIMIT 1"
        }
        DatabaseBackend::MySql => {
            "SELECT r.id AS run_id, r.feed_id AS feed_id, f.lease_token AS lease_token
             FROM feed_refresh_runs r
             JOIN feeds f ON f.id = r.feed_id
             WHERE r.status = 'QUEUED'
               AND f.is_disabled = FALSE
               AND (
                    (r.trigger_kind = 'SCHEDULED' AND f.next_fetch_at <= UTC_TIMESTAMP(6))
                    OR r.trigger_kind IN ('MANUAL', 'SUBSCRIBE', 'IMPORT', 'RETRY')
               )
               AND (
                    (f.lease_owner IS NULL AND f.lease_until IS NULL)
                    OR (f.lease_until IS NOT NULL AND f.lease_until <= UTC_TIMESTAMP(6))
               )
             ORDER BY r.queued_at, r.id
             LIMIT 1"
        }
    };
    let row = connection
        .query_one(Statement::from_string(backend, sql.to_owned()))
        .await?;
    row.map(decode_claim_candidate).transpose()
}

async fn find_expired_run_candidates<C>(
    connection: &C,
    backend: DbBackend,
    limit: u16,
) -> Result<Vec<ExpiredRunCandidate>, RefreshRepositoryError>
where
    C: ConnectionTrait,
{
    let sql = match backend {
        DatabaseBackend::Sqlite => {
            "SELECT r.id AS run_id, r.feed_id AS feed_id
             FROM feed_refresh_runs r
             JOIN feeds f ON f.id = r.feed_id
             WHERE r.status = 'RUNNING'
               AND (
                    r.lease_token IS NULL
                    OR f.lease_token <> r.lease_token
                    OR f.lease_owner IS NULL
                    OR f.lease_until IS NULL
                    OR julianday(f.lease_until) <= julianday('now')
               )
             ORDER BY r.queued_at, r.id
             LIMIT ?"
        }
        DatabaseBackend::Postgres => {
            "SELECT r.id AS run_id, r.feed_id AS feed_id
             FROM feed_refresh_runs r
             JOIN feeds f ON f.id = r.feed_id
             WHERE r.status = 'RUNNING'
               AND (
                    r.lease_token IS NULL
                    OR f.lease_token <> r.lease_token
                    OR f.lease_owner IS NULL
                    OR f.lease_until IS NULL
                    OR f.lease_until <= clock_timestamp()
               )
             ORDER BY r.queued_at, r.id
             LIMIT $1"
        }
        DatabaseBackend::MySql => {
            "SELECT r.id AS run_id, r.feed_id AS feed_id
             FROM feed_refresh_runs r
             JOIN feeds f ON f.id = r.feed_id
             WHERE r.status = 'RUNNING'
               AND (
                    r.lease_token IS NULL
                    OR f.lease_token <> r.lease_token
                    OR f.lease_owner IS NULL
                    OR f.lease_until IS NULL
                    OR f.lease_until <= UTC_TIMESTAMP(6)
               )
             ORDER BY r.queued_at, r.id
             LIMIT ?"
        }
    };
    connection
        .query_all(Statement::from_sql_and_values(
            backend,
            sql,
            [i64::from(limit).into()],
        ))
        .await?
        .into_iter()
        .map(|row| {
            Ok(ExpiredRunCandidate {
                run_id: row
                    .try_get("", "run_id")
                    .map_err(|_| RefreshRepositoryError::CorruptData)?,
                feed_id: row
                    .try_get("", "feed_id")
                    .map_err(|_| RefreshRepositoryError::CorruptData)?,
            })
        })
        .collect()
}

async fn find_due_scheduled_candidates<C>(
    connection: &C,
    backend: DbBackend,
    limit: u16,
) -> Result<Vec<ScheduledFeedCandidate>, RefreshRepositoryError>
where
    C: ConnectionTrait,
{
    let sql = match backend {
        DatabaseBackend::Sqlite => {
            "SELECT f.id AS feed_id, f.next_fetch_at AS next_fetch_at
             FROM feeds f
             WHERE f.is_disabled = FALSE
               AND f.orphaned_at IS NULL
               AND julianday(f.next_fetch_at) <= julianday('now')
               AND EXISTS (SELECT 1 FROM subscriptions s WHERE s.feed_id = f.id)
             ORDER BY f.next_fetch_at, f.id
             LIMIT ?"
        }
        DatabaseBackend::Postgres => {
            "SELECT f.id AS feed_id, f.next_fetch_at AS next_fetch_at
             FROM feeds f
             WHERE f.is_disabled = FALSE
               AND f.orphaned_at IS NULL
               AND f.next_fetch_at <= clock_timestamp()
               AND EXISTS (SELECT 1 FROM subscriptions s WHERE s.feed_id = f.id)
             ORDER BY f.next_fetch_at, f.id
             LIMIT $1"
        }
        DatabaseBackend::MySql => {
            "SELECT f.id AS feed_id, f.next_fetch_at AS next_fetch_at
             FROM feeds f
             WHERE f.is_disabled = FALSE
               AND f.orphaned_at IS NULL
               AND f.next_fetch_at <= UTC_TIMESTAMP(6)
               AND EXISTS (SELECT 1 FROM subscriptions s WHERE s.feed_id = f.id)
             ORDER BY f.next_fetch_at, f.id
             LIMIT ?"
        }
    };
    connection
        .query_all(Statement::from_sql_and_values(
            backend,
            sql,
            [i64::from(limit).into()],
        ))
        .await?
        .into_iter()
        .map(|row| {
            Ok(ScheduledFeedCandidate {
                feed_id: row
                    .try_get("", "feed_id")
                    .map_err(|_| RefreshRepositoryError::CorruptData)?,
                next_fetch_at: row
                    .try_get("", "next_fetch_at")
                    .map_err(|_| RefreshRepositoryError::CorruptData)?,
            })
        })
        .collect()
}

async fn has_feed_subscription<C>(
    connection: &C,
    backend: DbBackend,
    feed_id: &str,
) -> Result<bool, RefreshRepositoryError>
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

fn scheduled_idempotency_key(
    feed_id: &str,
    next_fetch_at: OffsetDateTime,
) -> Result<String, RefreshRepositoryError> {
    let next_fetch_at_us = i64::try_from(next_fetch_at.unix_timestamp_nanos() / 1_000)
        .map_err(|_| RefreshRepositoryError::InvalidTime)?;
    let timestamp = next_fetch_at_us.to_be_bytes();
    let mut hasher = blake3::Hasher::new();
    for part in [feed_id.as_bytes(), timestamp.as_slice()] {
        let length = u32::try_from(part.len()).map_err(|_| RefreshRepositoryError::CorruptData)?;
        hasher.update(&length.to_be_bytes());
        hasher.update(part);
    }
    let key = format!(
        "s1:{}",
        URL_SAFE_NO_PAD.encode(hasher.finalize().as_bytes())
    );
    debug_assert_eq!(key.len(), 46);
    Ok(key)
}

async fn lock_recoverable_run<C>(
    connection: &C,
    backend: DbBackend,
    run_id: &str,
    feed_id: &str,
) -> Result<Option<RecoverableRun>, RefreshRepositoryError>
where
    C: ConnectionTrait,
{
    let sql = match backend {
        DatabaseBackend::Sqlite => {
            "SELECT requested_by_user_id, lease_token
             FROM feed_refresh_runs
             WHERE id = ? AND feed_id = ? AND status = 'RUNNING'"
        }
        DatabaseBackend::Postgres => {
            "SELECT requested_by_user_id, lease_token
             FROM feed_refresh_runs
             WHERE id = $1 AND feed_id = $2 AND status = 'RUNNING'
             FOR UPDATE"
        }
        DatabaseBackend::MySql => {
            "SELECT requested_by_user_id, lease_token
             FROM feed_refresh_runs
             WHERE id = ? AND feed_id = ? AND status = 'RUNNING'
             FOR UPDATE"
        }
    };
    connection
        .query_one(Statement::from_sql_and_values(
            backend,
            sql,
            [run_id.into(), feed_id.into()],
        ))
        .await?
        .map(|row| {
            let lease_token = row
                .try_get("", "lease_token")
                .map_err(|_| RefreshRepositoryError::CorruptData)?;
            validate_active_token(lease_token)?;
            Ok(RecoverableRun {
                requested_by_user_id: row
                    .try_get("", "requested_by_user_id")
                    .map_err(|_| RefreshRepositoryError::CorruptData)?,
                lease_token,
            })
        })
        .transpose()
}

async fn find_exact_claim_candidate<C>(
    connection: &C,
    backend: DbBackend,
    run_id: &str,
) -> Result<Option<ClaimCandidate>, RefreshRepositoryError>
where
    C: ConnectionTrait,
{
    let sql = match backend {
        DatabaseBackend::Sqlite => {
            "SELECT r.id AS run_id, r.feed_id AS feed_id, f.lease_token AS lease_token
             FROM feed_refresh_runs r JOIN feeds f ON f.id = r.feed_id
             WHERE r.id = ? AND r.status = 'QUEUED' AND f.is_disabled = FALSE
               AND ((r.trigger_kind = 'SCHEDULED' AND julianday(f.next_fetch_at) <= julianday('now'))
                    OR r.trigger_kind IN ('MANUAL','SUBSCRIBE','IMPORT','RETRY'))
               AND ((f.lease_owner IS NULL AND f.lease_until IS NULL)
                    OR (f.lease_until IS NOT NULL AND julianday(f.lease_until) <= julianday('now')))"
        }
        DatabaseBackend::Postgres => {
            "SELECT r.id AS run_id, r.feed_id AS feed_id, f.lease_token AS lease_token
             FROM feed_refresh_runs r JOIN feeds f ON f.id = r.feed_id
             WHERE r.id = $1 AND r.status = 'QUEUED' AND f.is_disabled = FALSE
               AND ((r.trigger_kind = 'SCHEDULED' AND f.next_fetch_at <= clock_timestamp())
                    OR r.trigger_kind IN ('MANUAL','SUBSCRIBE','IMPORT','RETRY'))
               AND ((f.lease_owner IS NULL AND f.lease_until IS NULL)
                    OR (f.lease_until IS NOT NULL AND f.lease_until <= clock_timestamp()))"
        }
        DatabaseBackend::MySql => {
            "SELECT r.id AS run_id, r.feed_id AS feed_id, f.lease_token AS lease_token
             FROM feed_refresh_runs r JOIN feeds f ON f.id = r.feed_id
             WHERE r.id = ? AND r.status = 'QUEUED' AND f.is_disabled = FALSE
               AND ((r.trigger_kind = 'SCHEDULED' AND f.next_fetch_at <= UTC_TIMESTAMP(6))
                    OR r.trigger_kind IN ('MANUAL','SUBSCRIBE','IMPORT','RETRY'))
               AND ((f.lease_owner IS NULL AND f.lease_until IS NULL)
                    OR (f.lease_until IS NOT NULL AND f.lease_until <= UTC_TIMESTAMP(6)))"
        }
    };
    connection
        .query_one(Statement::from_sql_and_values(
            backend,
            sql,
            [run_id.into()],
        ))
        .await?
        .map(decode_claim_candidate)
        .transpose()
}

fn decode_claim_candidate(row: QueryResult) -> Result<ClaimCandidate, RefreshRepositoryError> {
    Ok(ClaimCandidate {
        run_id: row
            .try_get("", "run_id")
            .map_err(|_| RefreshRepositoryError::CorruptData)?,
        feed_id: row
            .try_get("", "feed_id")
            .map_err(|_| RefreshRepositoryError::CorruptData)?,
        lease_token: row
            .try_get("", "lease_token")
            .map_err(|_| RefreshRepositoryError::CorruptData)?,
    })
}

async fn lock_feed_for_mysql<C>(
    connection: &C,
    feed_id: &str,
) -> Result<i64, RefreshRepositoryError>
where
    C: ConnectionTrait,
{
    let row = connection
        .query_one(Statement::from_sql_and_values(
            DatabaseBackend::MySql,
            "SELECT lease_token FROM feeds WHERE id = ? FOR UPDATE",
            [feed_id.into()],
        ))
        .await?
        .ok_or(RefreshRepositoryError::CorruptData)?;
    row.try_get("", "lease_token")
        .map_err(|_| RefreshRepositoryError::CorruptData)
}

async fn lock_feed_before_deadline_check<C>(
    connection: &C,
    backend: DbBackend,
    feed_id: &str,
) -> Result<Option<i64>, RefreshRepositoryError>
where
    C: ConnectionTrait,
{
    match backend {
        DatabaseBackend::Sqlite => Ok(None),
        DatabaseBackend::Postgres => {
            let row = connection
                .query_one(Statement::from_sql_and_values(
                    DatabaseBackend::Postgres,
                    "SELECT lease_token FROM feeds WHERE id = $1 FOR UPDATE",
                    [feed_id.into()],
                ))
                .await?
                .ok_or(RefreshRepositoryError::CorruptData)?;
            row.try_get("", "lease_token")
                .map(Some)
                .map_err(|_| RefreshRepositoryError::CorruptData)
        }
        DatabaseBackend::MySql => lock_feed_for_mysql(connection, feed_id).await.map(Some),
    }
}

async fn read_run_feed_id<C>(
    connection: &C,
    backend: DbBackend,
    run_id: &str,
) -> Result<String, RefreshRepositoryError>
where
    C: ConnectionTrait,
{
    let sql = match backend {
        DatabaseBackend::Postgres => "SELECT feed_id FROM feed_refresh_runs WHERE id = $1",
        DatabaseBackend::Sqlite | DatabaseBackend::MySql => {
            "SELECT feed_id FROM feed_refresh_runs WHERE id = ?"
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
    row.try_get("", "feed_id")
        .map_err(|_| RefreshRepositoryError::CorruptData)
}

fn claim_statement(
    backend: DbBackend,
    owner: &str,
    lease_micros: i64,
    feed_id: &str,
    run_id: &str,
) -> Statement {
    let sql = match backend {
        DatabaseBackend::Sqlite => {
            "UPDATE feeds
             SET lease_owner = ?,
                 lease_token = lease_token + 1,
                 lease_until = strftime('%Y-%m-%dT%H:%M:%f000Z', 'now', printf('+%.6f seconds', CAST(? AS REAL) / 1000000.0)),
                 updated_at = strftime('%Y-%m-%dT%H:%M:%f000Z', 'now')
             WHERE id = ?
               AND is_disabled = FALSE
               AND lease_token >= 0
               AND lease_token < ?
               AND (
                    (lease_owner IS NULL AND lease_until IS NULL)
                    OR (lease_until IS NOT NULL AND julianday(lease_until) <= julianday('now'))
               )
               AND EXISTS (
                    SELECT 1 FROM feed_refresh_runs r
                    WHERE r.id = ? AND r.feed_id = feeds.id AND r.status = 'QUEUED'
                      AND (
                           (r.trigger_kind = 'SCHEDULED' AND julianday(feeds.next_fetch_at) <= julianday('now'))
                           OR r.trigger_kind IN ('MANUAL', 'SUBSCRIBE', 'IMPORT', 'RETRY')
                      )
               )"
        }
        DatabaseBackend::Postgres => {
            "UPDATE feeds
             SET lease_owner = $1,
                 lease_token = lease_token + 1,
                 lease_until = clock_timestamp() + ($2 * INTERVAL '1 microsecond'),
                 updated_at = clock_timestamp()
             WHERE id = $3
               AND is_disabled = FALSE
               AND lease_token >= 0
               AND lease_token < $4
               AND (
                    (lease_owner IS NULL AND lease_until IS NULL)
                    OR (lease_until IS NOT NULL AND lease_until <= clock_timestamp())
               )
               AND EXISTS (
                    SELECT 1 FROM feed_refresh_runs r
                    WHERE r.id = $5 AND r.feed_id = feeds.id AND r.status = 'QUEUED'
                      AND (
                           (r.trigger_kind = 'SCHEDULED' AND feeds.next_fetch_at <= clock_timestamp())
                           OR r.trigger_kind IN ('MANUAL', 'SUBSCRIBE', 'IMPORT', 'RETRY')
                      )
               )"
        }
        DatabaseBackend::MySql => {
            "UPDATE feeds
             SET lease_owner = ?,
                 lease_token = lease_token + 1,
                 lease_until = TIMESTAMPADD(MICROSECOND, ?, UTC_TIMESTAMP(6)),
                 updated_at = UTC_TIMESTAMP(6)
             WHERE id = ?
               AND is_disabled = FALSE
               AND lease_token >= 0
               AND lease_token < ?
               AND (
                    (lease_owner IS NULL AND lease_until IS NULL)
                    OR (lease_until IS NOT NULL AND lease_until <= UTC_TIMESTAMP(6))
               )
               AND EXISTS (
                    SELECT 1 FROM feed_refresh_runs r
                    WHERE r.id = ? AND r.feed_id = feeds.id AND r.status = 'QUEUED'
                      AND (
                           (r.trigger_kind = 'SCHEDULED' AND feeds.next_fetch_at <= UTC_TIMESTAMP(6))
                           OR r.trigger_kind IN ('MANUAL', 'SUBSCRIBE', 'IMPORT', 'RETRY')
                      )
               )"
        }
    };
    Statement::from_sql_and_values(
        backend,
        sql,
        [
            owner.into(),
            lease_micros.into(),
            feed_id.into(),
            MAX_LEASE_TOKEN.into(),
            run_id.into(),
        ],
    )
}

fn start_run_statement(
    backend: DbBackend,
    run_id: &str,
    feed_id: &str,
    lease_token: i64,
) -> Statement {
    let sql = match backend {
        DatabaseBackend::Sqlite => {
            "UPDATE feed_refresh_runs
             SET status = 'RUNNING', lease_token = ?,
                 started_at = strftime('%Y-%m-%dT%H:%M:%f000Z', 'now')
             WHERE id = ? AND feed_id = ? AND status = 'QUEUED'"
        }
        DatabaseBackend::Postgres => {
            "UPDATE feed_refresh_runs
             SET status = 'RUNNING', lease_token = $1, started_at = clock_timestamp()
             WHERE id = $2 AND feed_id = $3 AND status = 'QUEUED'"
        }
        DatabaseBackend::MySql => {
            "UPDATE feed_refresh_runs
             SET status = 'RUNNING', lease_token = ?, started_at = UTC_TIMESTAMP(6)
             WHERE id = ? AND feed_id = ? AND status = 'QUEUED'"
        }
    };
    Statement::from_sql_and_values(
        backend,
        sql,
        [lease_token.into(), run_id.into(), feed_id.into()],
    )
}

fn extend_statement(backend: DbBackend, claim: &RefreshClaim, lease_micros: i64) -> Statement {
    let sql = match backend {
        DatabaseBackend::Sqlite => {
            "UPDATE feeds
             SET lease_until = strftime('%Y-%m-%dT%H:%M:%f000Z', 'now', printf('+%.6f seconds', CAST(? AS REAL) / 1000000.0)),
                 updated_at = strftime('%Y-%m-%dT%H:%M:%f000Z', 'now')
             WHERE id = ?
               AND lease_owner = ?
               AND lease_token = ?
               AND lease_token >= 0
               AND lease_until IS NOT NULL
               AND julianday(lease_until) > julianday('now')
               AND EXISTS (
                    SELECT 1 FROM feed_refresh_runs r
                    WHERE r.id = ?
                      AND r.feed_id = feeds.id
                      AND r.status = 'RUNNING'
                      AND r.lease_token = feeds.lease_token
               )"
        }
        DatabaseBackend::Postgres => {
            "UPDATE feeds
             SET lease_until = clock_timestamp() + ($1 * INTERVAL '1 microsecond'),
                 updated_at = clock_timestamp()
             WHERE id = $2
               AND lease_owner = $3
               AND lease_token = $4
               AND lease_token >= 0
               AND lease_until IS NOT NULL
               AND lease_until > clock_timestamp()
               AND EXISTS (
                    SELECT 1 FROM feed_refresh_runs r
                    WHERE r.id = $5
                      AND r.feed_id = feeds.id
                      AND r.status = 'RUNNING'
                      AND r.lease_token = feeds.lease_token
               )"
        }
        DatabaseBackend::MySql => {
            "UPDATE feeds
             SET lease_until = TIMESTAMPADD(MICROSECOND, ?, UTC_TIMESTAMP(6)),
                 updated_at = UTC_TIMESTAMP(6)
             WHERE id = ?
               AND lease_owner = ?
               AND lease_token = ?
               AND lease_token >= 0
               AND lease_until IS NOT NULL
               AND lease_until > UTC_TIMESTAMP(6)
               AND EXISTS (
                    SELECT 1 FROM feed_refresh_runs r
                    WHERE r.id = ?
                      AND r.feed_id = feeds.id
                      AND r.status = 'RUNNING'
                      AND r.lease_token = feeds.lease_token
               )"
        }
    };
    Statement::from_sql_and_values(
        backend,
        sql,
        [
            lease_micros.into(),
            claim.feed_id.as_str().into(),
            claim.owner.as_str().into(),
            claim.lease_token.into(),
            claim.run_id.as_str().into(),
        ],
    )
}

fn terminal_authorization_statement(backend: DbBackend, claim: &RefreshClaim) -> Statement {
    let sql = match backend {
        DatabaseBackend::Sqlite => {
            "UPDATE feeds
             SET lease_owner = NULL,
                 lease_until = NULL,
                 updated_at = strftime('%Y-%m-%dT%H:%M:%f000Z', 'now')
             WHERE id = ?
               AND lease_owner = ?
               AND lease_token = ?
               AND lease_token >= 0
               AND lease_until IS NOT NULL
               AND julianday(lease_until) > julianday('now')
               AND EXISTS (
                    SELECT 1 FROM feed_refresh_runs r
                    WHERE r.id = ?
                      AND r.feed_id = feeds.id
                      AND r.status = 'RUNNING'
                      AND r.lease_token = feeds.lease_token
               )"
        }
        DatabaseBackend::Postgres => {
            "UPDATE feeds
             SET lease_owner = NULL,
                 lease_until = NULL,
                 updated_at = clock_timestamp()
             WHERE id = $1
               AND lease_owner = $2
               AND lease_token = $3
               AND lease_token >= 0
               AND lease_until IS NOT NULL
               AND lease_until > clock_timestamp()
               AND EXISTS (
                    SELECT 1 FROM feed_refresh_runs r
                    WHERE r.id = $4
                      AND r.feed_id = feeds.id
                      AND r.status = 'RUNNING'
                      AND r.lease_token = feeds.lease_token
               )"
        }
        DatabaseBackend::MySql => {
            "UPDATE feeds
             SET lease_owner = NULL,
                 lease_until = NULL,
                 updated_at = UTC_TIMESTAMP(6)
             WHERE id = ?
               AND lease_owner = ?
               AND lease_token = ?
               AND lease_token >= 0
               AND lease_until IS NOT NULL
               AND lease_until > UTC_TIMESTAMP(6)
               AND EXISTS (
                    SELECT 1 FROM feed_refresh_runs r
                    WHERE r.id = ?
                      AND r.feed_id = feeds.id
                      AND r.status = 'RUNNING'
                      AND r.lease_token = feeds.lease_token
               )"
        }
    };
    Statement::from_sql_and_values(
        backend,
        sql,
        [
            claim.feed_id.as_str().into(),
            claim.owner.as_str().into(),
            claim.lease_token.into(),
            claim.run_id.as_str().into(),
        ],
    )
}

fn scheduled_terminal_authorization_statement(
    backend: DbBackend,
    claim: &RefreshClaim,
    terminal: &TerminalUpdate<'_>,
    schedule: ScheduleOutcome,
    not_modified: Option<&NotModifiedUpdate<'_>>,
) -> Statement {
    let is_not_modified = terminal.status == RefreshStatus::NotModified;
    let clock = match backend {
        DatabaseBackend::Sqlite => "strftime('%Y-%m-%dT%H:%M:%f000Z','now')",
        DatabaseBackend::Postgres => "clock_timestamp()",
        DatabaseBackend::MySql => "UTC_TIMESTAMP(6)",
    };
    let success_assignment = if is_not_modified {
        format!("last_success_at={clock},")
    } else {
        String::new()
    };
    if let Some(update) = not_modified {
        let placeholders = if backend == DatabaseBackend::Postgres {
            [
                "$1", "$2", "$3", "$4", "$5", "$6", "$7", "$8", "$9", "$10", "$11", "$12", "$13",
                "$14", "$15", "$16",
            ]
        } else {
            [
                "?", "?", "?", "?", "?", "?", "?", "?", "?", "?", "?", "?", "?", "?", "?", "?",
            ]
        };
        let sql = format!(
            "UPDATE feeds SET lease_owner=NULL, lease_until=NULL, last_attempt_at={clock},
                    {success_assignment} next_fetch_at={0}, retry_after_at={1},
                    consecutive_failures={2}, last_error_code={3}, fetch_url={4}, validator_url={5},
                    etag=CASE WHEN {6} THEN COALESCE({7}, etag) ELSE {8} END,
                    last_modified=CASE WHEN {9} THEN COALESCE({10}, last_modified) ELSE {11} END,
                    updated_at={clock}
             WHERE id={12} AND lease_owner={13} AND lease_token={14} AND lease_token >= 0
               AND lease_until IS NOT NULL AND lease_until > {clock}
               AND EXISTS (SELECT 1 FROM feed_refresh_runs r WHERE r.id={15}
                           AND r.feed_id=feeds.id AND r.status='RUNNING'
                           AND r.lease_token=feeds.lease_token)",
            placeholders[0],
            placeholders[1],
            placeholders[2],
            placeholders[3],
            placeholders[4],
            placeholders[5],
            placeholders[6],
            placeholders[7],
            placeholders[8],
            placeholders[9],
            placeholders[10],
            placeholders[11],
            placeholders[12],
            placeholders[13],
            placeholders[14],
            placeholders[15],
        );
        return Statement::from_sql_and_values(
            backend,
            sql,
            [
                schedule.next_at().into(),
                schedule.retry_after_at().into(),
                schedule.consecutive_failures().into(),
                terminal.error_code.into(),
                update.final_url.into(),
                update.final_url.into(),
                update.same_validator_scope.into(),
                update.etag.into(),
                update.etag.into(),
                update.same_validator_scope.into(),
                update.last_modified.into(),
                update.last_modified.into(),
                claim.feed_id.as_str().into(),
                claim.owner.as_str().into(),
                claim.lease_token.into(),
                claim.run_id.as_str().into(),
            ],
        );
    }
    let placeholders = if backend == DatabaseBackend::Postgres {
        ["$1", "$2", "$3", "$4", "$5", "$6", "$7", "$8"]
    } else {
        ["?", "?", "?", "?", "?", "?", "?", "?"]
    };
    let sql = format!(
        "UPDATE feeds SET lease_owner=NULL, lease_until=NULL, last_attempt_at={clock},
                {success_assignment} next_fetch_at={0}, retry_after_at={1},
                consecutive_failures={2}, last_error_code={3}, updated_at={clock}
         WHERE id={4} AND lease_owner={5} AND lease_token={6} AND lease_token >= 0
           AND lease_until IS NOT NULL AND lease_until > {clock}
           AND EXISTS (SELECT 1 FROM feed_refresh_runs r WHERE r.id={7}
                       AND r.feed_id=feeds.id AND r.status='RUNNING'
                       AND r.lease_token=feeds.lease_token)",
        placeholders[0],
        placeholders[1],
        placeholders[2],
        placeholders[3],
        placeholders[4],
        placeholders[5],
        placeholders[6],
        placeholders[7],
    );
    Statement::from_sql_and_values(
        backend,
        sql,
        [
            schedule.next_at().into(),
            schedule.retry_after_at().into(),
            schedule.consecutive_failures().into(),
            terminal.error_code.into(),
            claim.feed_id.as_str().into(),
            claim.owner.as_str().into(),
            claim.lease_token.into(),
            claim.run_id.as_str().into(),
        ],
    )
}

async fn read_validator_scope<C>(
    connection: &C,
    backend: DbBackend,
    feed_id: &str,
) -> Result<Option<String>, RefreshRepositoryError>
where
    C: ConnectionTrait,
{
    let sql = match backend {
        DatabaseBackend::Sqlite => "SELECT validator_url FROM feeds WHERE id = ?",
        DatabaseBackend::Postgres => "SELECT validator_url FROM feeds WHERE id = $1 FOR UPDATE",
        DatabaseBackend::MySql => "SELECT validator_url FROM feeds WHERE id = ? FOR UPDATE",
    };
    let row = connection
        .query_one(Statement::from_sql_and_values(
            backend,
            sql,
            [feed_id.into()],
        ))
        .await?
        .ok_or(RefreshRepositoryError::CorruptData)?;
    row.try_get("", "validator_url")
        .map_err(|_| RefreshRepositoryError::CorruptData)
}

fn cancel_queued_statement(backend: DbBackend, run_id: &str) -> Statement {
    let sql = match backend {
        DatabaseBackend::Sqlite => {
            "UPDATE feed_refresh_runs
             SET status = 'CANCELLED', completed_at = strftime('%Y-%m-%dT%H:%M:%f000Z', 'now')
             WHERE id = ? AND status = 'QUEUED'"
        }
        DatabaseBackend::Postgres => {
            "UPDATE feed_refresh_runs
             SET status = 'CANCELLED', completed_at = clock_timestamp()
             WHERE id = $1 AND status = 'QUEUED'"
        }
        DatabaseBackend::MySql => {
            "UPDATE feed_refresh_runs
             SET status = 'CANCELLED', completed_at = UTC_TIMESTAMP(6)
             WHERE id = ? AND status = 'QUEUED'"
        }
    };
    Statement::from_sql_and_values(backend, sql, [run_id.into()])
}

fn record_lease_lost_statement(backend: DbBackend, run_id: &str) -> Statement {
    let sql = match backend {
        DatabaseBackend::Sqlite => {
            "UPDATE feed_refresh_runs
             SET status = 'LEASE_LOST', completed_at = strftime('%Y-%m-%dT%H:%M:%f000Z', 'now')
             WHERE id = ? AND status = 'RUNNING'
               AND EXISTS (
                    SELECT 1 FROM feeds f
                    WHERE f.id = feed_refresh_runs.feed_id
                      AND (
                           f.lease_token <> feed_refresh_runs.lease_token
                           OR f.lease_owner IS NULL
                           OR f.lease_until IS NULL
                           OR julianday(f.lease_until) <= julianday('now')
                      )
               )"
        }
        DatabaseBackend::Postgres => {
            "UPDATE feed_refresh_runs
             SET status = 'LEASE_LOST', completed_at = clock_timestamp()
             WHERE id = $1 AND status = 'RUNNING'
               AND EXISTS (
                    SELECT 1 FROM feeds f
                    WHERE f.id = feed_refresh_runs.feed_id
                      AND (
                           f.lease_token <> feed_refresh_runs.lease_token
                           OR f.lease_owner IS NULL
                           OR f.lease_until IS NULL
                           OR f.lease_until <= clock_timestamp()
                      )
               )"
        }
        DatabaseBackend::MySql => {
            "UPDATE feed_refresh_runs
             SET status = 'LEASE_LOST', completed_at = UTC_TIMESTAMP(6)
             WHERE id = ? AND status = 'RUNNING'
               AND EXISTS (
                    SELECT 1 FROM feeds f
                    WHERE f.id = feed_refresh_runs.feed_id
                      AND (
                           f.lease_token <> feed_refresh_runs.lease_token
                           OR f.lease_owner IS NULL
                           OR f.lease_until IS NULL
                           OR f.lease_until <= UTC_TIMESTAMP(6)
                      )
               )"
        }
    };
    Statement::from_sql_and_values(backend, sql, [run_id.into()])
}

fn recover_run_statement(
    backend: DbBackend,
    run_id: &str,
    feed_id: &str,
    lease_token: i64,
) -> Statement {
    let sql = match backend {
        DatabaseBackend::Sqlite => {
            "UPDATE feed_refresh_runs
             SET status = 'LEASE_LOST', completed_at = strftime('%Y-%m-%dT%H:%M:%f000Z', 'now')
             WHERE id = ? AND feed_id = ? AND status = 'RUNNING' AND lease_token = ?"
        }
        DatabaseBackend::Postgres => {
            "UPDATE feed_refresh_runs
             SET status = 'LEASE_LOST', completed_at = clock_timestamp()
             WHERE id = $1 AND feed_id = $2 AND status = 'RUNNING' AND lease_token = $3"
        }
        DatabaseBackend::MySql => {
            "UPDATE feed_refresh_runs
             SET status = 'LEASE_LOST', completed_at = UTC_TIMESTAMP(6)
             WHERE id = ? AND feed_id = ? AND status = 'RUNNING' AND lease_token = ?"
        }
    };
    Statement::from_sql_and_values(
        backend,
        sql,
        [run_id.into(), feed_id.into(), lease_token.into()],
    )
}

fn terminal_run_statement(
    backend: DbBackend,
    claim: &RefreshClaim,
    terminal: &TerminalUpdate<'_>,
) -> Statement {
    let fetched_assignment = if terminal.status == RefreshStatus::NotModified {
        match backend {
            DatabaseBackend::Sqlite => "fetched_at = strftime('%Y-%m-%dT%H:%M:%f000Z', 'now'),",
            DatabaseBackend::Postgres => "fetched_at = clock_timestamp(),",
            DatabaseBackend::MySql => "fetched_at = UTC_TIMESTAMP(6),",
        }
    } else {
        ""
    };
    let sql = match backend {
        DatabaseBackend::Sqlite => {
            format!("UPDATE feed_refresh_runs
             SET {fetched_assignment} status = ?, http_status = ?, new_count = ?, updated_count = ?, dropped_count = ?,
                 error_code = ?, retry_at = ?,
                 completed_at = strftime('%Y-%m-%dT%H:%M:%f000Z', 'now')
             WHERE id = ? AND feed_id = ? AND status = 'RUNNING' AND lease_token = ?")
        }
        DatabaseBackend::Postgres => {
            format!("UPDATE feed_refresh_runs
             SET {fetched_assignment} status = $1, http_status = $2, new_count = $3, updated_count = $4,
                 dropped_count = $5, error_code = $6, retry_at = $7,
                 completed_at = clock_timestamp()
             WHERE id = $8 AND feed_id = $9 AND status = 'RUNNING' AND lease_token = $10")
        }
        DatabaseBackend::MySql => {
            format!("UPDATE feed_refresh_runs
             SET {fetched_assignment} status = ?, http_status = ?, new_count = ?, updated_count = ?, dropped_count = ?,
                 error_code = ?, retry_at = ?, completed_at = UTC_TIMESTAMP(6)
             WHERE id = ? AND feed_id = ? AND status = 'RUNNING' AND lease_token = ?")
        }
    };
    Statement::from_sql_and_values(
        backend,
        sql,
        [
            terminal.status.as_str().into(),
            terminal.http_status.into(),
            terminal.counts.new_count.into(),
            terminal.counts.updated_count.into(),
            terminal.counts.dropped_count.into(),
            terminal.error_code.into(),
            terminal.retry_at.into(),
            claim.run_id.as_str().into(),
            claim.feed_id.as_str().into(),
            claim.lease_token.into(),
        ],
    )
}

async fn read_run_status<C>(
    connection: &C,
    backend: DbBackend,
    run_id: &str,
) -> Result<RefreshStatus, RefreshRepositoryError>
where
    C: ConnectionTrait,
{
    let sql = match backend {
        DatabaseBackend::Postgres => "SELECT status FROM feed_refresh_runs WHERE id = $1",
        DatabaseBackend::Sqlite | DatabaseBackend::MySql => {
            "SELECT status FROM feed_refresh_runs WHERE id = ?"
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
    let status: String = row
        .try_get("", "status")
        .map_err(|_| RefreshRepositoryError::CorruptData)?;
    status
        .parse()
        .map_err(|_| RefreshRepositoryError::CorruptData)
}

async fn read_run_status_optional<C>(
    connection: &C,
    backend: DbBackend,
    run_id: &str,
) -> Result<Option<RefreshStatus>, RefreshRepositoryError>
where
    C: ConnectionTrait,
{
    let sql = match backend {
        DatabaseBackend::Postgres => "SELECT status FROM feed_refresh_runs WHERE id = $1",
        DatabaseBackend::Sqlite | DatabaseBackend::MySql => {
            "SELECT status FROM feed_refresh_runs WHERE id = ?"
        }
    };
    connection
        .query_one(Statement::from_sql_and_values(
            backend,
            sql,
            [run_id.into()],
        ))
        .await?
        .map(|row| {
            let status: String = row
                .try_get("", "status")
                .map_err(|_| RefreshRepositoryError::CorruptData)?;
            status
                .parse()
                .map_err(|_| RefreshRepositoryError::CorruptData)
        })
        .transpose()
}

async fn read_exact_run_state<C>(
    connection: &C,
    backend: DbBackend,
    run_id: &str,
) -> Result<Option<(RefreshStatus, bool)>, RefreshRepositoryError>
where
    C: ConnectionTrait,
{
    let sql = match backend {
        DatabaseBackend::Postgres => {
            "SELECT r.status, f.is_disabled FROM feed_refresh_runs r
             JOIN feeds f ON f.id = r.feed_id WHERE r.id = $1"
        }
        DatabaseBackend::Sqlite | DatabaseBackend::MySql => {
            "SELECT r.status, f.is_disabled FROM feed_refresh_runs r
             JOIN feeds f ON f.id = r.feed_id WHERE r.id = ?"
        }
    };
    connection
        .query_one(Statement::from_sql_and_values(
            backend,
            sql,
            [run_id.into()],
        ))
        .await?
        .map(|row| {
            let status: String = row
                .try_get("", "status")
                .map_err(|_| RefreshRepositoryError::CorruptData)?;
            let disabled: bool = row
                .try_get("", "is_disabled")
                .map_err(|_| RefreshRepositoryError::CorruptData)?;
            Ok((
                status
                    .parse()
                    .map_err(|_| RefreshRepositoryError::CorruptData)?,
                disabled,
            ))
        })
        .transpose()
}

async fn read_lease_state<C>(
    connection: &C,
    backend: DbBackend,
    feed_id: &str,
) -> Result<(i64, OffsetDateTime), RefreshRepositoryError>
where
    C: ConnectionTrait,
{
    let sql = match backend {
        DatabaseBackend::Postgres => "SELECT lease_token, lease_until FROM feeds WHERE id = $1",
        DatabaseBackend::Sqlite | DatabaseBackend::MySql => {
            "SELECT lease_token, lease_until FROM feeds WHERE id = ?"
        }
    };
    let row = connection
        .query_one(Statement::from_sql_and_values(
            backend,
            sql,
            [feed_id.into()],
        ))
        .await?
        .ok_or(RefreshRepositoryError::CorruptData)?;
    let token = row
        .try_get("", "lease_token")
        .map_err(|_| RefreshRepositoryError::CorruptData)?;
    let deadline = row
        .try_get("", "lease_until")
        .map_err(|_| RefreshRepositoryError::CorruptData)?;
    Ok((token, deadline))
}
