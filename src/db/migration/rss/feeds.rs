use sea_orm_migration::prelude::*;

use super::super::operational_timestamp;

#[derive(DeriveMigrationName)]
pub struct CreateFeeds;

#[async_trait::async_trait]
impl MigrationTrait for CreateFeeds {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(Feeds::Table)
                    .if_not_exists()
                    .col(ColumnDef::new(Feeds::Id).string_len(36).primary_key())
                    .col(ColumnDef::new(Feeds::SourceUrl).text().not_null())
                    .col(ColumnDef::new(Feeds::NormalizedUrl).text().not_null())
                    .col(
                        ColumnDef::new(Feeds::NormalizedUrlHash)
                            .string_len(64)
                            .not_null(),
                    )
                    .col(ColumnDef::new(Feeds::FetchUrl).text().not_null())
                    .col(ColumnDef::new(Feeds::ValidatorUrl).text().null())
                    .col(ColumnDef::new(Feeds::Etag).text().null())
                    .col(ColumnDef::new(Feeds::LastModified).text().null())
                    .col(
                        ColumnDef::new(Feeds::ResponseContentHash)
                            .string_len(64)
                            .null(),
                    )
                    .col(
                        ColumnDef::new(Feeds::EntrySequenceHead)
                            .big_integer()
                            .not_null()
                            .default(0),
                    )
                    .col(operational_timestamp(manager, Feeds::LastAttemptAt).null())
                    .col(operational_timestamp(manager, Feeds::LastSuccessAt).null())
                    .col(operational_timestamp(manager, Feeds::LastChangedAt).null())
                    .col(operational_timestamp(manager, Feeds::NextFetchAt).not_null())
                    .col(operational_timestamp(manager, Feeds::RetryAfterAt).null())
                    .col(
                        ColumnDef::new(Feeds::ConsecutiveFailures)
                            .big_integer()
                            .not_null()
                            .default(0),
                    )
                    .col(ColumnDef::new(Feeds::LastErrorCode).string_len(64).null())
                    .col(
                        ColumnDef::new(Feeds::IsDisabled)
                            .boolean()
                            .not_null()
                            .default(false),
                    )
                    .col(operational_timestamp(manager, Feeds::OrphanedAt).null())
                    .col(ColumnDef::new(Feeds::LeaseOwner).string_len(128).null())
                    .col(
                        ColumnDef::new(Feeds::LeaseToken)
                            .big_integer()
                            .not_null()
                            .default(0),
                    )
                    .col(operational_timestamp(manager, Feeds::LeaseUntil).null())
                    .col(operational_timestamp(manager, Feeds::CreatedAt).not_null())
                    .col(operational_timestamp(manager, Feeds::UpdatedAt).not_null())
                    .to_owned(),
            )
            .await?;

        if !manager.has_index("feeds", "uq_feeds_url_hash").await? {
            manager
                .create_index(
                    Index::create()
                        .name("uq_feeds_url_hash")
                        .table(Feeds::Table)
                        .col(Feeds::NormalizedUrlHash)
                        .unique()
                        .if_not_exists()
                        .to_owned(),
                )
                .await?;
        }
        if !manager.has_index("feeds", "idx_feeds_due").await? {
            manager
                .create_index(
                    Index::create()
                        .name("idx_feeds_due")
                        .table(Feeds::Table)
                        .col(Feeds::IsDisabled)
                        .col(Feeds::NextFetchAt)
                        .col(Feeds::LeaseUntil)
                        .col(Feeds::Id)
                        .if_not_exists()
                        .to_owned(),
                )
                .await?;
        }

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(Feeds::Table).if_exists().to_owned())
            .await
    }
}

#[derive(DeriveIden)]
enum Feeds {
    Table,
    Id,
    SourceUrl,
    NormalizedUrl,
    NormalizedUrlHash,
    FetchUrl,
    ValidatorUrl,
    Etag,
    LastModified,
    ResponseContentHash,
    EntrySequenceHead,
    LastAttemptAt,
    LastSuccessAt,
    LastChangedAt,
    NextFetchAt,
    RetryAfterAt,
    ConsecutiveFailures,
    LastErrorCode,
    IsDisabled,
    OrphanedAt,
    LeaseOwner,
    LeaseToken,
    LeaseUntil,
    CreatedAt,
    UpdatedAt,
}
