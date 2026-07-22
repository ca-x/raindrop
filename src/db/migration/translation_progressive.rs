use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct AddDeepLxProgressiveTranslation;

#[async_trait::async_trait]
impl MigrationTrait for AddDeepLxProgressiveTranslation {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        if !manager
            .has_column("translation_configs", "deep_lx_is_progressive")
            .await?
        {
            manager
                .alter_table(
                    Table::alter()
                        .table(TranslationConfigs::Table)
                        .add_column(
                            ColumnDef::new(TranslationConfigs::DeepLxIsProgressive)
                                .boolean()
                                .not_null()
                                .default(true),
                        )
                        .to_owned(),
                )
                .await?;
        }
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        if manager
            .has_column("translation_configs", "deep_lx_is_progressive")
            .await?
        {
            manager
                .alter_table(
                    Table::alter()
                        .table(TranslationConfigs::Table)
                        .drop_column(TranslationConfigs::DeepLxIsProgressive)
                        .to_owned(),
                )
                .await?;
        }
        Ok(())
    }
}

#[derive(DeriveIden)]
enum TranslationConfigs {
    Table,
    DeepLxIsProgressive,
}
