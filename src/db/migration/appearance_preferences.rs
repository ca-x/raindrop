use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct AddAppearancePreferences;

#[async_trait::async_trait]
impl MigrationTrait for AddAppearancePreferences {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        for column in ["interface_blur", "interface_shadows"] {
            if !manager.has_column("user_preferences", column).await? {
                manager
                    .alter_table(
                        Table::alter()
                            .table(Alias::new("user_preferences"))
                            .add_column(
                                ColumnDef::new(Alias::new(column))
                                    .boolean()
                                    .not_null()
                                    .default(true),
                            )
                            .to_owned(),
                    )
                    .await?;
            }
        }
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        for column in ["interface_shadows", "interface_blur"] {
            if manager.has_column("user_preferences", column).await? {
                manager
                    .alter_table(
                        Table::alter()
                            .table(Alias::new("user_preferences"))
                            .drop_column(Alias::new(column))
                            .to_owned(),
                    )
                    .await?;
            }
        }
        Ok(())
    }
}
