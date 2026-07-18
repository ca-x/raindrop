use sea_orm::DatabaseBackend;
use sea_orm_migration::prelude::*;

use super::operational_timestamp;

#[derive(DeriveMigrationName)]
pub struct CreateOrganizationTables;

#[async_trait::async_trait]
impl MigrationTrait for CreateOrganizationTables {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        create_categories(manager).await?;
        add_subscription_category(manager).await?;
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        drop_subscription_category(manager).await?;
        manager
            .drop_table(
                Table::drop()
                    .table(Categories::Table)
                    .if_exists()
                    .to_owned(),
            )
            .await
    }
}

async fn create_categories(manager: &SchemaManager<'_>) -> Result<(), DbErr> {
    manager
        .create_table(
            Table::create()
                .table(Categories::Table)
                .if_not_exists()
                .col(ColumnDef::new(Categories::Id).string_len(36).primary_key())
                .col(ColumnDef::new(Categories::UserId).string_len(36).not_null())
                .col(ColumnDef::new(Categories::Title).string_len(200).not_null())
                .col(
                    ColumnDef::new(Categories::NormalizedTitle)
                        .string_len(320)
                        .not_null(),
                )
                .col(
                    ColumnDef::new(Categories::Position)
                        .big_integer()
                        .not_null(),
                )
                .col(operational_timestamp(manager, Categories::CreatedAt).not_null())
                .col(operational_timestamp(manager, Categories::UpdatedAt).not_null())
                .foreign_key(
                    ForeignKey::create()
                        .name("fk_categories_user")
                        .from(Categories::Table, Categories::UserId)
                        .to(Users::Table, Users::Id)
                        .on_delete(ForeignKeyAction::Cascade),
                )
                .to_owned(),
        )
        .await?;
    if !manager
        .has_index("categories", "uq_categories_user_normalized_title")
        .await?
    {
        manager
            .create_index(
                Index::create()
                    .name("uq_categories_user_normalized_title")
                    .table(Categories::Table)
                    .col(Categories::UserId)
                    .col(Categories::NormalizedTitle)
                    .unique()
                    .to_owned(),
            )
            .await?;
    }
    if !manager
        .has_index("categories", "idx_categories_user_position")
        .await?
    {
        manager
            .create_index(
                Index::create()
                    .name("idx_categories_user_position")
                    .table(Categories::Table)
                    .col(Categories::UserId)
                    .col(Categories::Position)
                    .col(Categories::Id)
                    .to_owned(),
            )
            .await?;
    }
    Ok(())
}

async fn add_subscription_category(manager: &SchemaManager<'_>) -> Result<(), DbErr> {
    if !manager.has_column("subscriptions", "category_id").await? {
        if manager.get_database_backend() == DatabaseBackend::Sqlite {
            manager
                .get_connection()
                .execute_unprepared(
                    "ALTER TABLE subscriptions
                     ADD COLUMN category_id VARCHAR(36) NULL
                     REFERENCES categories(id) ON DELETE SET NULL",
                )
                .await?;
        } else {
            manager
                .alter_table(
                    Table::alter()
                        .table(Subscriptions::Table)
                        .add_column(
                            ColumnDef::new(Subscriptions::CategoryId)
                                .string_len(36)
                                .null(),
                        )
                        .to_owned(),
                )
                .await?;
            manager
                .create_foreign_key(
                    ForeignKey::create()
                        .name("fk_subscriptions_category")
                        .from(Subscriptions::Table, Subscriptions::CategoryId)
                        .to(Categories::Table, Categories::Id)
                        .on_delete(ForeignKeyAction::SetNull)
                        .to_owned(),
                )
                .await?;
        }
    }
    if !manager
        .has_index("subscriptions", "idx_subscriptions_user_category_position")
        .await?
    {
        manager
            .create_index(
                Index::create()
                    .name("idx_subscriptions_user_category_position")
                    .table(Subscriptions::Table)
                    .col(Subscriptions::UserId)
                    .col(Subscriptions::CategoryId)
                    .col(Subscriptions::Position)
                    .col(Subscriptions::Id)
                    .to_owned(),
            )
            .await?;
    }
    Ok(())
}

async fn drop_subscription_category(manager: &SchemaManager<'_>) -> Result<(), DbErr> {
    if manager
        .has_index("subscriptions", "idx_subscriptions_user_category_position")
        .await?
    {
        manager
            .drop_index(
                Index::drop()
                    .name("idx_subscriptions_user_category_position")
                    .table(Subscriptions::Table)
                    .to_owned(),
            )
            .await?;
    }
    if !manager.has_column("subscriptions", "category_id").await? {
        return Ok(());
    }
    if manager.get_database_backend() != DatabaseBackend::Sqlite {
        manager
            .drop_foreign_key(
                ForeignKey::drop()
                    .name("fk_subscriptions_category")
                    .table(Subscriptions::Table)
                    .to_owned(),
            )
            .await?;
    }
    manager
        .alter_table(
            Table::alter()
                .table(Subscriptions::Table)
                .drop_column(Subscriptions::CategoryId)
                .to_owned(),
        )
        .await
}

#[derive(DeriveIden)]
enum Categories {
    Table,
    Id,
    UserId,
    Title,
    NormalizedTitle,
    Position,
    CreatedAt,
    UpdatedAt,
}

#[derive(DeriveIden)]
enum Users {
    Table,
    Id,
}

#[derive(DeriveIden)]
enum Subscriptions {
    Table,
    Id,
    UserId,
    CategoryId,
    Position,
}
