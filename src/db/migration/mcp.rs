use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct CreateMcpTokens;

#[async_trait::async_trait]
impl MigrationTrait for CreateMcpTokens {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(Alias::new("mcp_tokens"))
                    .col(
                        ColumnDef::new(Alias::new("id"))
                            .string_len(36)
                            .primary_key(),
                    )
                    .col(
                        ColumnDef::new(Alias::new("user_id"))
                            .string_len(36)
                            .not_null(),
                    )
                    .col(ColumnDef::new(Alias::new("name")).string_len(80).not_null())
                    .col(
                        ColumnDef::new(Alias::new("prefix"))
                            .string_len(16)
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(Alias::new("token_hash"))
                            .string_len(64)
                            .not_null()
                            .unique_key(),
                    )
                    .col(
                        ColumnDef::new(Alias::new("created_at"))
                            .big_integer()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(Alias::new("expires_at"))
                            .big_integer()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(Alias::new("last_used_at"))
                            .big_integer()
                            .null(),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .from(Alias::new("mcp_tokens"), Alias::new("user_id"))
                            .to(Alias::new("users"), Alias::new("id"))
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            )
            .await?;
        manager
            .create_index(
                Index::create()
                    .name("mcp_tokens_user")
                    .table(Alias::new("mcp_tokens"))
                    .col(Alias::new("user_id"))
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(Alias::new("mcp_tokens")).to_owned())
            .await
    }
}
