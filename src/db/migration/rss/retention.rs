use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct CreateFeedRetention;

#[async_trait::async_trait]
impl MigrationTrait for CreateFeedRetention {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        if !manager
            .has_index("feeds", "idx_feeds_orphan_retention")
            .await?
        {
            manager
                .create_index(
                    Index::create()
                        .name("idx_feeds_orphan_retention")
                        .table(Feeds::Table)
                        .col(Feeds::OrphanedAt)
                        .col(Feeds::Id)
                        .if_not_exists()
                        .to_owned(),
                )
                .await?;
        }
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        if manager
            .has_index("feeds", "idx_feeds_orphan_retention")
            .await?
        {
            manager
                .drop_index(
                    Index::drop()
                        .name("idx_feeds_orphan_retention")
                        .table(Feeds::Table)
                        .to_owned(),
                )
                .await?;
        }
        Ok(())
    }
}

#[derive(DeriveIden)]
enum Feeds {
    Table,
    Id,
    OrphanedAt,
}
