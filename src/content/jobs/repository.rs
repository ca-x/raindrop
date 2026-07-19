use constant_time_eq::constant_time_eq;
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, ConnectionTrait, DatabaseConnection,
    EntityTrait, QueryFilter, QueryOrder, TransactionTrait, sea_query::Expr,
};
use serde_json::Value;
use time::OffsetDateTime;
use uuid::Uuid;

use crate::{
    content::provider::ProviderKind,
    content::sanitize::extract_rendered_text,
    db::entities::{content_artifact, content_job, content_job_attempt, content_job_result},
    feeds::EntryContentDetail,
};

use super::{
    model::{
        ArtifactCandidate, ArtifactIdentity, ArtifactIdentityInput, ArtifactSnapshot,
        AttemptFailure, AttemptSnapshot, AttemptStatus, AttemptUsage, ContentExecutionEntry,
        ContentJobClaim, ContentRepositoryError, ContentRepositoryErrorKind, EnqueueContentJob,
        EnqueueResult, JobSnapshot, JobStatus, LeaseDeadline, StoredArtifactResult,
    },
    sql,
};

#[derive(Clone)]
pub struct ContentRepository {
    database: DatabaseConnection,
}

impl ContentRepository {
    #[must_use]
    pub fn new(database: DatabaseConnection) -> Self {
        Self { database }
    }

    pub async fn enqueue(
        &self,
        request: EnqueueContentJob,
    ) -> Result<EnqueueResult, ContentRepositoryError> {
        let backend = self.database.get_database_backend();
        let transaction = self.database.begin().await.map_err(sql::database_error)?;
        let result = async {
            let identity = request.identity();
            if !sql::lock_active_user(&transaction, backend, identity.user_id()).await? {
                return Err(not_found());
            }
            let content_hash = sql::visible_entry_content_hash(
                &transaction,
                backend,
                identity.user_id(),
                identity.entry_id(),
            )
            .await?
            .ok_or_else(not_found)?;
            if content_hash != identity.entry_content_hash() {
                return Err(ContentRepositoryError::new(
                    ContentRepositoryErrorKind::EntryChanged,
                ));
            }

            if let Some(existing) = content_job::Entity::find()
                .filter(content_job::Column::UserId.eq(identity.user_id()))
                .filter(content_job::Column::IdempotencyKeyHash.eq(request.idempotency_key_hash()))
                .one(&transaction)
                .await
                .map_err(sql::database_error)?
            {
                if !same_bytes(&existing.idempotency_key, request.idempotency_key()) {
                    return Err(ContentRepositoryError::new(
                        ContentRepositoryErrorKind::HashCollision,
                    ));
                }
                if existing.request_hash != request.request_hash() {
                    return Err(ContentRepositoryError::new(
                        ContentRepositoryErrorKind::IdempotencyConflict,
                    ));
                }
                return Ok(EnqueueResult::Existing(job_snapshot(existing)?));
            }

            let now = sql::database_now(&transaction, backend).await?;
            let artifact = content_artifact::Entity::find()
                .filter(content_artifact::Column::UserId.eq(identity.user_id()))
                .filter(content_artifact::Column::IdentityHash.eq(identity.hash()))
                .one(&transaction)
                .await
                .map_err(sql::database_error)?
                .map(|stored| artifact_matching(stored, identity))
                .transpose()?;
            let job = insert_job(
                &transaction,
                &request,
                now,
                if artifact.is_some() {
                    JobStatus::Succeeded
                } else {
                    JobStatus::Queued
                },
            )
            .await?;
            let snapshot = job_snapshot(job)?;
            if let Some(artifact) = artifact {
                content_job_result::ActiveModel {
                    job_id: Set(snapshot.id().to_owned()),
                    artifact_id: Set(artifact.id().to_owned()),
                    was_reused: Set(true),
                    linked_at: Set(now),
                }
                .insert(&transaction)
                .await
                .map_err(sql::database_error)?;
                Ok(EnqueueResult::Reused {
                    job: snapshot,
                    artifact: Box::new(artifact),
                })
            } else {
                Ok(EnqueueResult::Queued(snapshot))
            }
        }
        .await;
        finish_transaction(transaction, result).await
    }

    pub async fn get_job(
        &self,
        user_id: &str,
        job_id: &str,
    ) -> Result<JobSnapshot, ContentRepositoryError> {
        validate_lookup_id(user_id)?;
        validate_lookup_id(job_id)?;
        let stored = content_job::Entity::find_by_id(job_id)
            .filter(content_job::Column::UserId.eq(user_id))
            .one(&self.database)
            .await
            .map_err(sql::database_error)?
            .ok_or_else(not_found)?;
        job_snapshot(stored)
    }

    pub async fn find_latest_job_by_identity(
        &self,
        user_id: &str,
        identity: &ArtifactIdentity,
    ) -> Result<Option<JobSnapshot>, ContentRepositoryError> {
        validate_lookup_id(user_id)?;
        if user_id != identity.user_id() {
            return Err(ContentRepositoryError::new(
                ContentRepositoryErrorKind::InvalidInput,
            ));
        }
        let Some(stored) = content_job::Entity::find()
            .filter(content_job::Column::UserId.eq(user_id))
            .filter(content_job::Column::ArtifactIdentityHash.eq(identity.hash()))
            .order_by_desc(content_job::Column::CreatedAt)
            .order_by_desc(content_job::Column::Id)
            .one(&self.database)
            .await
            .map_err(sql::database_error)?
        else {
            return Ok(None);
        };
        let snapshot = job_snapshot(stored)?;
        if snapshot.identity() != identity {
            return Err(ContentRepositoryError::new(
                ContentRepositoryErrorKind::HashCollision,
            ));
        }
        Ok(Some(snapshot))
    }

    pub async fn claim_next(
        &self,
        request: super::model::ClaimContentJob,
    ) -> Result<super::model::ClaimOutcome, ContentRepositoryError> {
        let backend = self.database.get_database_backend();
        let candidates = sql::due_candidates(&self.database, backend).await?;
        for candidate in candidates {
            let transaction = self.database.begin().await.map_err(sql::database_error)?;
            let result = self
                .try_claim_candidate(&transaction, backend, &candidate, &request)
                .await;
            match result {
                Ok(Some(outcome)) => {
                    transaction.commit().await.map_err(sql::database_error)?;
                    return Ok(outcome);
                }
                Ok(None) => {
                    transaction.rollback().await.map_err(sql::database_error)?;
                }
                Err(error) => {
                    transaction.rollback().await.map_err(sql::database_error)?;
                    return Err(error);
                }
            }
        }
        Ok(super::model::ClaimOutcome::NoWork)
    }

    pub async fn heartbeat(
        &self,
        claim: &ContentJobClaim,
    ) -> Result<LeaseDeadline, ContentRepositoryError> {
        let backend = self.database.get_database_backend();
        let transaction = self.database.begin().await.map_err(sql::database_error)?;
        let result = async {
            if !sql::lock_active_user(&transaction, backend, claim.user_id()).await? {
                return Err(lease_lost());
            }
            if !sql::lock_job(&transaction, backend, claim.job_id()).await? {
                return Err(lease_lost());
            }
            let stored = content_job::Entity::find_by_id(claim.job_id())
                .one(&transaction)
                .await
                .map_err(sql::database_error)?
                .ok_or_else(lease_lost)?;
            let now = sql::database_now(&transaction, backend).await?;
            validate_claim(&stored, claim, now)?;
            let deadline = stored.attempt_deadline_at.ok_or_else(lease_lost)?;
            let lease_until = (now + time::Duration::seconds(30)).min(deadline);
            if lease_until <= now {
                return Err(lease_lost());
            }
            if !sql::heartbeat(
                &transaction,
                backend,
                &sql::HeartbeatWrite {
                    job_id: claim.job_id(),
                    user_id: claim.user_id(),
                    attempt: i32::from(claim.attempt()),
                    owner: claim.lease_owner(),
                    lease_token: claim.lease_token(),
                    lease_until,
                },
            )
            .await?
            {
                return Err(lease_lost());
            }
            Ok(LeaseDeadline {
                lease_until,
                attempt_deadline_at: deadline,
                remaining_attempt: std::time::Duration::try_from(deadline - now)
                    .map_err(|_| sql::corrupt_data())?,
            })
        }
        .await;
        finish_transaction(transaction, result).await
    }

    pub async fn load_execution_entry(
        &self,
        claim: &ContentJobClaim,
    ) -> Result<ContentExecutionEntry, ContentRepositoryError> {
        let entry = self
            .get_execution_entry_for_user(claim.user_id(), claim.entry_id())
            .await?;
        if entry.entry_id() != claim.entry_id()
            || entry.content_hash() != claim.identity().entry_content_hash()
        {
            return Err(ContentRepositoryError::new(
                ContentRepositoryErrorKind::EntryChanged,
            ));
        }
        Ok(entry)
    }

    pub async fn get_execution_entry_for_user(
        &self,
        user_id: &str,
        entry_id: &str,
    ) -> Result<ContentExecutionEntry, ContentRepositoryError> {
        validate_lookup_id(user_id)?;
        validate_lookup_id(entry_id)?;
        let backend = self.database.get_database_backend();
        let stored = sql::execution_entry(&self.database, backend, user_id, entry_id)
            .await?
            .ok_or_else(not_found)?;
        decode_execution_entry(stored)
    }

    pub async fn complete_failure(
        &self,
        claim: &ContentJobClaim,
        failure: AttemptFailure,
    ) -> Result<JobSnapshot, ContentRepositoryError> {
        let backend = self.database.get_database_backend();
        let transaction = self.database.begin().await.map_err(sql::database_error)?;
        let result = async {
            let stored = lock_claim_job(&transaction, backend, claim).await?;
            let now = sql::database_now(&transaction, backend).await?;
            validate_claim(&stored, claim, now)?;
            validate_claim_identity(&stored, claim)?;
            finish_attempt(
                &transaction,
                claim,
                &AttemptFinish {
                    status: AttemptStatus::Failed,
                    error_code: Some(failure.error_code()),
                    retryable: Some(failure.retryable()),
                    outcome_unknown: failure.outcome_unknown(),
                    usage: failure.usage(),
                    completed_at: now,
                },
            )
            .await?;

            let max_attempts =
                u8::try_from(stored.max_attempts).map_err(|_| sql::corrupt_data())?;
            if max_attempts != 3 || claim.attempt() > max_attempts {
                return Err(sql::corrupt_data());
            }
            let retrying = failure.retryable() && claim.attempt() < max_attempts;
            let next_attempt_at = if retrying {
                now + retry_delay(claim.attempt(), failure.retry_after())
            } else {
                now
            };
            let status = if retrying {
                JobStatus::RetryWait
            } else {
                JobStatus::Failed
            };
            if !sql::finish_job(
                &transaction,
                backend,
                &sql::JobFinishWrite {
                    status: status.as_storage(),
                    next_attempt_at,
                    completed_at: (!retrying).then_some(now),
                    error_code: Some(failure.error_code()),
                    job_id: claim.job_id(),
                    user_id: claim.user_id(),
                    attempt: i32::from(claim.attempt()),
                    owner: claim.lease_owner(),
                    lease_token: claim.lease_token(),
                },
            )
            .await?
            {
                return Err(lease_lost());
            }
            let updated = content_job::Entity::find_by_id(claim.job_id())
                .one(&transaction)
                .await
                .map_err(sql::database_error)?
                .ok_or_else(sql::corrupt_data)?;
            job_snapshot(updated)
        }
        .await;
        finish_transaction(transaction, result).await
    }

    pub async fn complete_success(
        &self,
        claim: &ContentJobClaim,
        candidate: ArtifactCandidate,
        usage: AttemptUsage,
    ) -> Result<StoredArtifactResult, ContentRepositoryError> {
        if candidate.identity() != claim.identity() {
            return Err(invalid());
        }
        let backend = self.database.get_database_backend();
        let transaction = self.database.begin().await.map_err(sql::database_error)?;
        let result = async {
            let stored = lock_claim_job(&transaction, backend, claim).await?;
            let now = sql::database_now(&transaction, backend).await?;
            validate_claim(&stored, claim, now)?;
            validate_claim_identity(&stored, claim)?;

            let existing = content_artifact::Entity::find()
                .filter(content_artifact::Column::UserId.eq(claim.user_id()))
                .filter(content_artifact::Column::IdentityHash.eq(claim.identity().hash()))
                .one(&transaction)
                .await
                .map_err(sql::database_error)?
                .map(|artifact| artifact_matching(artifact, claim.identity()))
                .transpose()?;
            let (artifact, was_reused) = if let Some(artifact) = existing {
                (artifact, true)
            } else {
                (
                    insert_artifact(&transaction, claim, &candidate, now).await?,
                    false,
                )
            };

            content_job_result::ActiveModel {
                job_id: Set(claim.job_id().to_owned()),
                artifact_id: Set(artifact.id().to_owned()),
                was_reused: Set(was_reused),
                linked_at: Set(now),
            }
            .insert(&transaction)
            .await
            .map_err(sql::database_error)?;
            finish_attempt(
                &transaction,
                claim,
                &AttemptFinish {
                    status: AttemptStatus::Succeeded,
                    error_code: None,
                    retryable: Some(false),
                    outcome_unknown: false,
                    usage: &usage,
                    completed_at: now,
                },
            )
            .await?;
            if !sql::finish_job(
                &transaction,
                backend,
                &sql::JobFinishWrite {
                    status: JobStatus::Succeeded.as_storage(),
                    next_attempt_at: now,
                    completed_at: Some(now),
                    error_code: None,
                    job_id: claim.job_id(),
                    user_id: claim.user_id(),
                    attempt: i32::from(claim.attempt()),
                    owner: claim.lease_owner(),
                    lease_token: claim.lease_token(),
                },
            )
            .await?
            {
                return Err(lease_lost());
            }
            Ok(StoredArtifactResult {
                artifact,
                was_reused,
            })
        }
        .await;
        finish_transaction(transaction, result).await
    }

    pub async fn list_attempts(
        &self,
        user_id: &str,
        job_id: &str,
    ) -> Result<Vec<AttemptSnapshot>, ContentRepositoryError> {
        self.get_job(user_id, job_id).await?;
        content_job_attempt::Entity::find()
            .filter(content_job_attempt::Column::JobId.eq(job_id))
            .order_by_asc(content_job_attempt::Column::Attempt)
            .all(&self.database)
            .await
            .map_err(sql::database_error)?
            .into_iter()
            .map(attempt_snapshot)
            .collect()
    }

    pub async fn get_result(
        &self,
        user_id: &str,
        job_id: &str,
    ) -> Result<StoredArtifactResult, ContentRepositoryError> {
        let job = self.get_job(user_id, job_id).await?;
        let result = content_job_result::Entity::find_by_id(job_id)
            .one(&self.database)
            .await
            .map_err(sql::database_error)?
            .ok_or_else(not_found)?;
        let stored = content_artifact::Entity::find_by_id(&result.artifact_id)
            .one(&self.database)
            .await
            .map_err(sql::database_error)?
            .ok_or_else(sql::corrupt_data)?;
        let artifact = artifact_matching(stored, job.identity())?;
        if artifact.id() != result.artifact_id {
            return Err(sql::corrupt_data());
        }
        Ok(StoredArtifactResult {
            artifact,
            was_reused: result.was_reused,
        })
    }

    pub async fn find_artifact_by_identity(
        &self,
        user_id: &str,
        identity: &ArtifactIdentity,
    ) -> Result<Option<ArtifactSnapshot>, ContentRepositoryError> {
        validate_lookup_id(user_id)?;
        if user_id != identity.user_id() {
            return Err(ContentRepositoryError::new(
                ContentRepositoryErrorKind::InvalidInput,
            ));
        }
        content_artifact::Entity::find()
            .filter(content_artifact::Column::UserId.eq(user_id))
            .filter(content_artifact::Column::IdentityHash.eq(identity.hash()))
            .one(&self.database)
            .await
            .map_err(sql::database_error)?
            .map(|stored| artifact_matching(stored, identity))
            .transpose()
    }

    async fn try_claim_candidate(
        &self,
        transaction: &sea_orm::DatabaseTransaction,
        backend: sea_orm::DatabaseBackend,
        candidate: &sql::CandidateJob,
        request: &super::model::ClaimContentJob,
    ) -> Result<Option<super::model::ClaimOutcome>, ContentRepositoryError> {
        if !sql::lock_active_user(transaction, backend, &candidate.user_id).await? {
            return Ok(None);
        }
        if !sql::lock_job(transaction, backend, &candidate.id).await? {
            return Ok(None);
        }
        let stored = content_job::Entity::find_by_id(&candidate.id)
            .one(transaction)
            .await
            .map_err(sql::database_error)?
            .ok_or_else(sql::corrupt_data)?;
        if stored.user_id != candidate.user_id {
            return Err(sql::corrupt_data());
        }
        let now = sql::database_now(transaction, backend).await?;
        let status = JobStatus::from_storage(&stored.status).map_err(|_| sql::corrupt_data())?;
        let due = matches!(status, JobStatus::Queued | JobStatus::RetryWait)
            && stored.next_attempt_at <= now;
        let expired = status == JobStatus::Running
            && (stored.lease_until.is_none_or(|deadline| deadline <= now)
                || stored
                    .attempt_deadline_at
                    .is_none_or(|deadline| deadline <= now));
        if !due && !expired {
            return Ok(None);
        }
        if sql::active_running_count(transaction, backend, &stored.user_id).await? >= 2 {
            return Ok(None);
        }

        if expired {
            abandon_attempt(transaction, &stored, now).await?;
        }
        let attempts = u8::try_from(stored.attempts).map_err(|_| sql::corrupt_data())?;
        let max_attempts = u8::try_from(stored.max_attempts).map_err(|_| sql::corrupt_data())?;
        if max_attempts != 3 || attempts > max_attempts {
            return Err(sql::corrupt_data());
        }
        if attempts == max_attempts {
            let terminal =
                terminalize_recovery(transaction, stored, now, "JOB_ATTEMPTS_EXHAUSTED").await?;
            return Ok(Some(super::model::ClaimOutcome::RecoveredTerminal(
                job_snapshot(terminal)?,
            )));
        }
        if stored.lease_token < 0 {
            return Err(sql::corrupt_data());
        }
        if stored.lease_token == i64::MAX {
            let terminal =
                terminalize_recovery(transaction, stored, now, "JOB_FENCE_EXHAUSTED").await?;
            return Ok(Some(super::model::ClaimOutcome::RecoveredTerminal(
                job_snapshot(terminal)?,
            )));
        }

        let attempt = attempts + 1;
        let lease_token = stored.lease_token + 1;
        let timeout_seconds = i64::from(stored.timeout_seconds);
        if !matches!(timeout_seconds, 120 | 180) {
            return Err(sql::corrupt_data());
        }
        let attempt_deadline_at = now + time::Duration::seconds(timeout_seconds);
        let lease_until = now + time::Duration::seconds(30);
        let mut active: content_job::ActiveModel = stored.into();
        active.status = Set(JobStatus::Running.as_storage().to_owned());
        active.attempts = Set(i32::from(attempt));
        active.lease_owner = Set(Some(request.owner().to_owned()));
        active.lease_token = Set(lease_token);
        active.lease_until = Set(Some(lease_until));
        active.attempt_deadline_at = Set(Some(attempt_deadline_at));
        active.next_attempt_at = Set(now);
        active.last_error_code = Set(None);
        if active.started_at.as_ref().is_none() {
            active.started_at = Set(Some(now));
        }
        let updated = active
            .update(transaction)
            .await
            .map_err(sql::database_error)?;
        content_job_attempt::ActiveModel {
            id: Set(Uuid::new_v4().to_string()),
            job_id: Set(updated.id.clone()),
            attempt: Set(i32::from(attempt)),
            lease_token: Set(lease_token),
            status: Set(super::model::AttemptStatus::Running.as_storage().to_owned()),
            started_at: Set(now),
            deadline_at: Set(attempt_deadline_at),
            completed_at: Set(None),
            error_code: Set(None),
            retryable: Set(None),
            outcome_unknown: Set(false),
            provider_request_count: Set(0),
            mcp_call_count: Set(0),
            input_tokens: Set(0),
            output_tokens: Set(0),
            estimated_cost_micros: Set(0),
            execution_metadata_json: Set("{}".to_owned()),
        }
        .insert(transaction)
        .await
        .map_err(sql::database_error)?;
        let snapshot = job_snapshot(updated.clone())?;
        let remaining_depth =
            u8::try_from(updated.remaining_depth).map_err(|_| sql::corrupt_data())?;
        if updated.idempotency_key.is_empty()
            || updated.idempotency_key.len() > 255
            || !updated
                .idempotency_key
                .bytes()
                .all(|byte| byte.is_ascii_graphic() || byte == b' ')
            || updated.call_chain_id.is_empty()
            || updated.call_chain_id.len() > 64
            || !updated
                .call_chain_id
                .bytes()
                .all(|byte| byte.is_ascii_graphic() || byte == b' ')
            || remaining_depth > 4
        {
            return Err(sql::corrupt_data());
        }
        Ok(Some(super::model::ClaimOutcome::Claimed(ContentJobClaim {
            job_id: snapshot.id().to_owned(),
            user_id: snapshot.identity().user_id().to_owned(),
            entry_id: snapshot.identity().entry_id().to_owned(),
            operation: snapshot.operation(),
            trigger: snapshot.trigger(),
            idempotency_key: updated.idempotency_key,
            call_chain_id: updated.call_chain_id,
            remaining_depth,
            attempt,
            lease_owner: request.owner().to_owned(),
            lease_token,
            lease_until,
            attempt_deadline_at,
            identity: snapshot.identity().clone(),
        })))
    }
}

async fn lock_claim_job(
    transaction: &sea_orm::DatabaseTransaction,
    backend: sea_orm::DatabaseBackend,
    claim: &ContentJobClaim,
) -> Result<content_job::Model, ContentRepositoryError> {
    if !sql::lock_active_user(transaction, backend, claim.user_id()).await? {
        return Err(lease_lost());
    }
    if !sql::lock_job(transaction, backend, claim.job_id()).await? {
        return Err(lease_lost());
    }
    let stored = content_job::Entity::find_by_id(claim.job_id())
        .one(transaction)
        .await
        .map_err(sql::database_error)?
        .ok_or_else(lease_lost)?;
    if matches!(
        JobStatus::from_storage(&stored.status).map_err(|_| sql::corrupt_data())?,
        JobStatus::Succeeded | JobStatus::Failed
    ) {
        Err(ContentRepositoryError::new(
            ContentRepositoryErrorKind::AlreadyCompleted,
        ))
    } else {
        Ok(stored)
    }
}

fn validate_claim_identity(
    stored: &content_job::Model,
    claim: &ContentJobClaim,
) -> Result<(), ContentRepositoryError> {
    let snapshot = job_snapshot(stored.clone())?;
    if snapshot.identity() == claim.identity() {
        Ok(())
    } else {
        Err(sql::corrupt_data())
    }
}

struct AttemptFinish<'a> {
    status: AttemptStatus,
    error_code: Option<&'a str>,
    retryable: Option<bool>,
    outcome_unknown: bool,
    usage: &'a AttemptUsage,
    completed_at: OffsetDateTime,
}

async fn finish_attempt(
    transaction: &sea_orm::DatabaseTransaction,
    claim: &ContentJobClaim,
    finish: &AttemptFinish<'_>,
) -> Result<(), ContentRepositoryError> {
    let result = content_job_attempt::Entity::update_many()
        .col_expr(
            content_job_attempt::Column::Status,
            Expr::value(finish.status.as_storage()),
        )
        .col_expr(
            content_job_attempt::Column::CompletedAt,
            Expr::value(finish.completed_at),
        )
        .col_expr(
            content_job_attempt::Column::ErrorCode,
            Expr::value(finish.error_code),
        )
        .col_expr(
            content_job_attempt::Column::Retryable,
            Expr::value(finish.retryable),
        )
        .col_expr(
            content_job_attempt::Column::OutcomeUnknown,
            Expr::value(finish.outcome_unknown),
        )
        .col_expr(
            content_job_attempt::Column::ProviderRequestCount,
            Expr::value(i32::from(finish.usage.provider_request_count())),
        )
        .col_expr(
            content_job_attempt::Column::McpCallCount,
            Expr::value(i32::from(finish.usage.mcp_call_count())),
        )
        .col_expr(
            content_job_attempt::Column::InputTokens,
            Expr::value(i64::try_from(finish.usage.input_tokens()).map_err(|_| invalid())?),
        )
        .col_expr(
            content_job_attempt::Column::OutputTokens,
            Expr::value(i64::try_from(finish.usage.output_tokens()).map_err(|_| invalid())?),
        )
        .col_expr(
            content_job_attempt::Column::EstimatedCostMicros,
            Expr::value(
                i64::try_from(finish.usage.estimated_cost_micros()).map_err(|_| invalid())?,
            ),
        )
        .col_expr(
            content_job_attempt::Column::ExecutionMetadataJson,
            Expr::value(finish.usage.execution_metadata_json()),
        )
        .filter(content_job_attempt::Column::JobId.eq(claim.job_id()))
        .filter(content_job_attempt::Column::Attempt.eq(i32::from(claim.attempt())))
        .filter(content_job_attempt::Column::LeaseToken.eq(claim.lease_token()))
        .filter(content_job_attempt::Column::Status.eq(AttemptStatus::Running.as_storage()))
        .exec(transaction)
        .await
        .map_err(sql::database_error)?;
    if result.rows_affected == 1 {
        Ok(())
    } else {
        Err(lease_lost())
    }
}

fn retry_delay(attempt: u8, retry_after: Option<std::time::Duration>) -> time::Duration {
    let base_seconds: u64 = match attempt {
        1 => 5,
        2 => 30,
        _ => 0,
    };
    let retry_after_seconds = retry_after.map_or(0, |duration| duration.as_secs());
    let seconds = base_seconds.max(retry_after_seconds).min(60 * 60);
    time::Duration::seconds(i64::try_from(seconds).unwrap_or(60 * 60))
}

async fn insert_artifact(
    transaction: &sea_orm::DatabaseTransaction,
    claim: &ContentJobClaim,
    candidate: &ArtifactCandidate,
    now: OffsetDateTime,
) -> Result<ArtifactSnapshot, ContentRepositoryError> {
    let identity = claim.identity();
    let stored = content_artifact::ActiveModel {
        id: Set(Uuid::new_v4().to_string()),
        user_id: Set(identity.user_id().to_owned()),
        entry_id: Set(identity.entry_id().to_owned()),
        producer_job_id: Set(claim.job_id().to_owned()),
        kind: Set(identity.kind().as_storage().to_owned()),
        locale: Set(identity.target_locale().map(str::to_owned)),
        schema_id: Set(identity.schema_id().to_owned()),
        entry_content_hash: Set(identity.entry_content_hash().to_owned()),
        input_hash: Set(identity.input_hash().to_owned()),
        config_hash: Set(identity.config_hash().to_owned()),
        processor_key: Set(identity.plugin_key().to_owned()),
        processor_version: Set(identity.plugin_version().to_owned()),
        component_digest: Set(identity.component_digest().to_owned()),
        provider_binding_id: Set(identity.provider_binding_id().to_owned()),
        provider_kind: Set(identity.provider_kind().as_storage().to_owned()),
        provider_model: Set(identity.provider_model().to_owned()),
        provider_revision: Set(i64::try_from(identity.provider_revision()).map_err(|_| invalid())?),
        provider_label: Set(candidate.provider_label().to_owned()),
        prompt_version: Set(identity.prompt_version().to_owned()),
        mcp_provenance_hash: Set(identity.mcp_provenance_hash().to_owned()),
        identity_hash: Set(identity.hash().to_owned()),
        payload_json: Set(candidate.payload_json().to_owned()),
        provenance_json: Set(candidate.provenance_json().to_owned()),
        payload_size_bytes: Set(
            i32::try_from(candidate.payload_size_bytes()).map_err(|_| invalid())?
        ),
        created_at: Set(now),
    }
    .insert(transaction)
    .await
    .map_err(sql::database_error)?;
    artifact_matching(stored, identity)
}

fn attempt_snapshot(
    model: content_job_attempt::Model,
) -> Result<AttemptSnapshot, ContentRepositoryError> {
    let attempt = u8::try_from(model.attempt).map_err(|_| sql::corrupt_data())?;
    let provider_request_count =
        u8::try_from(model.provider_request_count).map_err(|_| sql::corrupt_data())?;
    let mcp_call_count = u8::try_from(model.mcp_call_count).map_err(|_| sql::corrupt_data())?;
    let input_tokens = u64::try_from(model.input_tokens).map_err(|_| sql::corrupt_data())?;
    let output_tokens = u64::try_from(model.output_tokens).map_err(|_| sql::corrupt_data())?;
    let estimated_cost_micros =
        u64::try_from(model.estimated_cost_micros).map_err(|_| sql::corrupt_data())?;
    let metadata: Value =
        serde_json::from_str(&model.execution_metadata_json).map_err(|_| sql::corrupt_data())?;
    let usage = AttemptUsage::new(
        provider_request_count,
        mcp_call_count,
        input_tokens,
        output_tokens,
        estimated_cost_micros,
        metadata,
    )
    .map_err(|_| sql::corrupt_data())?;
    if usage.execution_metadata_json() != model.execution_metadata_json {
        return Err(sql::corrupt_data());
    }
    let status = AttemptStatus::from_storage(&model.status).map_err(|_| sql::corrupt_data())?;
    let terminal = status != AttemptStatus::Running;
    if terminal != model.completed_at.is_some()
        || terminal != model.retryable.is_some()
        || (status == AttemptStatus::Succeeded
            && (model.error_code.is_some() || model.retryable != Some(false)))
    {
        return Err(sql::corrupt_data());
    }
    Ok(AttemptSnapshot {
        attempt,
        lease_token: model.lease_token,
        status,
        started_at: model.started_at,
        deadline_at: model.deadline_at,
        completed_at: model.completed_at,
        error_code: model.error_code,
        retryable: model.retryable,
        outcome_unknown: model.outcome_unknown,
        usage,
    })
}

async fn abandon_attempt(
    transaction: &sea_orm::DatabaseTransaction,
    stored: &content_job::Model,
    now: OffsetDateTime,
) -> Result<(), ContentRepositoryError> {
    if stored.attempts <= 0 || stored.lease_token <= 0 {
        return Err(sql::corrupt_data());
    }
    let result = content_job_attempt::Entity::update_many()
        .col_expr(
            content_job_attempt::Column::Status,
            Expr::value(super::model::AttemptStatus::Abandoned.as_storage()),
        )
        .col_expr(content_job_attempt::Column::CompletedAt, Expr::value(now))
        .col_expr(
            content_job_attempt::Column::ErrorCode,
            Expr::value("JOB_LEASE_EXPIRED"),
        )
        .col_expr(content_job_attempt::Column::Retryable, Expr::value(true))
        .col_expr(
            content_job_attempt::Column::OutcomeUnknown,
            Expr::value(false),
        )
        .filter(content_job_attempt::Column::JobId.eq(&stored.id))
        .filter(content_job_attempt::Column::Attempt.eq(stored.attempts))
        .filter(content_job_attempt::Column::LeaseToken.eq(stored.lease_token))
        .filter(
            content_job_attempt::Column::Status
                .eq(super::model::AttemptStatus::Running.as_storage()),
        )
        .exec(transaction)
        .await
        .map_err(sql::database_error)?;
    if result.rows_affected == 1 {
        Ok(())
    } else {
        Err(sql::corrupt_data())
    }
}

async fn terminalize_recovery(
    transaction: &sea_orm::DatabaseTransaction,
    stored: content_job::Model,
    now: OffsetDateTime,
    error_code: &str,
) -> Result<content_job::Model, ContentRepositoryError> {
    let mut active: content_job::ActiveModel = stored.into();
    active.status = Set(JobStatus::Failed.as_storage().to_owned());
    active.lease_owner = Set(None);
    active.lease_until = Set(None);
    active.attempt_deadline_at = Set(None);
    active.last_error_code = Set(Some(error_code.to_owned()));
    active.completed_at = Set(Some(now));
    active
        .update(transaction)
        .await
        .map_err(sql::database_error)
}

fn validate_claim(
    stored: &content_job::Model,
    claim: &ContentJobClaim,
    now: OffsetDateTime,
) -> Result<(), ContentRepositoryError> {
    if stored.user_id != claim.user_id()
        || stored.entry_id != claim.entry_id()
        || stored.status != JobStatus::Running.as_storage()
        || stored.attempts != i32::from(claim.attempt())
        || stored.lease_owner.as_deref() != Some(claim.lease_owner())
        || stored.lease_token != claim.lease_token()
        || stored.lease_until.is_none_or(|deadline| deadline <= now)
        || stored
            .attempt_deadline_at
            .is_none_or(|deadline| deadline <= now)
    {
        Err(lease_lost())
    } else {
        Ok(())
    }
}

async fn insert_job(
    transaction: &sea_orm::DatabaseTransaction,
    request: &EnqueueContentJob,
    now: OffsetDateTime,
    status: JobStatus,
) -> Result<content_job::Model, ContentRepositoryError> {
    let identity = request.identity();
    let provider_revision = i64::try_from(identity.provider_revision()).map_err(|_| invalid())?;
    let completed_at = (status == JobStatus::Succeeded).then_some(now);
    content_job::ActiveModel {
        id: Set(Uuid::new_v4().to_string()),
        user_id: Set(identity.user_id().to_owned()),
        entry_id: Set(identity.entry_id().to_owned()),
        operation: Set(request.operation().as_storage().to_owned()),
        artifact_kind: Set(identity.kind().as_storage().to_owned()),
        target_locale: Set(identity.target_locale().map(str::to_owned)),
        trigger_kind: Set(request.trigger().as_storage().to_owned()),
        plugin_key: Set(identity.plugin_key().to_owned()),
        plugin_version: Set(identity.plugin_version().to_owned()),
        component_digest: Set(identity.component_digest().to_owned()),
        provider_binding_id: Set(identity.provider_binding_id().to_owned()),
        provider_kind: Set(identity.provider_kind().as_storage().to_owned()),
        provider_model: Set(identity.provider_model().to_owned()),
        provider_revision: Set(provider_revision),
        prompt_version: Set(identity.prompt_version().to_owned()),
        schema_id: Set(identity.schema_id().to_owned()),
        entry_content_hash: Set(identity.entry_content_hash().to_owned()),
        input_hash: Set(identity.input_hash().to_owned()),
        config_hash: Set(identity.config_hash().to_owned()),
        mcp_provenance_hash: Set(identity.mcp_provenance_hash().to_owned()),
        artifact_identity_hash: Set(identity.hash().to_owned()),
        idempotency_key: Set(request.idempotency_key().to_owned()),
        idempotency_key_hash: Set(request.idempotency_key_hash().to_owned()),
        request_hash: Set(request.request_hash().to_owned()),
        call_chain_id: Set(request.call_chain_id().to_owned()),
        remaining_depth: Set(i32::from(request.remaining_depth())),
        status: Set(status.as_storage().to_owned()),
        attempts: Set(0),
        max_attempts: Set(i32::from(request.max_attempts())),
        timeout_seconds: Set(i32::from(request.timeout_seconds())),
        next_attempt_at: Set(now),
        lease_owner: Set(None),
        lease_token: Set(0),
        lease_until: Set(None),
        attempt_deadline_at: Set(None),
        last_error_code: Set(None),
        created_at: Set(now),
        started_at: Set(None),
        completed_at: Set(completed_at),
    }
    .insert(transaction)
    .await
    .map_err(sql::database_error)
}

fn job_snapshot(model: content_job::Model) -> Result<JobSnapshot, ContentRepositoryError> {
    let operation = super::model::ContentJobOperation::from_storage(&model.operation)
        .map_err(|_| sql::corrupt_data())?;
    let trigger = super::model::ContentJobTrigger::from_storage(&model.trigger_kind)
        .map_err(|_| sql::corrupt_data())?;
    let kind = super::model::ArtifactKind::from_storage(&model.artifact_kind)
        .map_err(|_| sql::corrupt_data())?;
    if operation.artifact_kind() != kind {
        return Err(sql::corrupt_data());
    }
    let provider_kind =
        ProviderKind::from_storage(&model.provider_kind).map_err(|_| sql::corrupt_data())?;
    let provider_revision =
        u64::try_from(model.provider_revision).map_err(|_| sql::corrupt_data())?;
    let identity = ArtifactIdentity::new(ArtifactIdentityInput {
        user_id: model.user_id,
        entry_id: model.entry_id,
        kind,
        target_locale: model.target_locale,
        entry_content_hash: model.entry_content_hash,
        input_hash: model.input_hash,
        config_hash: model.config_hash,
        plugin_key: model.plugin_key,
        plugin_version: model.plugin_version,
        component_digest: model.component_digest,
        provider_binding_id: model.provider_binding_id,
        provider_kind,
        provider_model: model.provider_model,
        provider_revision,
        prompt_version: model.prompt_version,
        schema_id: model.schema_id,
        mcp_provenance_hash: model.mcp_provenance_hash,
    })
    .map_err(|_| sql::corrupt_data())?;
    if identity.hash() != model.artifact_identity_hash {
        return Err(sql::corrupt_data());
    }
    let attempts = u8::try_from(model.attempts).map_err(|_| sql::corrupt_data())?;
    let max_attempts = u8::try_from(model.max_attempts).map_err(|_| sql::corrupt_data())?;
    if max_attempts != 3
        || attempts > max_attempts
        || model.timeout_seconds != i32::from(trigger.timeout_seconds())
    {
        return Err(sql::corrupt_data());
    }
    Ok(JobSnapshot {
        id: model.id,
        operation,
        trigger,
        identity,
        status: JobStatus::from_storage(&model.status).map_err(|_| sql::corrupt_data())?,
        attempts,
        max_attempts,
        next_attempt_at: model.next_attempt_at,
        last_error_code: model.last_error_code,
        created_at: model.created_at,
        started_at: model.started_at,
        completed_at: model.completed_at,
    })
}

fn artifact_matching(
    model: content_artifact::Model,
    expected: &ArtifactIdentity,
) -> Result<ArtifactSnapshot, ContentRepositoryError> {
    validate_stored_json(
        &model.payload_json,
        super::model::MAX_ARTIFACT_BYTES,
        ContentRepositoryErrorKind::ArtifactTooLarge,
    )?;
    validate_stored_json(
        &model.provenance_json,
        super::model::MAX_METADATA_BYTES,
        ContentRepositoryErrorKind::InvalidInput,
    )?;
    let kind =
        super::model::ArtifactKind::from_storage(&model.kind).map_err(|_| sql::corrupt_data())?;
    let provider_kind =
        ProviderKind::from_storage(&model.provider_kind).map_err(|_| sql::corrupt_data())?;
    let provider_revision =
        u64::try_from(model.provider_revision).map_err(|_| sql::corrupt_data())?;
    let identity = ArtifactIdentity::new(ArtifactIdentityInput {
        user_id: model.user_id,
        entry_id: model.entry_id,
        kind,
        target_locale: model.locale,
        entry_content_hash: model.entry_content_hash,
        input_hash: model.input_hash,
        config_hash: model.config_hash,
        plugin_key: model.processor_key,
        plugin_version: model.processor_version,
        component_digest: model.component_digest,
        provider_binding_id: model.provider_binding_id,
        provider_kind,
        provider_model: model.provider_model,
        provider_revision,
        prompt_version: model.prompt_version,
        schema_id: model.schema_id,
        mcp_provenance_hash: model.mcp_provenance_hash,
    })
    .map_err(|_| sql::corrupt_data())?;
    if &identity != expected {
        return Err(ContentRepositoryError::new(
            ContentRepositoryErrorKind::HashCollision,
        ));
    }
    if model.identity_hash != expected.hash()
        || model.payload_size_bytes < 0
        || usize::try_from(model.payload_size_bytes).ok() != Some(model.payload_json.len())
    {
        return Err(sql::corrupt_data());
    }
    Ok(ArtifactSnapshot {
        id: model.id,
        producer_job_id: model.producer_job_id,
        identity,
        provider_label: model.provider_label,
        payload_json: model.payload_json,
        provenance_json: model.provenance_json,
        created_at: model.created_at,
    })
}

fn decode_execution_entry(
    stored: sql::ExecutionEntryRow,
) -> Result<ContentExecutionEntry, ContentRepositoryError> {
    let detail = EntryContentDetail::decode(&stored.sanitized_content)
        .map_err(|_| ContentRepositoryError::new(ContentRepositoryErrorKind::CorruptData))?;
    let text = extract_rendered_text(detail.html()).trim().to_owned();
    if text.len() > 512 * 1024 {
        return Err(ContentRepositoryError::new(
            ContentRepositoryErrorKind::ExecutionInputTooLarge,
        ));
    }
    Ok(ContentExecutionEntry {
        entry_id: stored.entry_id,
        feed_id: stored.feed_id,
        content_hash: stored.content_hash,
        title: stored.title,
        text,
        canonical_url: stored.canonical_url,
    })
}

fn validate_stored_json(
    encoded: &str,
    max_bytes: usize,
    too_large: ContentRepositoryErrorKind,
) -> Result<(), ContentRepositoryError> {
    let value: Value = serde_json::from_str(encoded).map_err(|_| sql::corrupt_data())?;
    let canonical = super::hash::canonical_json(value, max_bytes, too_large)
        .map_err(|_| sql::corrupt_data())?;
    if canonical == encoded {
        Ok(())
    } else {
        Err(sql::corrupt_data())
    }
}

async fn finish_transaction<T>(
    transaction: sea_orm::DatabaseTransaction,
    result: Result<T, ContentRepositoryError>,
) -> Result<T, ContentRepositoryError> {
    match result {
        Ok(value) => {
            transaction.commit().await.map_err(sql::database_error)?;
            Ok(value)
        }
        Err(error) => {
            transaction.rollback().await.map_err(sql::database_error)?;
            Err(error)
        }
    }
}

fn same_bytes(left: &str, right: &str) -> bool {
    left.len() == right.len() && constant_time_eq(left.as_bytes(), right.as_bytes())
}

fn validate_lookup_id(value: &str) -> Result<(), ContentRepositoryError> {
    if Uuid::parse_str(value).is_ok_and(|id| id.to_string() == value) {
        Ok(())
    } else {
        Err(invalid())
    }
}

const fn invalid() -> ContentRepositoryError {
    ContentRepositoryError::new(ContentRepositoryErrorKind::InvalidInput)
}

const fn not_found() -> ContentRepositoryError {
    ContentRepositoryError::new(ContentRepositoryErrorKind::NotFound)
}

const fn lease_lost() -> ContentRepositoryError {
    ContentRepositoryError::new(ContentRepositoryErrorKind::LeaseLost)
}
