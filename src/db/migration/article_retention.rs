use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct CreateArticleRetention;

#[async_trait::async_trait]
impl MigrationTrait for CreateArticleRetention {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(Alias::new("article_retention_settings"))
                    .if_not_exists()
                    .col(ColumnDef::new(Alias::new("id")).integer().primary_key())
                    .col(
                        ColumnDef::new(Alias::new("enabled"))
                            .boolean()
                            .not_null()
                            .default(false),
                    )
                    .col(
                        ColumnDef::new(Alias::new("retention_days"))
                            .integer()
                            .not_null()
                            .default(30),
                    )
                    .to_owned(),
            )
            .await?;
        manager
            .create_table(
                Table::create()
                    .table(Alias::new("retired_entry_identities"))
                    .if_not_exists()
                    .col(
                        ColumnDef::new(Alias::new("feed_id"))
                            .string_len(36)
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(Alias::new("identity_hash"))
                            .string_len(64)
                            .not_null(),
                    )
                    .primary_key(
                        Index::create()
                            .col(Alias::new("feed_id"))
                            .col(Alias::new("identity_hash")),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_retired_entries_feed")
                            .from(
                                Alias::new("retired_entry_identities"),
                                Alias::new("feed_id"),
                            )
                            .to(Alias::new("feeds"), Alias::new("id"))
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            )
            .await?;
        if !manager
            .has_index("entries", "idx_entries_retention")
            .await?
        {
            manager
                .create_index(
                    Index::create()
                        .name("idx_entries_retention")
                        .table(Alias::new("entries"))
                        .col(Alias::new("inserted_at"))
                        .col(Alias::new("id"))
                        .to_owned(),
                )
                .await?;
        }
        for (table, name, columns) in [
            (
                "entry_states",
                "idx_states_entry_retention",
                vec!["entry_id", "is_starred", "read_override"],
            ),
            ("content_jobs", "idx_jobs_entry_retention", vec!["entry_id"]),
            (
                "content_artifacts",
                "idx_artifacts_entry_retention",
                vec!["entry_id"],
            ),
        ] {
            if !manager.has_index(table, name).await? {
                let mut index = Index::create();
                index.name(name).table(Alias::new(table));
                for column in columns {
                    index.col(Alias::new(column));
                }
                manager.create_index(index.to_owned()).await?;
            }
        }
        Ok(())
    }
    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        if manager
            .has_index("entries", "idx_entries_retention")
            .await?
        {
            manager
                .drop_index(
                    Index::drop()
                        .name("idx_entries_retention")
                        .table(Alias::new("entries"))
                        .to_owned(),
                )
                .await?;
        }
        for (table, name, foreign_key_index) in [
            ("entry_states", "idx_states_entry_retention", None),
            (
                "content_jobs",
                "idx_jobs_entry_retention",
                Some("fk_content_jobs_entry"),
            ),
            (
                "content_artifacts",
                "idx_artifacts_entry_retention",
                Some("fk_content_artifacts_entry"),
            ),
        ] {
            if manager.has_index(table, name).await? {
                // MySQL may replace implicit FK indexes with these retention indexes.
                // Restore their support before dropping the replacement.
                if manager.get_database_backend() == sea_orm::DbBackend::MySql
                    && let Some(foreign_key_index) = foreign_key_index
                    && !manager.has_index(table, foreign_key_index).await?
                {
                    manager
                        .create_index(
                            Index::create()
                                .name(foreign_key_index)
                                .table(Alias::new(table))
                                .col(Alias::new("entry_id"))
                                .to_owned(),
                        )
                        .await?;
                }
                manager
                    .drop_index(Index::drop().name(name).table(Alias::new(table)).to_owned())
                    .await?;
            }
        }
        for table in ["retired_entry_identities", "article_retention_settings"] {
            manager
                .drop_table(
                    Table::drop()
                        .table(Alias::new(table))
                        .if_exists()
                        .to_owned(),
                )
                .await?;
        }
        Ok(())
    }
}
