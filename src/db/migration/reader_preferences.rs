use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct AddReaderPreferences;

#[async_trait::async_trait]
impl MigrationTrait for AddReaderPreferences {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        if !manager
            .has_column("user_preferences", "reading_font_family")
            .await?
        {
            manager
                .alter_table(
                    Table::alter()
                        .table(UserPreferences::Table)
                        .add_column(
                            ColumnDef::new(UserPreferences::ReadingFontFamily)
                                .string_len(16)
                                .not_null()
                                .default("SERIF")
                                .check(
                                    Expr::col(UserPreferences::ReadingFontFamily)
                                        .is_in(["SERIF", "SANS"]),
                                ),
                        )
                        .to_owned(),
                )
                .await?;
        }
        if !manager
            .has_column("user_preferences", "reading_color_scheme")
            .await?
        {
            manager
                .alter_table(
                    Table::alter()
                        .table(UserPreferences::Table)
                        .add_column(
                            ColumnDef::new(UserPreferences::ReadingColorScheme)
                                .string_len(16)
                                .not_null()
                                .default("AUTO")
                                .check(
                                    Expr::col(UserPreferences::ReadingColorScheme)
                                        .is_in(["AUTO", "PAPER", "SEPIA", "GRAY"]),
                                ),
                        )
                        .to_owned(),
                )
                .await?;
        }
        if !manager
            .has_column("user_preferences", "link_open_mode")
            .await?
        {
            manager
                .alter_table(
                    Table::alter()
                        .table(UserPreferences::Table)
                        .add_column(
                            ColumnDef::new(UserPreferences::LinkOpenMode)
                                .string_len(16)
                                .not_null()
                                .default("NEW_TAB")
                                .check(
                                    Expr::col(UserPreferences::LinkOpenMode)
                                        .is_in(["CURRENT_TAB", "NEW_TAB"]),
                                ),
                        )
                        .to_owned(),
                )
                .await?;
        }
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        if manager
            .has_column("user_preferences", "link_open_mode")
            .await?
        {
            manager
                .alter_table(
                    Table::alter()
                        .table(UserPreferences::Table)
                        .drop_column(UserPreferences::LinkOpenMode)
                        .to_owned(),
                )
                .await?;
        }
        if manager
            .has_column("user_preferences", "reading_color_scheme")
            .await?
        {
            manager
                .alter_table(
                    Table::alter()
                        .table(UserPreferences::Table)
                        .drop_column(UserPreferences::ReadingColorScheme)
                        .to_owned(),
                )
                .await?;
        }
        if manager
            .has_column("user_preferences", "reading_font_family")
            .await?
        {
            manager
                .alter_table(
                    Table::alter()
                        .table(UserPreferences::Table)
                        .drop_column(UserPreferences::ReadingFontFamily)
                        .to_owned(),
                )
                .await?;
        }
        Ok(())
    }
}

#[derive(DeriveIden)]
enum UserPreferences {
    Table,
    ReadingFontFamily,
    ReadingColorScheme,
    LinkOpenMode,
}
