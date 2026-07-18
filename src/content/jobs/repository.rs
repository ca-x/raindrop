use constant_time_eq::constant_time_eq;
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, ConnectionTrait, DatabaseConnection,
    EntityTrait, QueryFilter, TransactionTrait, sea_query::Expr,
};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::{
    content::provider::ProviderKind,
    db::entities::{content_artifact, content_job, content_job_attempt, content_job_result},
};

use super::{
    model::{
        ArtifactIdentity, ArtifactIdentityInput, ArtifactSnapshot, ContentJobClaim,
        ContentRepositoryError, ContentRepositoryErrorKind, EnqueueContentJob, EnqueueResult,
        JobSnapshot, JobStatus, LeaseDeadline,
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
            })
        }
        .await;
        finish_transaction(transaction, result).await
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
        let snapshot = job_snapshot(updated)?;
        Ok(Some(super::model::ClaimOutcome::Claimed(ContentJobClaim {
            job_id: snapshot.id().to_owned(),
            user_id: snapshot.identity().user_id().to_owned(),
            entry_id: snapshot.identity().entry_id().to_owned(),
            attempt,
            lease_owner: request.owner().to_owned(),
            lease_token,
            lease_until,
            attempt_deadline_at,
            identity: snapshot.identity().clone(),
        })))
    }
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
