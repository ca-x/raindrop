use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct CreateTtsConfigs;

#[async_trait::async_trait]
impl MigrationTrait for CreateTtsConfigs {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(TtsConfigs::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(TtsConfigs::UserId)
                            .string_len(36)
                            .primary_key(),
                    )
                    .col(ColumnDef::new(TtsConfigs::ConfigJson).text().not_null())
                    .col(ColumnDef::new(TtsConfigs::EncryptedApiKey).text().null())
                    .col(
                        ColumnDef::new(TtsConfigs::Revision)
                            .big_integer()
                            .not_null(),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_tts_configs_user")
                            .from(TtsConfigs::Table, TtsConfigs::UserId)
                            .to(Alias::new("users"), Alias::new("id"))
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(TtsConfigs::Table).to_owned())
            .await
    }
}

#[derive(DeriveIden)]
enum TtsConfigs {
    Table,
    UserId,
    ConfigJson,
    EncryptedApiKey,
    Revision,
}
