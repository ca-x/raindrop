use sea_orm::{
    ConnectionTrait, DatabaseBackend, DatabaseConnection, DbBackend, QueryResult, Statement,
    TransactionTrait,
};
use time::OffsetDateTime;
use uuid::Uuid;

use super::lifecycle::{record_completed_event, valid_error_code};
use super::{
    ClaimRequest, QueueRefreshRequest, RefreshClaim, RefreshCounts, RefreshFailure,
    RefreshRepositoryError, RefreshRun, RefreshStatus,
};

const MAX_LEASE_TOKEN: i64 = i64::MAX;

#[derive(Debug)]
struct ClaimCandidate {
    run_id: String,
    feed_id: String,
    lease_token: i64,
}

#[derive(Debug)]
struct TerminalUpdate<'a> {
    status: RefreshStatus,
    http_status: Option<i32>,
    counts: RefreshCounts,
    error_code: Option<&'a str>,
    retry_at: Option<OffsetDateTime>,
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

    pub async fn queue_refresh(
        &self,
        request: QueueRefreshRequest,
    ) -> Result<RefreshRun, RefreshRepositoryError> {
        validate_queue_request(&request)?;
        let backend = self.database.get_database_backend();
        if let Some(existing) = find_run_by_idempotency(
            &self.database,
            backend,
            &request.feed_id,
            &request.idempotency_key,
        )
        .await?
        {
            return idempotent_result(existing, &request);
        }

        let run_id = Uuid::new_v4().to_string();
        if let Err(error) = self
            .database
            .execute(queue_run_statement(backend, &run_id, &request))
            .await
        {
            if let Some(existing) = find_run_by_idempotency(
                &self.database,
                backend,
                &request.feed_id,
                &request.idempotency_key,
            )
            .await?
            {
                return idempotent_result(existing, &request);
            }
            return Err(RefreshRepositoryError::Database(error));
        }

        find_run_by_idempotency(
            &self.database,
            backend,
            &request.feed_id,
            &request.idempotency_key,
        )
        .await?
        .ok_or(RefreshRepositoryError::CorruptData)
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
        if backend == DatabaseBackend::MySql {
            let current_token = lock_feed_for_mysql(&transaction, &claim.feed_id).await?;
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
        if backend == DatabaseBackend::MySql {
            let current_token = lock_feed_for_mysql(&transaction, &claim.feed_id).await?;
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

fn idempotent_result(
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

fn queue_run_statement(
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

async fn find_run_by_idempotency<C>(
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

fn terminal_run_statement(
    backend: DbBackend,
    claim: &RefreshClaim,
    terminal: &TerminalUpdate<'_>,
) -> Statement {
    let sql = match backend {
        DatabaseBackend::Sqlite => {
            "UPDATE feed_refresh_runs
             SET status = ?, http_status = ?, new_count = ?, updated_count = ?, dropped_count = ?,
                 error_code = ?, retry_at = ?,
                 completed_at = strftime('%Y-%m-%dT%H:%M:%f000Z', 'now')
             WHERE id = ? AND feed_id = ? AND status = 'RUNNING' AND lease_token = ?"
        }
        DatabaseBackend::Postgres => {
            "UPDATE feed_refresh_runs
             SET status = $1, http_status = $2, new_count = $3, updated_count = $4,
                 dropped_count = $5, error_code = $6, retry_at = $7,
                 completed_at = clock_timestamp()
             WHERE id = $8 AND feed_id = $9 AND status = 'RUNNING' AND lease_token = $10"
        }
        DatabaseBackend::MySql => {
            "UPDATE feed_refresh_runs
             SET status = ?, http_status = ?, new_count = ?, updated_count = ?, dropped_count = ?,
                 error_code = ?, retry_at = ?, completed_at = UTC_TIMESTAMP(6)
             WHERE id = ? AND feed_id = ? AND status = 'RUNNING' AND lease_token = ?"
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
