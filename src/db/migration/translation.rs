use sea_orm_migration::prelude::*;

use super::operational_timestamp;

#[derive(DeriveMigrationName)]
pub struct CreateTranslationConfigs;

#[async_trait::async_trait]
impl MigrationTrait for CreateTranslationConfigs {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(TranslationConfigs::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(TranslationConfigs::UserId)
                            .string_len(36)
                            .primary_key(),
                    )
                    .col(
                        ColumnDef::new(TranslationConfigs::Engine)
                            .string_len(16)
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(TranslationConfigs::DisplayMode)
                            .string_len(24)
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(TranslationConfigs::IsEnabled)
                            .boolean()
                            .not_null()
                            .default(false),
                    )
                    .col(
                        ColumnDef::new(TranslationConfigs::DefaultTargetLocale)
                            .string_len(35)
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(TranslationConfigs::OpenAiProviderId)
                            .string_len(36)
                            .null(),
                    )
                    .col(
                        ColumnDef::new(TranslationConfigs::OpenAiMaxOutputTokens)
                            .integer()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(TranslationConfigs::OpenAiProfile)
                            .string_len(24)
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(TranslationConfigs::OpenAiCustomSystemPrompt)
                            .text()
                            .null(),
                    )
                    .col(
                        ColumnDef::new(TranslationConfigs::OpenAiCustomPrompt)
                            .text()
                            .null(),
                    )
                    .col(
                        ColumnDef::new(TranslationConfigs::DeepLxDisplayName)
                            .string_len(80)
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(TranslationConfigs::DeepLxDescription)
                            .string_len(500)
                            .null(),
                    )
                    .col(
                        ColumnDef::new(TranslationConfigs::DeepLxBaseUrl)
                            .string_len(2048)
                            .null(),
                    )
                    .col(
                        ColumnDef::new(TranslationConfigs::EncryptedDeepLxApiKey)
                            .text()
                            .null(),
                    )
                    .col(
                        ColumnDef::new(TranslationConfigs::Revision)
                            .big_integer()
                            .not_null()
                            .default(0),
                    )
                    .col(operational_timestamp(manager, TranslationConfigs::CreatedAt).not_null())
                    .col(operational_timestamp(manager, TranslationConfigs::UpdatedAt).not_null())
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_translation_configs_user")
                            .from(TranslationConfigs::Table, TranslationConfigs::UserId)
                            .to(Users::Table, Users::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(
                Table::drop()
                    .table(TranslationConfigs::Table)
                    .if_exists()
                    .to_owned(),
            )
            .await
    }
}

#[derive(DeriveIden)]
enum TranslationConfigs {
    Table,
    UserId,
    Engine,
    DisplayMode,
    IsEnabled,
    DefaultTargetLocale,
    OpenAiProviderId,
    OpenAiMaxOutputTokens,
    OpenAiProfile,
    OpenAiCustomSystemPrompt,
    OpenAiCustomPrompt,
    DeepLxDisplayName,
    DeepLxDescription,
    DeepLxBaseUrl,
    EncryptedDeepLxApiKey,
    Revision,
    CreatedAt,
    UpdatedAt,
}

#[derive(DeriveIden)]
enum Users {
    Table,
    Id,
}
