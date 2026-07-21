use sea_orm::DatabaseBackend;
use sea_orm_migration::prelude::*;

use super::operational_timestamp;

#[derive(DeriveMigrationName)]
pub struct CreateUserFonts;

#[async_trait::async_trait]
impl MigrationTrait for CreateUserFonts {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let mut font_bytes = ColumnDef::new(UserFonts::FontBytes);
        if manager.get_database_backend() == DatabaseBackend::MySql {
            font_bytes.custom(Alias::new("MEDIUMBLOB"));
        } else {
            font_bytes.blob();
        }
        manager
            .create_table(
                Table::create()
                    .table(UserFonts::Table)
                    .if_not_exists()
                    .col(ColumnDef::new(UserFonts::Id).string_len(36).primary_key())
                    .col(ColumnDef::new(UserFonts::UserId).string_len(36).not_null())
                    .col(
                        ColumnDef::new(UserFonts::DisplayName)
                            .string_len(80)
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(UserFonts::NormalizedName)
                            .string_len(80)
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(UserFonts::ContentHash)
                            .string_len(64)
                            .not_null(),
                    )
                    .col(font_bytes.not_null())
                    .col(ColumnDef::new(UserFonts::ByteSize).integer().not_null())
                    .col(operational_timestamp(manager, UserFonts::CreatedAt).not_null())
                    .col(operational_timestamp(manager, UserFonts::UpdatedAt).not_null())
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_user_fonts_user")
                            .from(UserFonts::Table, UserFonts::UserId)
                            .to(Users::Table, Users::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            )
            .await?;
        manager
            .create_index(
                Index::create()
                    .name("uq_user_fonts_name")
                    .table(UserFonts::Table)
                    .col(UserFonts::UserId)
                    .col(UserFonts::NormalizedName)
                    .unique()
                    .if_not_exists()
                    .to_owned(),
            )
            .await?;
        manager
            .create_index(
                Index::create()
                    .name("uq_user_fonts_content")
                    .table(UserFonts::Table)
                    .col(UserFonts::UserId)
                    .col(UserFonts::ContentHash)
                    .unique()
                    .if_not_exists()
                    .to_owned(),
            )
            .await?;
        if !manager
            .has_column("user_preferences", "reading_custom_font_id")
            .await?
        {
            manager
                .alter_table(
                    Table::alter()
                        .table(UserPreferences::Table)
                        .add_column(
                            ColumnDef::new(UserPreferences::ReadingCustomFontId)
                                .string_len(36)
                                .null(),
                        )
                        .to_owned(),
                )
                .await?;
        }
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        if manager
            .has_column("user_preferences", "reading_custom_font_id")
            .await?
        {
            manager
                .alter_table(
                    Table::alter()
                        .table(UserPreferences::Table)
                        .drop_column(UserPreferences::ReadingCustomFontId)
                        .to_owned(),
                )
                .await?;
        }
        manager
            .drop_table(Table::drop().table(UserFonts::Table).if_exists().to_owned())
            .await
    }
}

#[derive(DeriveIden)]
enum UserFonts {
    Table,
    Id,
    UserId,
    DisplayName,
    NormalizedName,
    ContentHash,
    FontBytes,
    ByteSize,
    CreatedAt,
    UpdatedAt,
}

#[derive(DeriveIden)]
enum UserPreferences {
    Table,
    ReadingCustomFontId,
}

#[derive(DeriveIden)]
enum Users {
    Table,
    Id,
}
