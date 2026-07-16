use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct CreateEntryStates;

#[async_trait::async_trait]
impl MigrationTrait for CreateEntryStates {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(EntryStates::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(EntryStates::UserId)
                            .string_len(36)
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(EntryStates::EntryId)
                            .string_len(36)
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(EntryStates::FeedId)
                            .string_len(36)
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(EntryStates::FeedSequence)
                            .big_integer()
                            .not_null(),
                    )
                    .col(ColumnDef::new(EntryStates::ReadOverride).boolean().null())
                    .col(
                        ColumnDef::new(EntryStates::IsStarred)
                            .boolean()
                            .not_null()
                            .default(false),
                    )
                    .col(
                        ColumnDef::new(EntryStates::StarredAt)
                            .timestamp_with_time_zone()
                            .null(),
                    )
                    .col(
                        ColumnDef::new(EntryStates::Revision)
                            .big_integer()
                            .not_null()
                            .default(0),
                    )
                    .col(
                        ColumnDef::new(EntryStates::UpdatedAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .primary_key(
                        Index::create()
                            .name("pk_entry_states")
                            .col(EntryStates::UserId)
                            .col(EntryStates::EntryId),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_entry_states_subscription")
                            .from(
                                EntryStates::Table,
                                (EntryStates::UserId, EntryStates::FeedId),
                            )
                            .to(
                                Subscriptions::Table,
                                (Subscriptions::UserId, Subscriptions::FeedId),
                            )
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_entry_states_entry")
                            .from(
                                EntryStates::Table,
                                (
                                    EntryStates::EntryId,
                                    EntryStates::FeedId,
                                    EntryStates::FeedSequence,
                                ),
                            )
                            .to(
                                Entries::Table,
                                (Entries::Id, Entries::FeedId, Entries::FeedSequence),
                            )
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            )
            .await?;

        if !manager
            .has_index("entry_states", "idx_states_feed_read")
            .await?
        {
            manager
                .create_index(
                    Index::create()
                        .name("idx_states_feed_read")
                        .table(EntryStates::Table)
                        .col(EntryStates::UserId)
                        .col(EntryStates::FeedId)
                        .col(EntryStates::ReadOverride)
                        .col(EntryStates::FeedSequence)
                        .if_not_exists()
                        .to_owned(),
                )
                .await?;
        }
        if !manager
            .has_index("entry_states", "idx_states_starred")
            .await?
        {
            manager
                .create_index(
                    Index::create()
                        .name("idx_states_starred")
                        .table(EntryStates::Table)
                        .col(EntryStates::UserId)
                        .col(EntryStates::IsStarred)
                        .col(EntryStates::StarredAt)
                        .col(EntryStates::EntryId)
                        .if_not_exists()
                        .to_owned(),
                )
                .await?;
        }

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(
                Table::drop()
                    .table(EntryStates::Table)
                    .if_exists()
                    .to_owned(),
            )
            .await
    }
}

#[derive(DeriveIden)]
enum EntryStates {
    Table,
    UserId,
    EntryId,
    FeedId,
    FeedSequence,
    ReadOverride,
    IsStarred,
    StarredAt,
    Revision,
    UpdatedAt,
}

#[derive(DeriveIden)]
enum Subscriptions {
    Table,
    UserId,
    FeedId,
}

#[derive(DeriveIden)]
enum Entries {
    Table,
    Id,
    FeedId,
    FeedSequence,
}
