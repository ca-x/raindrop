use std::{collections::HashSet, sync::Arc};

use sea_orm::{
    ActiveModelTrait,
    ActiveValue::Set,
    ColumnTrait, ConnectionTrait, DatabaseBackend, DatabaseConnection, DbErr, EntityTrait,
    PaginatorTrait, QueryFilter, QueryOrder, QuerySelect, Statement, TransactionTrait,
    sea_query::Expr,
    sqlx::{
        error::Error as SqlxError, mysql::MySqlDatabaseError as SqlxMySqlError,
        postgres::PgDatabaseError as SqlxPostgresError, sqlite::SqliteError as SqlxSqliteError,
    },
};
use time::{Duration, OffsetDateTime};
use uuid::Uuid;

use crate::{
    content::provider::ProviderSecretKeyring,
    db::entities::{
        backup_job, backup_job_target, backup_schedule, backup_schedule_target, backup_target,
    },
};

use super::{
    BackupError, BackupErrorKind, BackupJob, BackupJobStatus, BackupJobTarget,
    BackupJobTargetStatus, BackupPublicConfig, BackupSchedule, BackupSecretConfig, BackupTarget,
    BackupTargetKind, BackupTriggerKind, CreateBackupTarget, RetentionPolicy, UpdateBackupTarget,
    model::{normalize_display_name, validate_id},
};

const MAX_TARGETS_PER_USER: u64 = 32;
const MAX_HISTORY_ITEMS: u64 = 100;
const HISTORY_DAYS: i64 = 7;

#[derive(Clone)]
pub struct BackupRepository {
    database: DatabaseConnection,
    keyring: Option<Arc<ProviderSecretKeyring>>,
}

#[derive(Clone, Debug)]
pub struct BackupClaim {
    pub job_id: String,
    pub user_id: String,
    pub owner: String,
    pub lease_token: i64,
}

#[derive(Clone, Debug)]
pub struct ExecutionTarget {
    pub target_result_id: String,
    pub target_id: String,
    pub display_name: String,
    pub config: BackupPublicConfig,
    pub secret: BackupSecretConfig,
    pub retention: RetentionPolicy,
    pub object_key: String,
}

impl BackupRepository {
    #[must_use]
    pub fn new(database: DatabaseConnection, keyring: Option<Arc<ProviderSecretKeyring>>) -> Self {
        Self { database, keyring }
    }

    pub async fn list_targets(&self, user_id: &str) -> Result<Vec<BackupTarget>, BackupError> {
        validate_id(user_id)?;
        backup_target::Entity::find()
            .filter(backup_target::Column::UserId.eq(user_id))
            .order_by_asc(backup_target::Column::Kind)
            .order_by_asc(backup_target::Column::DisplayName)
            .all(&self.database)
            .await
            .map_err(database_error)?
            .into_iter()
            .map(target_from_model)
            .collect()
    }

    pub async fn create_target(
        &self,
        user_id: &str,
        input: CreateBackupTarget,
    ) -> Result<BackupTarget, BackupError> {
        validate_id(user_id)?;
        let display_name = normalize_display_name(&input.display_name)?;
        let config = input.config.validate_and_normalize()?;
        if config.kind() != input.secret.kind() {
            return Err(BackupError::new(BackupErrorKind::InvalidInput));
        }
        input.secret.validate()?;
        let retention = input.retention.validate()?;
        if backup_target::Entity::find()
            .filter(backup_target::Column::UserId.eq(user_id))
            .count(&self.database)
            .await
            .map_err(database_error)?
            >= MAX_TARGETS_PER_USER
        {
            return Err(BackupError::new(BackupErrorKind::Conflict));
        }
        let id = Uuid::new_v4().to_string();
        let ciphertext = self.encrypt_secret(&id, &input.secret)?;
        let now = OffsetDateTime::now_utc();
        let stored = backup_target::ActiveModel {
            id: Set(id),
            user_id: Set(user_id.to_owned()),
            kind: Set(config.kind().as_storage().to_owned()),
            display_name: Set(display_name),
            enabled: Set(input.enabled),
            public_config_json: Set(serialize_config(&config)?),
            secret_config_ciphertext: Set(ciphertext),
            retain_count: Set(retention.retain_count.map(i32::from)),
            retain_days: Set(retention.retain_days.map(i32::from)),
            revision: Set(1),
            created_at: Set(now),
            updated_at: Set(now),
        }
        .insert(&self.database)
        .await
        .map_err(conflict_or_database)?;
        target_from_model(stored)
    }

    pub async fn update_target(
        &self,
        user_id: &str,
        target_id: &str,
        input: UpdateBackupTarget,
    ) -> Result<BackupTarget, BackupError> {
        validate_id(user_id)?;
        validate_id(target_id)?;
        let display_name = normalize_display_name(&input.display_name)?;
        let config = input.config.validate_and_normalize()?;
        if input
            .secret
            .as_ref()
            .is_some_and(|secret| secret.kind() != config.kind())
        {
            return Err(BackupError::new(BackupErrorKind::InvalidInput));
        }
        if let Some(secret) = &input.secret {
            secret.validate()?;
        }
        let retention = input.retention.validate()?;
        let stored = backup_target::Entity::find_by_id(target_id)
            .filter(backup_target::Column::UserId.eq(user_id))
            .one(&self.database)
            .await
            .map_err(database_error)?
            .ok_or_else(not_found)?;
        if BackupTargetKind::parse_storage(&stored.kind)? != config.kind() {
            return Err(BackupError::new(BackupErrorKind::InvalidInput));
        }
        if stored.revision == i64::MAX {
            return Err(BackupError::new(BackupErrorKind::Conflict));
        }
        let mut active: backup_target::ActiveModel = stored.into();
        active.display_name = Set(display_name);
        active.enabled = Set(input.enabled);
        active.public_config_json = Set(serialize_config(&config)?);
        active.retain_count = Set(retention.retain_count.map(i32::from));
        active.retain_days = Set(retention.retain_days.map(i32::from));
        active.revision = Set(active.revision.as_ref() + 1);
        active.updated_at = Set(OffsetDateTime::now_utc());
        if let Some(secret) = &input.secret {
            active.secret_config_ciphertext = Set(self.encrypt_secret(target_id, secret)?);
        }
        active
            .update(&self.database)
            .await
            .map_err(conflict_or_database)
            .and_then(target_from_model)
    }

    pub async fn delete_target(&self, user_id: &str, target_id: &str) -> Result<(), BackupError> {
        validate_id(user_id)?;
        validate_id(target_id)?;
        backup_target::Entity::delete_many()
            .filter(backup_target::Column::Id.eq(target_id))
            .filter(backup_target::Column::UserId.eq(user_id))
            .exec(&self.database)
            .await
            .map_err(database_error)?;
        Ok(())
    }

    pub async fn get_schedule(&self, user_id: &str) -> Result<BackupSchedule, BackupError> {
        validate_id(user_id)?;
        let Some(stored) = backup_schedule::Entity::find_by_id(user_id)
            .one(&self.database)
            .await
            .map_err(database_error)?
        else {
            return Ok(BackupSchedule::default());
        };
        let target_ids = backup_schedule_target::Entity::find()
            .filter(backup_schedule_target::Column::UserId.eq(user_id))
            .order_by_asc(backup_schedule_target::Column::TargetId)
            .all(&self.database)
            .await
            .map_err(database_error)?
            .into_iter()
            .map(|row| row.target_id)
            .collect();
        schedule_from_model(stored, target_ids)
    }

    pub async fn put_schedule(
        &self,
        user_id: &str,
        enabled: bool,
        interval_hours: u16,
        target_ids: &[String],
    ) -> Result<BackupSchedule, BackupError> {
        validate_id(user_id)?;
        if !(1..=720).contains(&interval_hours) {
            return Err(BackupError::new(BackupErrorKind::InvalidInput));
        }
        let target_ids = validate_target_ids(target_ids, enabled)?;
        let transaction = self.database.begin().await.map_err(database_error)?;
        let result = async {
            let targets = backup_target::Entity::find()
                .filter(backup_target::Column::UserId.eq(user_id))
                .filter(backup_target::Column::Id.is_in(target_ids.iter().cloned()))
                .all(&transaction)
                .await
                .map_err(database_error)?;
            if targets.len() != target_ids.len() || targets.iter().any(|target| !target.enabled) {
                return Err(BackupError::new(BackupErrorKind::InvalidInput));
            }
            let now = OffsetDateTime::now_utc();
            let stored = backup_schedule::Entity::find_by_id(user_id)
                .one(&transaction)
                .await
                .map_err(database_error)?;
            let schedule = if let Some(stored) = stored {
                if stored.revision == i64::MAX {
                    return Err(BackupError::new(BackupErrorKind::Conflict));
                }
                let mut active: backup_schedule::ActiveModel = stored.into();
                active.enabled = Set(enabled);
                active.interval_hours = Set(i32::from(interval_hours));
                active.next_run_at =
                    Set(enabled.then_some(now + Duration::hours(i64::from(interval_hours))));
                active.revision = Set(active.revision.as_ref() + 1);
                active.updated_at = Set(now);
                active.update(&transaction).await.map_err(database_error)?
            } else {
                backup_schedule::ActiveModel {
                    user_id: Set(user_id.to_owned()),
                    enabled: Set(enabled),
                    interval_hours: Set(i32::from(interval_hours)),
                    next_run_at: Set(
                        enabled.then_some(now + Duration::hours(i64::from(interval_hours)))
                    ),
                    revision: Set(1),
                    created_at: Set(now),
                    updated_at: Set(now),
                }
                .insert(&transaction)
                .await
                .map_err(database_error)?
            };
            backup_schedule_target::Entity::delete_many()
                .filter(backup_schedule_target::Column::UserId.eq(user_id))
                .exec(&transaction)
                .await
                .map_err(database_error)?;
            for target_id in &target_ids {
                backup_schedule_target::ActiveModel {
                    user_id: Set(user_id.to_owned()),
                    target_id: Set(target_id.clone()),
                }
                .insert(&transaction)
                .await
                .map_err(database_error)?;
            }
            schedule_from_model(schedule, target_ids)
        }
        .await;
        finish_transaction(transaction, result).await
    }

    pub async fn enqueue_manual(
        &self,
        user_id: &str,
        target_ids: &[String],
    ) -> Result<BackupJob, BackupError> {
        validate_id(user_id)?;
        let target_ids = validate_target_ids(target_ids, true)?;
        let targets = self.owned_enabled_targets(user_id, &target_ids).await?;
        self.insert_job(user_id, BackupTriggerKind::Manual, None, targets)
            .await
    }

    pub async fn enqueue_due_schedules(&self) -> Result<usize, BackupError> {
        let now = OffsetDateTime::now_utc();
        let due = backup_schedule::Entity::find()
            .filter(backup_schedule::Column::Enabled.eq(true))
            .filter(backup_schedule::Column::NextRunAt.lte(now))
            .order_by_asc(backup_schedule::Column::NextRunAt)
            .limit(32)
            .all(&self.database)
            .await
            .map_err(database_error)?;
        let mut enqueued = 0;
        for schedule in due {
            if self
                .enqueue_due_schedule(&schedule.user_id)
                .await?
                .is_some()
            {
                enqueued += 1;
            }
        }
        Ok(enqueued)
    }

    async fn enqueue_due_schedule(&self, user_id: &str) -> Result<Option<BackupJob>, BackupError> {
        let transaction = self.database.begin().await.map_err(database_error)?;
        let result = async {
            let now = OffsetDateTime::now_utc();
            let Some(stored) = backup_schedule::Entity::find_by_id(user_id)
                .one(&transaction)
                .await
                .map_err(database_error)?
            else {
                return Ok(None);
            };
            let Some(slot) = stored.next_run_at else {
                return Ok(None);
            };
            if !stored.enabled || slot > now || !(1..=720).contains(&stored.interval_hours) {
                return Ok(None);
            }
            let selected = backup_schedule_target::Entity::find()
                .filter(backup_schedule_target::Column::UserId.eq(user_id))
                .all(&transaction)
                .await
                .map_err(database_error)?;
            let target_ids: Vec<_> = selected.into_iter().map(|row| row.target_id).collect();
            let targets = backup_target::Entity::find()
                .filter(backup_target::Column::UserId.eq(user_id))
                .filter(backup_target::Column::Enabled.eq(true))
                .filter(backup_target::Column::Id.is_in(target_ids))
                .all(&transaction)
                .await
                .map_err(database_error)?;
            let mut active: backup_schedule::ActiveModel = stored.into();
            active.next_run_at = Set(Some(next_schedule_slot(
                slot,
                now,
                *active.interval_hours.as_ref(),
            )?));
            active.updated_at = Set(now);
            active.update(&transaction).await.map_err(database_error)?;
            if targets.is_empty() {
                return Ok(None);
            }
            let job = insert_job_rows(
                &transaction,
                user_id,
                BackupTriggerKind::Scheduled,
                Some(slot),
                targets,
                now,
            )
            .await?;
            Ok(Some(job))
        }
        .await;
        finish_transaction(transaction, result).await
    }

    async fn insert_job(
        &self,
        user_id: &str,
        trigger: BackupTriggerKind,
        scheduled_for: Option<OffsetDateTime>,
        targets: Vec<backup_target::Model>,
    ) -> Result<BackupJob, BackupError> {
        let transaction = self.database.begin().await.map_err(database_error)?;
        let result = insert_job_rows(
            &transaction,
            user_id,
            trigger,
            scheduled_for,
            targets,
            OffsetDateTime::now_utc(),
        )
        .await;
        finish_transaction(transaction, result).await
    }

    pub async fn claim_next(&self, owner: &str) -> Result<Option<BackupClaim>, BackupError> {
        if owner.is_empty() || owner.len() > 64 {
            return Err(BackupError::new(BackupErrorKind::InvalidInput));
        }
        let now = OffsetDateTime::now_utc();
        backup_job::Entity::update_many()
            .col_expr(backup_job::Column::Status, Expr::value("QUEUED"))
            .col_expr(
                backup_job::Column::LeaseOwner,
                Expr::value(Option::<String>::None),
            )
            .col_expr(
                backup_job::Column::LeaseUntil,
                Expr::value(Option::<OffsetDateTime>::None),
            )
            .filter(backup_job::Column::Status.eq("RUNNING"))
            .filter(backup_job::Column::LeaseUntil.lte(now))
            .exec(&self.database)
            .await
            .map_err(database_error)?;
        let candidates = backup_job::Entity::find()
            .filter(backup_job::Column::Status.eq("QUEUED"))
            .order_by_asc(backup_job::Column::CreatedAt)
            .limit(16)
            .all(&self.database)
            .await
            .map_err(database_error)?;
        for candidate in candidates {
            if candidate.lease_token < 0 || candidate.lease_token == i64::MAX {
                continue;
            }
            let token = candidate.lease_token + 1;
            let lease_until = now + Duration::seconds(30);
            let updated = backup_job::Entity::update_many()
                .col_expr(backup_job::Column::Status, Expr::value("RUNNING"))
                .col_expr(
                    backup_job::Column::LeaseOwner,
                    Expr::value(owner.to_owned()),
                )
                .col_expr(backup_job::Column::LeaseToken, Expr::value(token))
                .col_expr(backup_job::Column::LeaseUntil, Expr::value(lease_until))
                .col_expr(
                    backup_job::Column::StartedAt,
                    Expr::value(candidate.started_at.or(Some(now))),
                )
                .filter(backup_job::Column::Id.eq(&candidate.id))
                .filter(backup_job::Column::Status.eq("QUEUED"))
                .filter(backup_job::Column::LeaseToken.eq(candidate.lease_token))
                .exec(&self.database)
                .await
                .map_err(database_error)?;
            if updated.rows_affected == 1 {
                return Ok(Some(BackupClaim {
                    job_id: candidate.id,
                    user_id: candidate.user_id,
                    owner: owner.to_owned(),
                    lease_token: token,
                }));
            }
        }
        Ok(None)
    }

    pub async fn heartbeat(&self, claim: &BackupClaim) -> Result<(), BackupError> {
        let now = OffsetDateTime::now_utc();
        let updated = backup_job::Entity::update_many()
            .col_expr(
                backup_job::Column::LeaseUntil,
                Expr::value(now + Duration::seconds(30)),
            )
            .filter(backup_job::Column::Id.eq(&claim.job_id))
            .filter(backup_job::Column::UserId.eq(&claim.user_id))
            .filter(backup_job::Column::Status.eq("RUNNING"))
            .filter(backup_job::Column::LeaseOwner.eq(&claim.owner))
            .filter(backup_job::Column::LeaseToken.eq(claim.lease_token))
            .filter(backup_job::Column::LeaseUntil.gt(now))
            .exec(&self.database)
            .await
            .map_err(database_error)?;
        if updated.rows_affected == 1 {
            Ok(())
        } else {
            Err(BackupError::new(BackupErrorKind::LeaseLost))
        }
    }

    pub async fn pending_execution_targets(
        &self,
        claim: &BackupClaim,
    ) -> Result<Vec<ExecutionTarget>, BackupError> {
        self.ensure_active_claim(claim).await?;
        let rows = backup_job_target::Entity::find()
            .filter(backup_job_target::Column::JobId.eq(&claim.job_id))
            .filter(backup_job_target::Column::Status.ne("SUCCEEDED"))
            .order_by_asc(backup_job_target::Column::TargetName)
            .all(&self.database)
            .await
            .map_err(database_error)?;
        let mut result = Vec::with_capacity(rows.len());
        for row in rows {
            let Some(target_id) = row.target_id.clone() else {
                self.complete_target(
                    claim,
                    &row.id,
                    Err(BackupError::new(BackupErrorKind::TargetChanged)),
                )
                .await?;
                continue;
            };
            let stored = backup_target::Entity::find_by_id(&target_id)
                .filter(backup_target::Column::UserId.eq(&claim.user_id))
                .one(&self.database)
                .await
                .map_err(database_error)?;
            let Some(stored) = stored else {
                self.complete_target(
                    claim,
                    &row.id,
                    Err(BackupError::new(BackupErrorKind::TargetChanged)),
                )
                .await?;
                continue;
            };
            if !stored.enabled
                || stored.revision != row.target_revision
                || stored.kind != row.target_kind
            {
                self.complete_target(
                    claim,
                    &row.id,
                    Err(BackupError::new(BackupErrorKind::TargetChanged)),
                )
                .await?;
                continue;
            }
            let kind = BackupTargetKind::parse_storage(&stored.kind)?;
            let config = deserialize_config(&stored.public_config_json)?;
            let secret =
                match self.decrypt_secret(&stored.id, kind, &stored.secret_config_ciphertext) {
                    Ok(secret) => secret,
                    Err(error) => {
                        self.complete_target(claim, &row.id, Err(error)).await?;
                        continue;
                    }
                };
            result.push(ExecutionTarget {
                target_result_id: row.id,
                target_id: stored.id,
                display_name: stored.display_name,
                config,
                secret,
                retention: retention_from_model(stored.retain_count, stored.retain_days)?,
                object_key: row.object_key,
            });
        }
        Ok(result)
    }

    pub async fn execution_target_for_test(
        &self,
        user_id: &str,
        target_id: &str,
    ) -> Result<ExecutionTarget, BackupError> {
        validate_id(user_id)?;
        validate_id(target_id)?;
        let stored = backup_target::Entity::find_by_id(target_id)
            .filter(backup_target::Column::UserId.eq(user_id))
            .one(&self.database)
            .await
            .map_err(database_error)?
            .ok_or_else(not_found)?;
        let kind = BackupTargetKind::parse_storage(&stored.kind)?;
        Ok(ExecutionTarget {
            target_result_id: Uuid::nil().to_string(),
            target_id: stored.id.clone(),
            display_name: stored.display_name.clone(),
            config: deserialize_config(&stored.public_config_json)?,
            secret: self.decrypt_secret(&stored.id, kind, &stored.secret_config_ciphertext)?,
            retention: retention_from_model(stored.retain_count, stored.retain_days)?,
            object_key: String::new(),
        })
    }

    pub async fn mark_target_running(
        &self,
        claim: &BackupClaim,
        target_result_id: &str,
    ) -> Result<(), BackupError> {
        let transaction = self.database.begin().await.map_err(database_error)?;
        let result = async {
            lock_active_claim(&transaction, self.database.get_database_backend(), claim).await?;
            let now = OffsetDateTime::now_utc();
            let updated = backup_job_target::Entity::update_many()
                .col_expr(backup_job_target::Column::Status, Expr::value("RUNNING"))
                .col_expr(backup_job_target::Column::StartedAt, Expr::value(now))
                .col_expr(
                    backup_job_target::Column::CompletedAt,
                    Expr::value(Option::<OffsetDateTime>::None),
                )
                .col_expr(
                    backup_job_target::Column::ErrorCode,
                    Expr::value(Option::<String>::None),
                )
                .filter(backup_job_target::Column::Id.eq(target_result_id))
                .filter(backup_job_target::Column::JobId.eq(&claim.job_id))
                .exec(&transaction)
                .await
                .map_err(database_error)?;
            if updated.rows_affected == 1 {
                Ok(())
            } else {
                Err(not_found())
            }
        }
        .await;
        finish_transaction(transaction, result).await
    }

    pub async fn complete_target(
        &self,
        claim: &BackupClaim,
        target_result_id: &str,
        result: Result<u64, BackupError>,
    ) -> Result<(), BackupError> {
        let transaction = self.database.begin().await.map_err(database_error)?;
        let result = async {
            lock_active_claim(&transaction, self.database.get_database_backend(), claim).await?;
            let now = OffsetDateTime::now_utc();
            let (status, byte_size, error_code) = match result {
                Ok(bytes) => (
                    BackupJobTargetStatus::Succeeded,
                    Some(
                        i64::try_from(bytes)
                            .map_err(|_| BackupError::new(BackupErrorKind::CorruptData))?,
                    ),
                    None,
                ),
                Err(error) => (
                    BackupJobTargetStatus::Failed,
                    None,
                    Some(error.public_code().to_owned()),
                ),
            };
            let updated = backup_job_target::Entity::update_many()
                .col_expr(
                    backup_job_target::Column::Status,
                    Expr::value(status.as_storage()),
                )
                .col_expr(backup_job_target::Column::ByteSize, Expr::value(byte_size))
                .col_expr(
                    backup_job_target::Column::ErrorCode,
                    Expr::value(error_code),
                )
                .col_expr(backup_job_target::Column::CompletedAt, Expr::value(now))
                .filter(backup_job_target::Column::Id.eq(target_result_id))
                .filter(backup_job_target::Column::JobId.eq(&claim.job_id))
                .exec(&transaction)
                .await
                .map_err(database_error)?;
            if updated.rows_affected == 1 {
                Ok(())
            } else {
                Err(not_found())
            }
        }
        .await;
        finish_transaction(transaction, result).await
    }

    pub async fn fail_pending_targets(
        &self,
        claim: &BackupClaim,
        error: &BackupError,
    ) -> Result<(), BackupError> {
        let transaction = self.database.begin().await.map_err(database_error)?;
        let result = async {
            lock_active_claim(&transaction, self.database.get_database_backend(), claim).await?;
            let now = OffsetDateTime::now_utc();
            backup_job_target::Entity::update_many()
                .col_expr(
                    backup_job_target::Column::Status,
                    Expr::value(BackupJobTargetStatus::Failed.as_storage()),
                )
                .col_expr(
                    backup_job_target::Column::ErrorCode,
                    Expr::value(error.public_code()),
                )
                .col_expr(backup_job_target::Column::CompletedAt, Expr::value(now))
                .filter(backup_job_target::Column::JobId.eq(&claim.job_id))
                .filter(backup_job_target::Column::Status.ne("SUCCEEDED"))
                .exec(&transaction)
                .await
                .map_err(database_error)?;
            Ok(())
        }
        .await;
        finish_transaction(transaction, result).await
    }

    pub async fn finish_job(&self, claim: &BackupClaim) -> Result<BackupJob, BackupError> {
        self.ensure_active_claim(claim).await?;
        let rows = backup_job_target::Entity::find()
            .filter(backup_job_target::Column::JobId.eq(&claim.job_id))
            .all(&self.database)
            .await
            .map_err(database_error)?;
        let succeeded = rows.iter().filter(|row| row.status == "SUCCEEDED").count();
        let failed = rows.iter().filter(|row| row.status == "FAILED").count();
        let status = if succeeded == rows.len() {
            BackupJobStatus::Succeeded
        } else if succeeded > 0 && failed > 0 {
            BackupJobStatus::Partial
        } else {
            BackupJobStatus::Failed
        };
        let last_error = (status != BackupJobStatus::Succeeded).then(|| {
            if succeeded > 0 {
                "PARTIAL_FAILURE"
            } else {
                "ALL_TARGETS_FAILED"
            }
            .to_owned()
        });
        let now = OffsetDateTime::now_utc();
        let updated = backup_job::Entity::update_many()
            .col_expr(backup_job::Column::Status, Expr::value(status.as_storage()))
            .col_expr(backup_job::Column::LastErrorCode, Expr::value(last_error))
            .col_expr(backup_job::Column::CompletedAt, Expr::value(now))
            .col_expr(
                backup_job::Column::LeaseOwner,
                Expr::value(Option::<String>::None),
            )
            .col_expr(
                backup_job::Column::LeaseUntil,
                Expr::value(Option::<OffsetDateTime>::None),
            )
            .filter(backup_job::Column::Id.eq(&claim.job_id))
            .filter(backup_job::Column::UserId.eq(&claim.user_id))
            .filter(backup_job::Column::Status.eq("RUNNING"))
            .filter(backup_job::Column::LeaseOwner.eq(&claim.owner))
            .filter(backup_job::Column::LeaseToken.eq(claim.lease_token))
            .exec(&self.database)
            .await
            .map_err(database_error)?;
        if updated.rows_affected != 1 {
            return Err(BackupError::new(BackupErrorKind::LeaseLost));
        }
        self.get_job(&claim.user_id, &claim.job_id).await
    }

    pub async fn get_job(&self, user_id: &str, job_id: &str) -> Result<BackupJob, BackupError> {
        validate_id(user_id)?;
        validate_id(job_id)?;
        let stored = backup_job::Entity::find_by_id(job_id)
            .filter(backup_job::Column::UserId.eq(user_id))
            .one(&self.database)
            .await
            .map_err(database_error)?
            .ok_or_else(not_found)?;
        job_from_model(stored, self.job_targets(job_id).await?)
    }

    pub async fn list_jobs(
        &self,
        user_id: &str,
        since: Option<OffsetDateTime>,
        limit: u16,
    ) -> Result<Vec<BackupJob>, BackupError> {
        validate_id(user_id)?;
        if !(1..=100).contains(&limit) {
            return Err(BackupError::new(BackupErrorKind::InvalidInput));
        }
        let floor = OffsetDateTime::now_utc() - Duration::days(HISTORY_DAYS);
        let since = since.map_or(floor, |value| value.max(floor));
        let rows = backup_job::Entity::find()
            .filter(backup_job::Column::UserId.eq(user_id))
            .filter(backup_job::Column::CreatedAt.gte(since))
            .order_by_desc(backup_job::Column::CreatedAt)
            .order_by_desc(backup_job::Column::Id)
            .limit(u64::from(limit).min(MAX_HISTORY_ITEMS))
            .all(&self.database)
            .await
            .map_err(database_error)?;
        let mut jobs = Vec::with_capacity(rows.len());
        for row in rows {
            let targets = self.job_targets(&row.id).await?;
            jobs.push(job_from_model(row, targets)?);
        }
        Ok(jobs)
    }

    pub async fn cleanup_history(&self) -> Result<u64, BackupError> {
        backup_job::Entity::delete_many()
            .filter(
                backup_job::Column::CreatedAt
                    .lt(OffsetDateTime::now_utc() - Duration::days(HISTORY_DAYS)),
            )
            .filter(backup_job::Column::Status.is_in(["SUCCEEDED", "PARTIAL", "FAILED"]))
            .exec(&self.database)
            .await
            .map(|result| result.rows_affected)
            .map_err(database_error)
    }

    async fn owned_enabled_targets(
        &self,
        user_id: &str,
        target_ids: &[String],
    ) -> Result<Vec<backup_target::Model>, BackupError> {
        let targets = backup_target::Entity::find()
            .filter(backup_target::Column::UserId.eq(user_id))
            .filter(backup_target::Column::Enabled.eq(true))
            .filter(backup_target::Column::Id.is_in(target_ids.iter().cloned()))
            .all(&self.database)
            .await
            .map_err(database_error)?;
        if targets.len() != target_ids.len() {
            return Err(BackupError::new(BackupErrorKind::InvalidInput));
        }
        Ok(targets)
    }

    async fn ensure_active_claim(&self, claim: &BackupClaim) -> Result<(), BackupError> {
        let now = OffsetDateTime::now_utc();
        let active = backup_job::Entity::find_by_id(&claim.job_id)
            .filter(backup_job::Column::UserId.eq(&claim.user_id))
            .filter(backup_job::Column::Status.eq("RUNNING"))
            .filter(backup_job::Column::LeaseOwner.eq(&claim.owner))
            .filter(backup_job::Column::LeaseToken.eq(claim.lease_token))
            .filter(backup_job::Column::LeaseUntil.gt(now))
            .one(&self.database)
            .await
            .map_err(database_error)?;
        active
            .is_some()
            .then_some(())
            .ok_or_else(|| BackupError::new(BackupErrorKind::LeaseLost))
    }

    async fn job_targets(&self, job_id: &str) -> Result<Vec<BackupJobTarget>, BackupError> {
        backup_job_target::Entity::find()
            .filter(backup_job_target::Column::JobId.eq(job_id))
            .order_by_asc(backup_job_target::Column::TargetName)
            .all(&self.database)
            .await
            .map_err(database_error)?
            .into_iter()
            .map(job_target_from_model)
            .collect()
    }

    fn encrypt_secret(
        &self,
        target_id: &str,
        secret: &BackupSecretConfig,
    ) -> Result<String, BackupError> {
        let json = secret.to_secret_json()?;
        self.keyring
            .as_ref()
            .ok_or_else(|| BackupError::new(BackupErrorKind::SecretUnavailable))?
            .encrypt_scoped(target_id, secret_purpose(secret.kind()), &json)
            .map_err(|_| BackupError::new(BackupErrorKind::SecretUnavailable))
    }

    fn decrypt_secret(
        &self,
        target_id: &str,
        kind: BackupTargetKind,
        ciphertext: &str,
    ) -> Result<BackupSecretConfig, BackupError> {
        let json = self
            .keyring
            .as_ref()
            .ok_or_else(|| BackupError::new(BackupErrorKind::SecretUnavailable))?
            .decrypt_scoped(target_id, secret_purpose(kind), ciphertext)
            .map_err(|_| BackupError::new(BackupErrorKind::SecretUnavailable))?;
        BackupSecretConfig::from_secret_json(kind, json)
    }
}

async fn insert_job_rows<C>(
    connection: &C,
    user_id: &str,
    trigger: BackupTriggerKind,
    scheduled_for: Option<OffsetDateTime>,
    targets: Vec<backup_target::Model>,
    now: OffsetDateTime,
) -> Result<BackupJob, BackupError>
where
    C: sea_orm::ConnectionTrait,
{
    let target_count =
        u16::try_from(targets.len()).map_err(|_| BackupError::new(BackupErrorKind::CorruptData))?;
    if target_count == 0 {
        return Err(BackupError::new(BackupErrorKind::InvalidInput));
    }
    let job_id = Uuid::new_v4().to_string();
    let stored = backup_job::ActiveModel {
        id: Set(job_id.clone()),
        user_id: Set(user_id.to_owned()),
        trigger_kind: Set(trigger.as_storage().to_owned()),
        scheduled_for: Set(scheduled_for),
        status: Set(BackupJobStatus::Queued.as_storage().to_owned()),
        target_count: Set(i32::from(target_count)),
        lease_owner: Set(None),
        lease_token: Set(0),
        lease_until: Set(None),
        last_error_code: Set(None),
        created_at: Set(now),
        started_at: Set(None),
        completed_at: Set(None),
    }
    .insert(connection)
    .await
    .map_err(conflict_or_database)?;
    let mut result_targets = Vec::with_capacity(targets.len());
    for target in targets {
        let config = deserialize_config(&target.public_config_json)?;
        let result_id = Uuid::new_v4().to_string();
        let result = backup_job_target::ActiveModel {
            id: Set(result_id),
            job_id: Set(job_id.clone()),
            target_id: Set(Some(target.id.clone())),
            target_kind: Set(target.kind.clone()),
            target_name: Set(target.display_name.clone()),
            target_revision: Set(target.revision),
            object_key: Set(object_key(user_id, &job_id, now, config.prefix())),
            status: Set(BackupJobTargetStatus::Queued.as_storage().to_owned()),
            byte_size: Set(None),
            error_code: Set(None),
            started_at: Set(None),
            completed_at: Set(None),
        }
        .insert(connection)
        .await
        .map_err(database_error)?;
        result_targets.push(job_target_from_model(result)?);
    }
    job_from_model(stored, result_targets)
}

fn object_key(user_id: &str, job_id: &str, now: OffsetDateTime, prefix: &str) -> String {
    let user_hash = blake3::hash(user_id.as_bytes()).to_hex();
    let timestamp = format!(
        "{:04}{:02}{:02}T{:02}{:02}{:02}Z",
        now.year(),
        u8::from(now.month()),
        now.day(),
        now.hour(),
        now.minute(),
        now.second()
    );
    let owned = format!(
        "raindrop/{}/subscriptions/raindrop-subscriptions-{timestamp}-{job_id}.opml",
        &user_hash[..24]
    );
    if prefix.is_empty() {
        owned
    } else {
        format!("{prefix}/{owned}")
    }
}

fn next_schedule_slot(
    slot: OffsetDateTime,
    now: OffsetDateTime,
    interval_hours: i32,
) -> Result<OffsetDateTime, BackupError> {
    let seconds = i64::from(interval_hours)
        .checked_mul(3_600)
        .ok_or_else(|| BackupError::new(BackupErrorKind::CorruptData))?;
    if seconds <= 0 {
        return Err(BackupError::new(BackupErrorKind::CorruptData));
    }
    let elapsed = (now - slot).whole_seconds().max(0);
    let steps = elapsed / seconds + 1;
    slot.checked_add(Duration::seconds(
        seconds
            .checked_mul(steps)
            .ok_or_else(|| BackupError::new(BackupErrorKind::CorruptData))?,
    ))
    .ok_or_else(|| BackupError::new(BackupErrorKind::CorruptData))
}

fn validate_target_ids(
    target_ids: &[String],
    require_nonempty: bool,
) -> Result<Vec<String>, BackupError> {
    if target_ids.len() > MAX_TARGETS_PER_USER as usize
        || (require_nonempty && target_ids.is_empty())
    {
        return Err(BackupError::new(BackupErrorKind::InvalidInput));
    }
    let mut unique = HashSet::with_capacity(target_ids.len());
    for id in target_ids {
        validate_id(id)?;
        if !unique.insert(id.clone()) {
            return Err(BackupError::new(BackupErrorKind::InvalidInput));
        }
    }
    let mut ids: Vec<_> = unique.into_iter().collect();
    ids.sort();
    Ok(ids)
}

fn target_from_model(stored: backup_target::Model) -> Result<BackupTarget, BackupError> {
    let config = deserialize_config(&stored.public_config_json)?;
    if BackupTargetKind::parse_storage(&stored.kind)? != config.kind() || stored.revision <= 0 {
        return Err(BackupError::new(BackupErrorKind::CorruptData));
    }
    Ok(BackupTarget {
        target_id: stored.id,
        display_name: stored.display_name,
        enabled: stored.enabled,
        config,
        retention: retention_from_model(stored.retain_count, stored.retain_days)?,
        revision: stored.revision,
        has_credentials: !stored.secret_config_ciphertext.is_empty(),
        created_at: stored.created_at,
        updated_at: stored.updated_at,
    })
}

fn schedule_from_model(
    stored: backup_schedule::Model,
    target_ids: Vec<String>,
) -> Result<BackupSchedule, BackupError> {
    Ok(BackupSchedule {
        enabled: stored.enabled,
        interval_hours: u16::try_from(stored.interval_hours)
            .ok()
            .filter(|value| (1..=720).contains(value))
            .ok_or_else(|| BackupError::new(BackupErrorKind::CorruptData))?,
        target_ids,
        next_run_at: stored.next_run_at,
        revision: stored.revision,
    })
}

fn job_from_model(
    stored: backup_job::Model,
    targets: Vec<BackupJobTarget>,
) -> Result<BackupJob, BackupError> {
    Ok(BackupJob {
        job_id: stored.id,
        trigger_kind: BackupTriggerKind::parse_storage(&stored.trigger_kind)?,
        status: BackupJobStatus::parse_storage(&stored.status)?,
        target_count: u16::try_from(stored.target_count)
            .map_err(|_| BackupError::new(BackupErrorKind::CorruptData))?,
        last_error_code: stored.last_error_code,
        created_at: stored.created_at,
        started_at: stored.started_at,
        completed_at: stored.completed_at,
        targets,
    })
}

fn job_target_from_model(stored: backup_job_target::Model) -> Result<BackupJobTarget, BackupError> {
    Ok(BackupJobTarget {
        target_result_id: stored.id,
        target_id: stored.target_id,
        target_kind: BackupTargetKind::parse_storage(&stored.target_kind)?,
        target_name: stored.target_name,
        status: BackupJobTargetStatus::parse_storage(&stored.status)?,
        byte_size: stored
            .byte_size
            .map(u64::try_from)
            .transpose()
            .map_err(|_| BackupError::new(BackupErrorKind::CorruptData))?,
        error_code: stored.error_code,
        started_at: stored.started_at,
        completed_at: stored.completed_at,
    })
}

fn retention_from_model(
    retain_count: Option<i32>,
    retain_days: Option<i32>,
) -> Result<RetentionPolicy, BackupError> {
    RetentionPolicy {
        retain_count: retain_count
            .map(u16::try_from)
            .transpose()
            .map_err(|_| BackupError::new(BackupErrorKind::CorruptData))?,
        retain_days: retain_days
            .map(u16::try_from)
            .transpose()
            .map_err(|_| BackupError::new(BackupErrorKind::CorruptData))?,
    }
    .validate()
}

fn serialize_config(config: &BackupPublicConfig) -> Result<String, BackupError> {
    serde_json::to_string(config).map_err(|_| BackupError::new(BackupErrorKind::InvalidInput))
}

fn deserialize_config(value: &str) -> Result<BackupPublicConfig, BackupError> {
    serde_json::from_str::<BackupPublicConfig>(value)
        .map_err(|_| BackupError::new(BackupErrorKind::CorruptData))?
        .validate_and_normalize()
        .map_err(|_| BackupError::new(BackupErrorKind::CorruptData))
}

const fn secret_purpose(kind: BackupTargetKind) -> &'static str {
    match kind {
        BackupTargetKind::S3 => "backup-target:S3",
        BackupTargetKind::Webdav => "backup-target:WEBDAV",
    }
}

async fn lock_active_claim<C>(
    connection: &C,
    backend: DatabaseBackend,
    claim: &BackupClaim,
) -> Result<(), BackupError>
where
    C: ConnectionTrait,
{
    if backend == DatabaseBackend::Sqlite {
        let locked = connection
            .execute(Statement::from_sql_and_values(
                backend,
                "UPDATE backup_jobs SET lease_token = lease_token WHERE id = ?",
                [claim.job_id.clone().into()],
            ))
            .await
            .map_err(database_error)?;
        if locked.rows_affected() != 1 {
            return Err(BackupError::new(BackupErrorKind::LeaseLost));
        }
    } else {
        let sql = if backend == DatabaseBackend::Postgres {
            "SELECT id FROM backup_jobs WHERE id = $1 FOR UPDATE"
        } else {
            "SELECT id FROM backup_jobs WHERE id = ? FOR UPDATE"
        };
        if connection
            .query_one(Statement::from_sql_and_values(
                backend,
                sql,
                [claim.job_id.clone().into()],
            ))
            .await
            .map_err(database_error)?
            .is_none()
        {
            return Err(BackupError::new(BackupErrorKind::LeaseLost));
        }
    }

    let now = OffsetDateTime::now_utc();
    let active = backup_job::Entity::find_by_id(&claim.job_id)
        .filter(backup_job::Column::UserId.eq(&claim.user_id))
        .filter(backup_job::Column::Status.eq("RUNNING"))
        .filter(backup_job::Column::LeaseOwner.eq(&claim.owner))
        .filter(backup_job::Column::LeaseToken.eq(claim.lease_token))
        .filter(backup_job::Column::LeaseUntil.gt(now))
        .one(connection)
        .await
        .map_err(database_error)?;
    active
        .is_some()
        .then_some(())
        .ok_or_else(|| BackupError::new(BackupErrorKind::LeaseLost))
}

async fn finish_transaction<T>(
    transaction: sea_orm::DatabaseTransaction,
    result: Result<T, BackupError>,
) -> Result<T, BackupError> {
    match result {
        Ok(value) => {
            transaction.commit().await.map_err(database_error)?;
            Ok(value)
        }
        Err(error) => {
            transaction.rollback().await.map_err(database_error)?;
            Err(error)
        }
    }
}

fn not_found() -> BackupError {
    BackupError::new(BackupErrorKind::NotFound)
}

fn database_error(_: DbErr) -> BackupError {
    BackupError::new(BackupErrorKind::Database)
}

fn conflict_or_database(error: DbErr) -> BackupError {
    if is_unique_violation(&error) {
        BackupError::new(BackupErrorKind::Conflict)
    } else {
        database_error(error)
    }
}

fn is_unique_violation(error: &DbErr) -> bool {
    let runtime = match error {
        DbErr::Conn(runtime) | DbErr::Exec(runtime) | DbErr::Query(runtime) => runtime,
        _ => return false,
    };
    let sea_orm::RuntimeErr::SqlxError(SqlxError::Database(database_error)) = runtime else {
        return false;
    };
    if let Some(error) = database_error.try_downcast_ref::<SqlxPostgresError>() {
        return error.code() == "23505";
    }
    if let Some(error) = database_error.try_downcast_ref::<SqlxMySqlError>() {
        return error.number() == 1062;
    }
    database_error
        .try_downcast_ref::<SqlxSqliteError>()
        .is_some()
        && database_error
            .code()
            .as_deref()
            .and_then(|code| code.parse::<i32>().ok())
            .is_some_and(|code| matches!(code, 1555 | 2067))
}
