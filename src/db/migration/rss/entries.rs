use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct CreateEntries;

#[async_trait::async_trait]
impl MigrationTrait for CreateEntries {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(Entries::Table)
                    .if_not_exists()
                    .col(ColumnDef::new(Entries::Id).string_len(36).primary_key())
                    .col(ColumnDef::new(Entries::FeedId).string_len(36).not_null())
                    .col(
                        ColumnDef::new(Entries::FeedSequence)
                            .big_integer()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(Entries::IngestGeneration)
                            .big_integer()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(Entries::IdentityKind)
                            .string_len(16)
                            .not_null(),
                    )
                    .col(ColumnDef::new(Entries::Identity).text().not_null())
                    .col(
                        ColumnDef::new(Entries::IdentityHash)
                            .string_len(64)
                            .not_null(),
                    )
                    .col(ColumnDef::new(Entries::CanonicalUrl).text().null())
                    .col(ColumnDef::new(Entries::Title).text().null())
                    .col(ColumnDef::new(Entries::Author).text().null())
                    .col(ColumnDef::new(Entries::SanitizedContent).text().not_null())
                    .col(ColumnDef::new(Entries::Summary).text().null())
                    .col(ColumnDef::new(Entries::PublishedAtUs).big_integer().null())
                    .col(ColumnDef::new(Entries::SortAtUs).big_integer().not_null())
                    .col(
                        ColumnDef::new(Entries::InsertedAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(Entries::UpdatedAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(Entries::SourceContentHash)
                            .string_len(64)
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(Entries::ContentHash)
                            .string_len(64)
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(Entries::PipelineVersion)
                            .string_len(64)
                            .not_null(),
                    )
                    .col(ColumnDef::new(Entries::Direction).string_len(8).null())
                    .col(ColumnDef::new(Entries::EnclosureJson).text().null())
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_entries_feed")
                            .from(Entries::Table, Entries::FeedId)
                            .to(Feeds::Table, Feeds::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            )
            .await?;

        for (name, index) in [
            (
                "uq_entries_feed_identity",
                Index::create()
                    .name("uq_entries_feed_identity")
                    .table(Entries::Table)
                    .col(Entries::FeedId)
                    .col(Entries::IdentityHash)
                    .unique()
                    .if_not_exists()
                    .to_owned(),
            ),
            (
                "uq_entries_feed_seq",
                Index::create()
                    .name("uq_entries_feed_seq")
                    .table(Entries::Table)
                    .col(Entries::FeedId)
                    .col(Entries::FeedSequence)
                    .unique()
                    .if_not_exists()
                    .to_owned(),
            ),
            (
                "uq_entries_state_tuple",
                Index::create()
                    .name("uq_entries_state_tuple")
                    .table(Entries::Table)
                    .col(Entries::Id)
                    .col(Entries::FeedId)
                    .col(Entries::FeedSequence)
                    .unique()
                    .if_not_exists()
                    .to_owned(),
            ),
            (
                "idx_entries_feed_list",
                Index::create()
                    .name("idx_entries_feed_list")
                    .table(Entries::Table)
                    .col(Entries::FeedId)
                    .col(Entries::SortAtUs)
                    .col(Entries::Id)
                    .if_not_exists()
                    .to_owned(),
            ),
            (
                "idx_entries_all_list",
                Index::create()
                    .name("idx_entries_all_list")
                    .table(Entries::Table)
                    .col(Entries::SortAtUs)
                    .col(Entries::Id)
                    .col(Entries::FeedId)
                    .if_not_exists()
                    .to_owned(),
            ),
            (
                "idx_entries_snapshot",
                Index::create()
                    .name("idx_entries_snapshot")
                    .table(Entries::Table)
                    .col(Entries::FeedId)
                    .col(Entries::IngestGeneration)
                    .col(Entries::FeedSequence)
                    .if_not_exists()
                    .to_owned(),
            ),
        ] {
            if !manager.has_index("entries", name).await? {
                manager.create_index(index).await?;
            }
        }

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(Entries::Table).if_exists().to_owned())
            .await
    }
}

#[derive(DeriveIden)]
enum Entries {
    Table,
    Id,
    FeedId,
    FeedSequence,
    IngestGeneration,
    IdentityKind,
    Identity,
    IdentityHash,
    CanonicalUrl,
    Title,
    Author,
    SanitizedContent,
    Summary,
    PublishedAtUs,
    SortAtUs,
    InsertedAt,
    UpdatedAt,
    SourceContentHash,
    ContentHash,
    PipelineVersion,
    Direction,
    EnclosureJson,
}

#[derive(DeriveIden)]
enum Feeds {
    Table,
    Id,
}
