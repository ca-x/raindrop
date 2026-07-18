use sea_orm_migration::prelude::*;

use super::operational_timestamp;

#[derive(DeriveMigrationName)]
pub struct CreateAiProviders;

#[async_trait::async_trait]
impl MigrationTrait for CreateAiProviders {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(AiProviders::Table)
                    .if_not_exists()
                    .col(ColumnDef::new(AiProviders::Id).string_len(36).primary_key())
                    .col(
                        ColumnDef::new(AiProviders::OwnerUserId)
                            .string_len(36)
                            .null(),
                    )
                    .col(
                        ColumnDef::new(AiProviders::DisplayName)
                            .string_len(80)
                            .not_null(),
                    )
                    .col(ColumnDef::new(AiProviders::Kind).string_len(40).not_null())
                    .col(ColumnDef::new(AiProviders::Endpoint).text().not_null())
                    .col(
                        ColumnDef::new(AiProviders::Model)
                            .string_len(200)
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(AiProviders::EncryptedSecret)
                            .text()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(AiProviders::SupportsUsage)
                            .boolean()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(AiProviders::SupportsIdempotency)
                            .boolean()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(AiProviders::SupportsStreaming)
                            .boolean()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(AiProviders::MaxConcurrency)
                            .integer()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(AiProviders::RequestsPerMinute)
                            .integer()
                            .null(),
                    )
                    .col(
                        ColumnDef::new(AiProviders::MaxInputTokensPerRequest)
                            .integer()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(AiProviders::MaxOutputTokensPerRequest)
                            .integer()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(AiProviders::InputCostMicrosPerMillionTokens)
                            .big_integer()
                            .null(),
                    )
                    .col(
                        ColumnDef::new(AiProviders::OutputCostMicrosPerMillionTokens)
                            .big_integer()
                            .null(),
                    )
                    .col(
                        ColumnDef::new(AiProviders::MaxCostMicrosPerRequest)
                            .big_integer()
                            .null(),
                    )
                    .col(ColumnDef::new(AiProviders::IsEnabled).boolean().not_null())
                    .col(
                        ColumnDef::new(AiProviders::Revision)
                            .big_integer()
                            .not_null(),
                    )
                    .col(operational_timestamp(manager, AiProviders::CreatedAt).not_null())
                    .col(operational_timestamp(manager, AiProviders::UpdatedAt).not_null())
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_ai_providers_owner_user")
                            .from(AiProviders::Table, AiProviders::OwnerUserId)
                            .to(Users::Table, Users::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            )
            .await?;
        if !manager
            .has_index("ai_providers", "idx_ai_providers_owner_enabled")
            .await?
        {
            manager
                .create_index(
                    Index::create()
                        .name("idx_ai_providers_owner_enabled")
                        .table(AiProviders::Table)
                        .col(AiProviders::OwnerUserId)
                        .col(AiProviders::IsEnabled)
                        .to_owned(),
                )
                .await?;
        }
        if !manager
            .has_index("ai_providers", "idx_ai_providers_kind")
            .await?
        {
            manager
                .create_index(
                    Index::create()
                        .name("idx_ai_providers_kind")
                        .table(AiProviders::Table)
                        .col(AiProviders::Kind)
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
                    .table(AiProviders::Table)
                    .if_exists()
                    .to_owned(),
            )
            .await
    }
}

#[derive(DeriveIden)]
enum AiProviders {
    Table,
    Id,
    OwnerUserId,
    DisplayName,
    Kind,
    Endpoint,
    Model,
    EncryptedSecret,
    SupportsUsage,
    SupportsIdempotency,
    SupportsStreaming,
    MaxConcurrency,
    RequestsPerMinute,
    MaxInputTokensPerRequest,
    MaxOutputTokensPerRequest,
    InputCostMicrosPerMillionTokens,
    OutputCostMicrosPerMillionTokens,
    MaxCostMicrosPerRequest,
    IsEnabled,
    Revision,
    CreatedAt,
    UpdatedAt,
}

#[derive(DeriveIden)]
enum Users {
    Table,
    Id,
}
