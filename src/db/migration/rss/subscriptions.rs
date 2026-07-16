use sea_orm_migration::prelude::*;

use super::super::operational_timestamp;

#[derive(DeriveMigrationName)]
pub struct CreateSubscriptions;

#[async_trait::async_trait]
impl MigrationTrait for CreateSubscriptions {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(Subscriptions::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(Subscriptions::Id)
                            .string_len(36)
                            .primary_key(),
                    )
                    .col(
                        ColumnDef::new(Subscriptions::UserId)
                            .string_len(36)
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(Subscriptions::FeedId)
                            .string_len(36)
                            .not_null(),
                    )
                    .col(ColumnDef::new(Subscriptions::TitleOverride).text().null())
                    .col(
                        ColumnDef::new(Subscriptions::Position)
                            .big_integer()
                            .not_null()
                            .default(0),
                    )
                    .col(
                        ColumnDef::new(Subscriptions::StartSequence)
                            .big_integer()
                            .not_null()
                            .default(0),
                    )
                    .col(
                        ColumnDef::new(Subscriptions::ReadThroughSequence)
                            .big_integer()
                            .not_null()
                            .default(0),
                    )
                    .col(
                        ColumnDef::new(Subscriptions::StateRevision)
                            .big_integer()
                            .not_null()
                            .default(0),
                    )
                    .col(operational_timestamp(manager, Subscriptions::CreatedAt).not_null())
                    .col(operational_timestamp(manager, Subscriptions::UpdatedAt).not_null())
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_subscriptions_user")
                            .from(Subscriptions::Table, Subscriptions::UserId)
                            .to(Users::Table, Users::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_subscriptions_feed")
                            .from(Subscriptions::Table, Subscriptions::FeedId)
                            .to(Feeds::Table, Feeds::Id)
                            .on_delete(ForeignKeyAction::Restrict),
                    )
                    .to_owned(),
            )
            .await?;

        if !manager
            .has_index("subscriptions", "uq_subscriptions_user_feed")
            .await?
        {
            manager
                .create_index(
                    Index::create()
                        .name("uq_subscriptions_user_feed")
                        .table(Subscriptions::Table)
                        .col(Subscriptions::UserId)
                        .col(Subscriptions::FeedId)
                        .unique()
                        .if_not_exists()
                        .to_owned(),
                )
                .await?;
        }
        if !manager
            .has_index("subscriptions", "idx_subscriptions_user_pos")
            .await?
        {
            manager
                .create_index(
                    Index::create()
                        .name("idx_subscriptions_user_pos")
                        .table(Subscriptions::Table)
                        .col(Subscriptions::UserId)
                        .col(Subscriptions::Position)
                        .col(Subscriptions::Id)
                        .if_not_exists()
                        .to_owned(),
                )
                .await?;
        }
        if !manager
            .has_index("subscriptions", "idx_subscriptions_feed")
            .await?
        {
            manager
                .create_index(
                    Index::create()
                        .name("idx_subscriptions_feed")
                        .table(Subscriptions::Table)
                        .col(Subscriptions::FeedId)
                        .col(Subscriptions::Id)
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
                    .table(Subscriptions::Table)
                    .if_exists()
                    .to_owned(),
            )
            .await
    }
}

#[derive(DeriveIden)]
enum Subscriptions {
    Table,
    Id,
    UserId,
    FeedId,
    TitleOverride,
    Position,
    StartSequence,
    ReadThroughSequence,
    StateRevision,
    CreatedAt,
    UpdatedAt,
}

#[derive(DeriveIden)]
enum Users {
    Table,
    Id,
}

#[derive(DeriveIden)]
enum Feeds {
    Table,
    Id,
}
