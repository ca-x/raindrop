use sea_orm::{ConnectionTrait, Statement};
use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct CreateBootstrapState;

#[async_trait::async_trait]
impl MigrationTrait for CreateBootstrapState {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(BootstrapState::Table)
                    .if_not_exists()
                    .col(ColumnDef::new(BootstrapState::Id).integer().primary_key())
                    .col(
                        ColumnDef::new(BootstrapState::AdministratorUserId)
                            .string_len(36)
                            .not_null(),
                    )
                    .to_owned(),
            )
            .await?;
        manager
            .get_connection()
            .execute(Statement::from_string(
                manager.get_database_backend(),
                "INSERT INTO bootstrap_state (id, administrator_user_id) \
                 SELECT 1, id FROM users \
                 WHERE NOT EXISTS (SELECT 1 FROM bootstrap_state WHERE id = 1) \
                 ORDER BY created_at, id LIMIT 1"
                    .to_owned(),
            ))
            .await?;
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(
                Table::drop()
                    .table(BootstrapState::Table)
                    .if_exists()
                    .to_owned(),
            )
            .await?;
        Ok(())
    }
}

#[derive(DeriveIden)]
enum BootstrapState {
    Table,
    Id,
    AdministratorUserId,
}
