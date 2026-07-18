use constant_time_eq::constant_time_eq;
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, ConnectionTrait, DatabaseConnection,
    EntityTrait, QueryFilter, TransactionTrait,
};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::{
    content::provider::ProviderKind,
    db::entities::{content_artifact, content_job, content_job_result},
};

use super::{
    model::{
        ArtifactIdentity, ArtifactIdentityInput, ArtifactSnapshot, ContentRepositoryError,
        ContentRepositoryErrorKind, EnqueueContentJob, EnqueueResult, JobSnapshot, JobStatus,
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
