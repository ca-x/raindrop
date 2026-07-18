use sea_orm_migration::prelude::*;

use super::operational_timestamp;

#[derive(DeriveMigrationName)]
pub struct CreateUserPreferences;

#[async_trait::async_trait]
impl MigrationTrait for CreateUserPreferences {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(UserPreferences::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(UserPreferences::UserId)
                            .string_len(36)
                            .primary_key(),
                    )
                    .col(
                        ColumnDef::new(UserPreferences::Locale)
                            .string_len(8)
                            .not_null()
                            .check(Expr::col(UserPreferences::Locale).is_in(["zh-CN", "en"])),
                    )
                    .col(
                        ColumnDef::new(UserPreferences::ThemeMode)
                            .string_len(16)
                            .not_null()
                            .check(
                                Expr::col(UserPreferences::ThemeMode)
                                    .is_in(["SYSTEM", "LIGHT", "DARK"]),
                            ),
                    )
                    .col(
                        ColumnDef::new(UserPreferences::LayoutDensity)
                            .string_len(16)
                            .not_null()
                            .check(
                                Expr::col(UserPreferences::LayoutDensity)
                                    .is_in(["COMPACT", "BALANCED", "SPACIOUS"]),
                            ),
                    )
                    .col(
                        ColumnDef::new(UserPreferences::ReadingFontScale)
                            .integer()
                            .not_null()
                            .check(Expr::col(UserPreferences::ReadingFontScale).between(85, 130)),
                    )
                    .col(operational_timestamp(manager, UserPreferences::CreatedAt).not_null())
                    .col(operational_timestamp(manager, UserPreferences::UpdatedAt).not_null())
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_user_preferences_user")
                            .from(UserPreferences::Table, UserPreferences::UserId)
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
                    .table(UserPreferences::Table)
                    .if_exists()
                    .to_owned(),
            )
            .await
    }
}

#[derive(DeriveIden)]
enum UserPreferences {
    Table,
    UserId,
    Locale,
    ThemeMode,
    LayoutDensity,
    ReadingFontScale,
    CreatedAt,
    UpdatedAt,
}

#[derive(DeriveIden)]
enum Users {
    Table,
    Id,
}
