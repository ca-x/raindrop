use sea_orm_migration::prelude::*;

use super::super::operational_timestamp;

#[derive(DeriveMigrationName)]
pub struct CreateLifecycleOutbox;

#[async_trait::async_trait]
impl MigrationTrait for CreateLifecycleOutbox {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(LifecycleOutbox::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(LifecycleOutbox::Id)
                            .string_len(36)
                            .primary_key(),
                    )
                    .col(
                        ColumnDef::new(LifecycleOutbox::EventType)
                            .string_len(64)
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(LifecycleOutbox::AggregateType)
                            .string_len(32)
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(LifecycleOutbox::AggregateId)
                            .string_len(64)
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(LifecycleOutbox::RefreshId)
                            .string_len(36)
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(LifecycleOutbox::EventSequence)
                            .integer()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(LifecycleOutbox::PayloadVersion)
                            .integer()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(LifecycleOutbox::PayloadJson)
                            .text()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(LifecycleOutbox::IdempotencyKey)
                            .string_len(128)
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(LifecycleOutbox::Status)
                            .string_len(16)
                            .not_null()
                            .default("PENDING"),
                    )
                    .col(operational_timestamp(manager, LifecycleOutbox::AvailableAt).not_null())
                    .col(
                        ColumnDef::new(LifecycleOutbox::Attempts)
                            .integer()
                            .not_null()
                            .default(0),
                    )
                    .col(
                        ColumnDef::new(LifecycleOutbox::LeaseOwner)
                            .string_len(64)
                            .null(),
                    )
                    .col(operational_timestamp(manager, LifecycleOutbox::LeaseUntil).null())
                    .col(operational_timestamp(manager, LifecycleOutbox::CreatedAt).not_null())
                    .col(operational_timestamp(manager, LifecycleOutbox::CompletedAt).null())
                    .to_owned(),
            )
            .await?;

        for (name, index) in [
            (
                "uq_lifecycle_outbox_idem",
                Index::create()
                    .name("uq_lifecycle_outbox_idem")
                    .table(LifecycleOutbox::Table)
                    .col(LifecycleOutbox::IdempotencyKey)
                    .unique()
                    .if_not_exists()
                    .to_owned(),
            ),
            (
                "uq_lifecycle_outbox_order",
                Index::create()
                    .name("uq_lifecycle_outbox_order")
                    .table(LifecycleOutbox::Table)
                    .col(LifecycleOutbox::RefreshId)
                    .col(LifecycleOutbox::EventSequence)
                    .unique()
                    .if_not_exists()
                    .to_owned(),
            ),
            (
                "idx_lifecycle_outbox_due",
                Index::create()
                    .name("idx_lifecycle_outbox_due")
                    .table(LifecycleOutbox::Table)
                    .col(LifecycleOutbox::Status)
                    .col(LifecycleOutbox::AvailableAt)
                    .col(LifecycleOutbox::LeaseUntil)
                    .col(LifecycleOutbox::Id)
                    .if_not_exists()
                    .to_owned(),
            ),
        ] {
            if !manager.has_index("lifecycle_outbox", name).await? {
                manager.create_index(index).await?;
            }
        }

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(
                Table::drop()
                    .table(LifecycleOutbox::Table)
                    .if_exists()
                    .to_owned(),
            )
            .await
    }
}

#[derive(DeriveIden)]
enum LifecycleOutbox {
    Table,
    Id,
    EventType,
    AggregateType,
    AggregateId,
    RefreshId,
    EventSequence,
    PayloadVersion,
    PayloadJson,
    IdempotencyKey,
    Status,
    AvailableAt,
    Attempts,
    LeaseOwner,
    LeaseUntil,
    CreatedAt,
    CompletedAt,
}
