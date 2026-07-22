use sea_orm_migration::prelude::*;

use super::operational_timestamp;

#[derive(DeriveMigrationName)]
pub struct CreateSubscriptionBackups;

#[async_trait::async_trait]
impl MigrationTrait for CreateSubscriptionBackups {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        create_targets(manager).await?;
        create_schedules(manager).await?;
        create_schedule_targets(manager).await?;
        create_jobs(manager).await?;
        create_job_targets(manager).await?;
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        for table in [
            BackupJobTargets::Table.into_iden(),
            BackupJobs::Table.into_iden(),
            BackupScheduleTargets::Table.into_iden(),
            BackupSchedules::Table.into_iden(),
            BackupTargets::Table.into_iden(),
        ] {
            manager
                .drop_table(Table::drop().table(table).if_exists().to_owned())
                .await?;
        }
        Ok(())
    }
}

async fn create_targets(manager: &SchemaManager<'_>) -> Result<(), DbErr> {
    manager
        .create_table(
            Table::create()
                .table(BackupTargets::Table)
                .if_not_exists()
                .col(
                    ColumnDef::new(BackupTargets::Id)
                        .string_len(36)
                        .primary_key(),
                )
                .col(
                    ColumnDef::new(BackupTargets::UserId)
                        .string_len(36)
                        .not_null(),
                )
                .col(
                    ColumnDef::new(BackupTargets::Kind)
                        .string_len(16)
                        .not_null(),
                )
                .col(
                    ColumnDef::new(BackupTargets::DisplayName)
                        .string_len(320)
                        .not_null(),
                )
                .col(ColumnDef::new(BackupTargets::Enabled).boolean().not_null())
                .col(
                    ColumnDef::new(BackupTargets::PublicConfigJson)
                        .text()
                        .not_null(),
                )
                .col(
                    ColumnDef::new(BackupTargets::SecretConfigCiphertext)
                        .text()
                        .not_null(),
                )
                .col(ColumnDef::new(BackupTargets::RetainCount).integer().null())
                .col(ColumnDef::new(BackupTargets::RetainDays).integer().null())
                .col(
                    ColumnDef::new(BackupTargets::Revision)
                        .big_integer()
                        .not_null()
                        .default(1),
                )
                .col(operational_timestamp(manager, BackupTargets::CreatedAt).not_null())
                .col(operational_timestamp(manager, BackupTargets::UpdatedAt).not_null())
                .foreign_key(
                    ForeignKey::create()
                        .name("fk_backup_targets_user")
                        .from(BackupTargets::Table, BackupTargets::UserId)
                        .to(Users::Table, Users::Id)
                        .on_delete(ForeignKeyAction::Cascade),
                )
                .to_owned(),
        )
        .await?;
    manager
        .create_index(
            Index::create()
                .name("uq_backup_targets_user_name")
                .table(BackupTargets::Table)
                .col(BackupTargets::UserId)
                .col(BackupTargets::DisplayName)
                .unique()
                .if_not_exists()
                .to_owned(),
        )
        .await?;
    manager
        .create_index(
            Index::create()
                .name("idx_backup_targets_user_kind")
                .table(BackupTargets::Table)
                .col(BackupTargets::UserId)
                .col(BackupTargets::Kind)
                .if_not_exists()
                .to_owned(),
        )
        .await
}

async fn create_schedules(manager: &SchemaManager<'_>) -> Result<(), DbErr> {
    manager
        .create_table(
            Table::create()
                .table(BackupSchedules::Table)
                .if_not_exists()
                .col(
                    ColumnDef::new(BackupSchedules::UserId)
                        .string_len(36)
                        .primary_key(),
                )
                .col(
                    ColumnDef::new(BackupSchedules::Enabled)
                        .boolean()
                        .not_null(),
                )
                .col(
                    ColumnDef::new(BackupSchedules::IntervalHours)
                        .integer()
                        .not_null(),
                )
                .col(operational_timestamp(manager, BackupSchedules::NextRunAt).null())
                .col(
                    ColumnDef::new(BackupSchedules::Revision)
                        .big_integer()
                        .not_null()
                        .default(1),
                )
                .col(operational_timestamp(manager, BackupSchedules::CreatedAt).not_null())
                .col(operational_timestamp(manager, BackupSchedules::UpdatedAt).not_null())
                .foreign_key(
                    ForeignKey::create()
                        .name("fk_backup_schedules_user")
                        .from(BackupSchedules::Table, BackupSchedules::UserId)
                        .to(Users::Table, Users::Id)
                        .on_delete(ForeignKeyAction::Cascade),
                )
                .to_owned(),
        )
        .await?;
    manager
        .create_index(
            Index::create()
                .name("idx_backup_schedules_due")
                .table(BackupSchedules::Table)
                .col(BackupSchedules::Enabled)
                .col(BackupSchedules::NextRunAt)
                .if_not_exists()
                .to_owned(),
        )
        .await
}

async fn create_schedule_targets(manager: &SchemaManager<'_>) -> Result<(), DbErr> {
    manager
        .create_table(
            Table::create()
                .table(BackupScheduleTargets::Table)
                .if_not_exists()
                .col(
                    ColumnDef::new(BackupScheduleTargets::UserId)
                        .string_len(36)
                        .not_null(),
                )
                .col(
                    ColumnDef::new(BackupScheduleTargets::TargetId)
                        .string_len(36)
                        .not_null(),
                )
                .primary_key(
                    Index::create()
                        .name("pk_backup_schedule_targets")
                        .col(BackupScheduleTargets::UserId)
                        .col(BackupScheduleTargets::TargetId),
                )
                .foreign_key(
                    ForeignKey::create()
                        .name("fk_backup_schedule_targets_schedule")
                        .from(BackupScheduleTargets::Table, BackupScheduleTargets::UserId)
                        .to(BackupSchedules::Table, BackupSchedules::UserId)
                        .on_delete(ForeignKeyAction::Cascade),
                )
                .foreign_key(
                    ForeignKey::create()
                        .name("fk_backup_schedule_targets_target")
                        .from(
                            BackupScheduleTargets::Table,
                            BackupScheduleTargets::TargetId,
                        )
                        .to(BackupTargets::Table, BackupTargets::Id)
                        .on_delete(ForeignKeyAction::Cascade),
                )
                .to_owned(),
        )
        .await
}

async fn create_jobs(manager: &SchemaManager<'_>) -> Result<(), DbErr> {
    manager
        .create_table(
            Table::create()
                .table(BackupJobs::Table)
                .if_not_exists()
                .col(ColumnDef::new(BackupJobs::Id).string_len(36).primary_key())
                .col(ColumnDef::new(BackupJobs::UserId).string_len(36).not_null())
                .col(
                    ColumnDef::new(BackupJobs::TriggerKind)
                        .string_len(16)
                        .not_null(),
                )
                .col(operational_timestamp(manager, BackupJobs::ScheduledFor).null())
                .col(ColumnDef::new(BackupJobs::Status).string_len(16).not_null())
                .col(ColumnDef::new(BackupJobs::TargetCount).integer().not_null())
                .col(ColumnDef::new(BackupJobs::LeaseOwner).string_len(64).null())
                .col(
                    ColumnDef::new(BackupJobs::LeaseToken)
                        .big_integer()
                        .not_null()
                        .default(0),
                )
                .col(operational_timestamp(manager, BackupJobs::LeaseUntil).null())
                .col(
                    ColumnDef::new(BackupJobs::LastErrorCode)
                        .string_len(64)
                        .null(),
                )
                .col(operational_timestamp(manager, BackupJobs::CreatedAt).not_null())
                .col(operational_timestamp(manager, BackupJobs::StartedAt).null())
                .col(operational_timestamp(manager, BackupJobs::CompletedAt).null())
                .foreign_key(
                    ForeignKey::create()
                        .name("fk_backup_jobs_user")
                        .from(BackupJobs::Table, BackupJobs::UserId)
                        .to(Users::Table, Users::Id)
                        .on_delete(ForeignKeyAction::Cascade),
                )
                .to_owned(),
        )
        .await?;
    manager
        .create_index(
            Index::create()
                .name("uq_backup_jobs_schedule_slot")
                .table(BackupJobs::Table)
                .col(BackupJobs::UserId)
                .col(BackupJobs::ScheduledFor)
                .unique()
                .if_not_exists()
                .to_owned(),
        )
        .await?;
    manager
        .create_index(
            Index::create()
                .name("idx_backup_jobs_claim")
                .table(BackupJobs::Table)
                .col(BackupJobs::Status)
                .col(BackupJobs::CreatedAt)
                .if_not_exists()
                .to_owned(),
        )
        .await?;
    manager
        .create_index(
            Index::create()
                .name("idx_backup_jobs_user_created")
                .table(BackupJobs::Table)
                .col(BackupJobs::UserId)
                .col(BackupJobs::CreatedAt)
                .if_not_exists()
                .to_owned(),
        )
        .await
}

async fn create_job_targets(manager: &SchemaManager<'_>) -> Result<(), DbErr> {
    manager
        .create_table(
            Table::create()
                .table(BackupJobTargets::Table)
                .if_not_exists()
                .col(
                    ColumnDef::new(BackupJobTargets::Id)
                        .string_len(36)
                        .primary_key(),
                )
                .col(
                    ColumnDef::new(BackupJobTargets::JobId)
                        .string_len(36)
                        .not_null(),
                )
                .col(
                    ColumnDef::new(BackupJobTargets::TargetId)
                        .string_len(36)
                        .null(),
                )
                .col(
                    ColumnDef::new(BackupJobTargets::TargetKind)
                        .string_len(16)
                        .not_null(),
                )
                .col(
                    ColumnDef::new(BackupJobTargets::TargetName)
                        .string_len(320)
                        .not_null(),
                )
                .col(
                    ColumnDef::new(BackupJobTargets::TargetRevision)
                        .big_integer()
                        .not_null(),
                )
                .col(
                    ColumnDef::new(BackupJobTargets::ObjectKey)
                        .text()
                        .not_null(),
                )
                .col(
                    ColumnDef::new(BackupJobTargets::Status)
                        .string_len(16)
                        .not_null(),
                )
                .col(
                    ColumnDef::new(BackupJobTargets::ByteSize)
                        .big_integer()
                        .null(),
                )
                .col(
                    ColumnDef::new(BackupJobTargets::ErrorCode)
                        .string_len(64)
                        .null(),
                )
                .col(operational_timestamp(manager, BackupJobTargets::StartedAt).null())
                .col(operational_timestamp(manager, BackupJobTargets::CompletedAt).null())
                .foreign_key(
                    ForeignKey::create()
                        .name("fk_backup_job_targets_job")
                        .from(BackupJobTargets::Table, BackupJobTargets::JobId)
                        .to(BackupJobs::Table, BackupJobs::Id)
                        .on_delete(ForeignKeyAction::Cascade),
                )
                .foreign_key(
                    ForeignKey::create()
                        .name("fk_backup_job_targets_target")
                        .from(BackupJobTargets::Table, BackupJobTargets::TargetId)
                        .to(BackupTargets::Table, BackupTargets::Id)
                        .on_delete(ForeignKeyAction::SetNull),
                )
                .to_owned(),
        )
        .await?;
    manager
        .create_index(
            Index::create()
                .name("idx_backup_job_targets_job")
                .table(BackupJobTargets::Table)
                .col(BackupJobTargets::JobId)
                .if_not_exists()
                .to_owned(),
        )
        .await
}

#[derive(DeriveIden)]
enum Users {
    Table,
    Id,
}

#[derive(DeriveIden)]
enum BackupTargets {
    Table,
    Id,
    UserId,
    Kind,
    DisplayName,
    Enabled,
    PublicConfigJson,
    SecretConfigCiphertext,
    RetainCount,
    RetainDays,
    Revision,
    CreatedAt,
    UpdatedAt,
}

#[derive(DeriveIden)]
enum BackupSchedules {
    Table,
    UserId,
    Enabled,
    IntervalHours,
    NextRunAt,
    Revision,
    CreatedAt,
    UpdatedAt,
}

#[derive(DeriveIden)]
enum BackupScheduleTargets {
    Table,
    UserId,
    TargetId,
}

#[derive(DeriveIden)]
enum BackupJobs {
    Table,
    Id,
    UserId,
    TriggerKind,
    ScheduledFor,
    Status,
    TargetCount,
    LeaseOwner,
    LeaseToken,
    LeaseUntil,
    LastErrorCode,
    CreatedAt,
    StartedAt,
    CompletedAt,
}

#[derive(DeriveIden)]
enum BackupJobTargets {
    Table,
    Id,
    JobId,
    TargetId,
    TargetKind,
    TargetName,
    TargetRevision,
    ObjectKey,
    Status,
    ByteSize,
    ErrorCode,
    StartedAt,
    CompletedAt,
}
