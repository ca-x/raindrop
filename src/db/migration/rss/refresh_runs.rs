use sea_orm_migration::prelude::*;

use super::super::operational_timestamp;

#[derive(DeriveMigrationName)]
pub struct CreateRefreshRuns;

#[async_trait::async_trait]
impl MigrationTrait for CreateRefreshRuns {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(FeedRefreshRuns::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(FeedRefreshRuns::Id)
                            .string_len(36)
                            .primary_key(),
                    )
                    .col(
                        ColumnDef::new(FeedRefreshRuns::FeedId)
                            .string_len(36)
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(FeedRefreshRuns::RequestedByUserId)
                            .string_len(36)
                            .null(),
                    )
                    .col(
                        ColumnDef::new(FeedRefreshRuns::TriggerKind)
                            .string_len(32)
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(FeedRefreshRuns::Status)
                            .string_len(32)
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(FeedRefreshRuns::IdempotencyKey)
                            .string_len(64)
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(FeedRefreshRuns::LeaseToken)
                            .big_integer()
                            .null(),
                    )
                    .col(
                        ColumnDef::new(FeedRefreshRuns::CommitGeneration)
                            .big_integer()
                            .null(),
                    )
                    .col(operational_timestamp(manager, FeedRefreshRuns::QueuedAt).not_null())
                    .col(operational_timestamp(manager, FeedRefreshRuns::StartedAt).null())
                    .col(operational_timestamp(manager, FeedRefreshRuns::FetchedAt).null())
                    .col(operational_timestamp(manager, FeedRefreshRuns::PersistedAt).null())
                    .col(operational_timestamp(manager, FeedRefreshRuns::CompletedAt).null())
                    .col(ColumnDef::new(FeedRefreshRuns::HttpStatus).integer().null())
                    .col(
                        ColumnDef::new(FeedRefreshRuns::NewCount)
                            .integer()
                            .not_null()
                            .default(0),
                    )
                    .col(
                        ColumnDef::new(FeedRefreshRuns::UpdatedCount)
                            .integer()
                            .not_null()
                            .default(0),
                    )
                    .col(
                        ColumnDef::new(FeedRefreshRuns::DroppedCount)
                            .integer()
                            .not_null()
                            .default(0),
                    )
                    .col(
                        ColumnDef::new(FeedRefreshRuns::ErrorCode)
                            .string_len(64)
                            .null(),
                    )
                    .col(operational_timestamp(manager, FeedRefreshRuns::RetryAt).null())
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_refresh_runs_feed")
                            .from(FeedRefreshRuns::Table, FeedRefreshRuns::FeedId)
                            .to(Feeds::Table, Feeds::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_refresh_runs_requested_by")
                            .from(FeedRefreshRuns::Table, FeedRefreshRuns::RequestedByUserId)
                            .to(Users::Table, Users::Id)
                            .on_delete(ForeignKeyAction::SetNull),
                    )
                    .to_owned(),
            )
            .await?;

        create_index_if_missing(
            manager,
            "uq_refresh_runs_idem",
            Index::create()
                .name("uq_refresh_runs_idem")
                .table(FeedRefreshRuns::Table)
                .col(FeedRefreshRuns::FeedId)
                .col(FeedRefreshRuns::IdempotencyKey)
                .unique()
                .if_not_exists()
                .to_owned(),
        )
        .await?;
        create_index_if_missing(
            manager,
            "uq_refresh_runs_generation",
            Index::create()
                .name("uq_refresh_runs_generation")
                .table(FeedRefreshRuns::Table)
                .col(FeedRefreshRuns::CommitGeneration)
                .unique()
                .if_not_exists()
                .to_owned(),
        )
        .await?;
        create_index_if_missing(
            manager,
            "idx_refresh_runs_feed",
            Index::create()
                .name("idx_refresh_runs_feed")
                .table(FeedRefreshRuns::Table)
                .col(FeedRefreshRuns::FeedId)
                .col(FeedRefreshRuns::QueuedAt)
                .col(FeedRefreshRuns::Id)
                .if_not_exists()
                .to_owned(),
        )
        .await?;
        create_index_if_missing(
            manager,
            "idx_refresh_runs_status",
            Index::create()
                .name("idx_refresh_runs_status")
                .table(FeedRefreshRuns::Table)
                .col(FeedRefreshRuns::Status)
                .col(FeedRefreshRuns::QueuedAt)
                .col(FeedRefreshRuns::Id)
                .if_not_exists()
                .to_owned(),
        )
        .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(
                Table::drop()
                    .table(FeedRefreshRuns::Table)
                    .if_exists()
                    .to_owned(),
            )
            .await
    }
}

async fn create_index_if_missing(
    manager: &SchemaManager<'_>,
    name: &str,
    index: IndexCreateStatement,
) -> Result<(), DbErr> {
    if !manager.has_index("feed_refresh_runs", name).await? {
        manager.create_index(index).await?;
    }
    Ok(())
}

#[derive(DeriveIden)]
enum FeedRefreshRuns {
    Table,
    Id,
    FeedId,
    RequestedByUserId,
    TriggerKind,
    Status,
    IdempotencyKey,
    LeaseToken,
    CommitGeneration,
    QueuedAt,
    StartedAt,
    FetchedAt,
    PersistedAt,
    CompletedAt,
    HttpStatus,
    NewCount,
    UpdatedCount,
    DroppedCount,
    ErrorCode,
    RetryAt,
}

#[derive(DeriveIden)]
enum Feeds {
    Table,
    Id,
}

#[derive(DeriveIden)]
enum Users {
    Table,
    Id,
}
