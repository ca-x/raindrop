use sea_orm::DatabaseBackend;
use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct FeedMetadata;

#[async_trait::async_trait]
impl MigrationTrait for FeedMetadata {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        if !manager.has_column("feeds", "title").await? {
            let mut title = ColumnDef::new(Feeds::Title);
            if manager.get_database_backend() == DatabaseBackend::MySql {
                title.custom(Alias::new("MEDIUMTEXT"));
            } else {
                title.text();
            }
            manager
                .alter_table(
                    Table::alter()
                        .table(Feeds::Table)
                        .add_column(title.null())
                        .to_owned(),
                )
                .await?;
        }
        if !manager.has_column("feeds", "site_url").await? {
            manager
                .alter_table(
                    Table::alter()
                        .table(Feeds::Table)
                        .add_column(ColumnDef::new(Feeds::SiteUrl).text().null())
                        .to_owned(),
                )
                .await?;
        }
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        if manager.has_column("feeds", "site_url").await? {
            manager
                .alter_table(
                    Table::alter()
                        .table(Feeds::Table)
                        .drop_column(Feeds::SiteUrl)
                        .to_owned(),
                )
                .await?;
        }
        if manager.has_column("feeds", "title").await? {
            manager
                .alter_table(
                    Table::alter()
                        .table(Feeds::Table)
                        .drop_column(Feeds::Title)
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
    Title,
    SiteUrl,
}
