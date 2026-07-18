use sea_orm_migration::prelude::*;

use super::operational_timestamp;

#[derive(DeriveMigrationName)]
pub struct CreatePluginRegistry;

#[async_trait::async_trait]
impl MigrationTrait for CreatePluginRegistry {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        create_installations(manager).await?;
        create_configs(manager).await?;
        create_grants(manager).await?;
        create_kv(manager).await?;
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(PluginKv::Table).if_exists().to_owned())
            .await?;
        manager
            .drop_table(
                Table::drop()
                    .table(PluginCapabilityGrants::Table)
                    .if_exists()
                    .to_owned(),
            )
            .await?;
        manager
            .drop_table(
                Table::drop()
                    .table(PluginConfigs::Table)
                    .if_exists()
                    .to_owned(),
            )
            .await?;
        manager
            .drop_table(
                Table::drop()
                    .table(PluginInstallations::Table)
                    .if_exists()
                    .to_owned(),
            )
            .await?;
        Ok(())
    }
}

async fn create_installations(manager: &SchemaManager<'_>) -> Result<(), DbErr> {
    manager
        .create_table(
            Table::create()
                .table(PluginInstallations::Table)
                .if_not_exists()
                .col(
                    ColumnDef::new(PluginInstallations::Id)
                        .string_len(36)
                        .primary_key(),
                )
                .col(
                    ColumnDef::new(PluginInstallations::PluginKey)
                        .string_len(128)
                        .not_null(),
                )
                .col(
                    ColumnDef::new(PluginInstallations::Version)
                        .string_len(64)
                        .not_null(),
                )
                .col(
                    ColumnDef::new(PluginInstallations::AbiVersion)
                        .string_len(128)
                        .not_null(),
                )
                .col(
                    ColumnDef::new(PluginInstallations::Distribution)
                        .string_len(32)
                        .not_null(),
                )
                .col(
                    ColumnDef::new(PluginInstallations::ComponentDigest)
                        .string_len(64)
                        .not_null(),
                )
                .col(
                    ColumnDef::new(PluginInstallations::ManifestJson)
                        .text()
                        .not_null(),
                )
                .col(
                    ColumnDef::new(PluginInstallations::SignatureKeyId)
                        .string_len(128)
                        .not_null(),
                )
                .col(
                    ColumnDef::new(PluginInstallations::Signature)
                        .string_len(128)
                        .not_null(),
                )
                .col(
                    ColumnDef::new(PluginInstallations::SystemState)
                        .string_len(16)
                        .not_null(),
                )
                .col(
                    ColumnDef::new(PluginInstallations::Revision)
                        .big_integer()
                        .not_null()
                        .default(0),
                )
                .col(operational_timestamp(manager, PluginInstallations::InstalledAt).not_null())
                .col(operational_timestamp(manager, PluginInstallations::UpdatedAt).not_null())
                .to_owned(),
        )
        .await?;

    create_index(
        manager,
        "plugin_installations",
        "uq_plugin_installations_key",
        Index::create()
            .name("uq_plugin_installations_key")
            .table(PluginInstallations::Table)
            .col(PluginInstallations::PluginKey)
            .unique()
            .to_owned(),
    )
    .await?;
    create_index(
        manager,
        "plugin_installations",
        "idx_plugin_installations_state",
        Index::create()
            .name("idx_plugin_installations_state")
            .table(PluginInstallations::Table)
            .col(PluginInstallations::SystemState)
            .col(PluginInstallations::PluginKey)
            .to_owned(),
    )
    .await
}

async fn create_configs(manager: &SchemaManager<'_>) -> Result<(), DbErr> {
    manager
        .create_table(
            Table::create()
                .table(PluginConfigs::Table)
                .if_not_exists()
                .col(
                    ColumnDef::new(PluginConfigs::Id)
                        .string_len(36)
                        .primary_key(),
                )
                .col(
                    ColumnDef::new(PluginConfigs::PluginId)
                        .string_len(36)
                        .not_null(),
                )
                .col(
                    ColumnDef::new(PluginConfigs::OwnerUserId)
                        .string_len(36)
                        .not_null(),
                )
                .col(
                    ColumnDef::new(PluginConfigs::SchemaVersion)
                        .integer()
                        .not_null(),
                )
                .col(ColumnDef::new(PluginConfigs::ConfigJson).text().not_null())
                .col(
                    ColumnDef::new(PluginConfigs::ConfigHash)
                        .string_len(64)
                        .not_null(),
                )
                .col(
                    ColumnDef::new(PluginConfigs::IsEnabled)
                        .boolean()
                        .not_null(),
                )
                .col(
                    ColumnDef::new(PluginConfigs::Revision)
                        .big_integer()
                        .not_null()
                        .default(0),
                )
                .col(operational_timestamp(manager, PluginConfigs::CreatedAt).not_null())
                .col(operational_timestamp(manager, PluginConfigs::UpdatedAt).not_null())
                .foreign_key(
                    ForeignKey::create()
                        .name("fk_plugin_configs_plugin")
                        .from(PluginConfigs::Table, PluginConfigs::PluginId)
                        .to(PluginInstallations::Table, PluginInstallations::Id)
                        .on_delete(ForeignKeyAction::Restrict),
                )
                .foreign_key(
                    ForeignKey::create()
                        .name("fk_plugin_configs_user")
                        .from(PluginConfigs::Table, PluginConfigs::OwnerUserId)
                        .to(Users::Table, Users::Id)
                        .on_delete(ForeignKeyAction::Cascade),
                )
                .to_owned(),
        )
        .await?;

    create_index(
        manager,
        "plugin_configs",
        "uq_plugin_configs_owner",
        Index::create()
            .name("uq_plugin_configs_owner")
            .table(PluginConfigs::Table)
            .col(PluginConfigs::PluginId)
            .col(PluginConfigs::OwnerUserId)
            .unique()
            .to_owned(),
    )
    .await?;
    create_index(
        manager,
        "plugin_configs",
        "idx_plugin_configs_owner_enabled",
        Index::create()
            .name("idx_plugin_configs_owner_enabled")
            .table(PluginConfigs::Table)
            .col(PluginConfigs::OwnerUserId)
            .col(PluginConfigs::IsEnabled)
            .col(PluginConfigs::PluginId)
            .to_owned(),
    )
    .await
}

async fn create_grants(manager: &SchemaManager<'_>) -> Result<(), DbErr> {
    manager
        .create_table(
            Table::create()
                .table(PluginCapabilityGrants::Table)
                .if_not_exists()
                .col(
                    ColumnDef::new(PluginCapabilityGrants::Id)
                        .string_len(36)
                        .primary_key(),
                )
                .col(
                    ColumnDef::new(PluginCapabilityGrants::PluginId)
                        .string_len(36)
                        .not_null(),
                )
                .col(
                    ColumnDef::new(PluginCapabilityGrants::OwnerUserId)
                        .string_len(36)
                        .not_null(),
                )
                .col(
                    ColumnDef::new(PluginCapabilityGrants::Capability)
                        .string_len(64)
                        .not_null(),
                )
                .col(
                    ColumnDef::new(PluginCapabilityGrants::Operation)
                        .string_len(32)
                        .not_null(),
                )
                .col(
                    ColumnDef::new(PluginCapabilityGrants::ResourceType)
                        .string_len(32)
                        .not_null(),
                )
                .col(
                    ColumnDef::new(PluginCapabilityGrants::ResourceId)
                        .string_len(255)
                        .not_null(),
                )
                .col(
                    ColumnDef::new(PluginCapabilityGrants::GrantKeyHash)
                        .string_len(64)
                        .not_null(),
                )
                .col(
                    ColumnDef::new(PluginCapabilityGrants::ConstraintsJson)
                        .text()
                        .not_null(),
                )
                .col(
                    ColumnDef::new(PluginCapabilityGrants::Revision)
                        .big_integer()
                        .not_null()
                        .default(0),
                )
                .col(operational_timestamp(manager, PluginCapabilityGrants::CreatedAt).not_null())
                .col(operational_timestamp(manager, PluginCapabilityGrants::UpdatedAt).not_null())
                .col(operational_timestamp(manager, PluginCapabilityGrants::RevokedAt).null())
                .foreign_key(
                    ForeignKey::create()
                        .name("fk_plugin_grants_plugin")
                        .from(
                            PluginCapabilityGrants::Table,
                            PluginCapabilityGrants::PluginId,
                        )
                        .to(PluginInstallations::Table, PluginInstallations::Id)
                        .on_delete(ForeignKeyAction::Restrict),
                )
                .foreign_key(
                    ForeignKey::create()
                        .name("fk_plugin_grants_user")
                        .from(
                            PluginCapabilityGrants::Table,
                            PluginCapabilityGrants::OwnerUserId,
                        )
                        .to(Users::Table, Users::Id)
                        .on_delete(ForeignKeyAction::Cascade),
                )
                .to_owned(),
        )
        .await?;

    create_index(
        manager,
        "plugin_capability_grants",
        "uq_plugin_grants_key",
        Index::create()
            .name("uq_plugin_grants_key")
            .table(PluginCapabilityGrants::Table)
            .col(PluginCapabilityGrants::PluginId)
            .col(PluginCapabilityGrants::OwnerUserId)
            .col(PluginCapabilityGrants::GrantKeyHash)
            .unique()
            .to_owned(),
    )
    .await?;
    create_index(
        manager,
        "plugin_capability_grants",
        "idx_plugin_grants_owner_active",
        Index::create()
            .name("idx_plugin_grants_owner_active")
            .table(PluginCapabilityGrants::Table)
            .col(PluginCapabilityGrants::OwnerUserId)
            .col(PluginCapabilityGrants::PluginId)
            .col(PluginCapabilityGrants::RevokedAt)
            .col(PluginCapabilityGrants::Capability)
            .col(PluginCapabilityGrants::Operation)
            .to_owned(),
    )
    .await?;
    create_index(
        manager,
        "plugin_capability_grants",
        "idx_plugin_grants_resource",
        Index::create()
            .name("idx_plugin_grants_resource")
            .table(PluginCapabilityGrants::Table)
            .col(PluginCapabilityGrants::OwnerUserId)
            .col(PluginCapabilityGrants::ResourceType)
            .col(PluginCapabilityGrants::ResourceId)
            .col(PluginCapabilityGrants::RevokedAt)
            .to_owned(),
    )
    .await
}

async fn create_kv(manager: &SchemaManager<'_>) -> Result<(), DbErr> {
    manager
        .create_table(
            Table::create()
                .table(PluginKv::Table)
                .if_not_exists()
                .col(ColumnDef::new(PluginKv::PluginId).string_len(36).not_null())
                .col(
                    ColumnDef::new(PluginKv::OwnerUserId)
                        .string_len(36)
                        .not_null(),
                )
                .col(ColumnDef::new(PluginKv::Key).string_len(128).not_null())
                .col(ColumnDef::new(PluginKv::Value).blob().not_null())
                .col(
                    ColumnDef::new(PluginKv::ValueSizeBytes)
                        .integer()
                        .not_null(),
                )
                .col(
                    ColumnDef::new(PluginKv::Revision)
                        .big_integer()
                        .not_null()
                        .default(0),
                )
                .col(operational_timestamp(manager, PluginKv::CreatedAt).not_null())
                .col(operational_timestamp(manager, PluginKv::UpdatedAt).not_null())
                .primary_key(
                    Index::create()
                        .name("pk_plugin_kv")
                        .col(PluginKv::PluginId)
                        .col(PluginKv::OwnerUserId)
                        .col(PluginKv::Key),
                )
                .foreign_key(
                    ForeignKey::create()
                        .name("fk_plugin_kv_plugin")
                        .from(PluginKv::Table, PluginKv::PluginId)
                        .to(PluginInstallations::Table, PluginInstallations::Id)
                        .on_delete(ForeignKeyAction::Restrict),
                )
                .foreign_key(
                    ForeignKey::create()
                        .name("fk_plugin_kv_user")
                        .from(PluginKv::Table, PluginKv::OwnerUserId)
                        .to(Users::Table, Users::Id)
                        .on_delete(ForeignKeyAction::Cascade),
                )
                .to_owned(),
        )
        .await?;

    create_index(
        manager,
        "plugin_kv",
        "idx_plugin_kv_owner_updated",
        Index::create()
            .name("idx_plugin_kv_owner_updated")
            .table(PluginKv::Table)
            .col(PluginKv::OwnerUserId)
            .col(PluginKv::PluginId)
            .col(PluginKv::UpdatedAt)
            .col(PluginKv::Key)
            .to_owned(),
    )
    .await
}

async fn create_index(
    manager: &SchemaManager<'_>,
    table: &str,
    name: &str,
    statement: IndexCreateStatement,
) -> Result<(), DbErr> {
    if !manager.has_index(table, name).await? {
        manager.create_index(statement).await?;
    }
    Ok(())
}

#[derive(DeriveIden, Clone, Copy)]
enum PluginInstallations {
    Table,
    Id,
    PluginKey,
    Version,
    AbiVersion,
    Distribution,
    ComponentDigest,
    ManifestJson,
    SignatureKeyId,
    Signature,
    SystemState,
    Revision,
    InstalledAt,
    UpdatedAt,
}

#[derive(DeriveIden, Clone, Copy)]
enum PluginConfigs {
    Table,
    Id,
    PluginId,
    OwnerUserId,
    SchemaVersion,
    ConfigJson,
    ConfigHash,
    IsEnabled,
    Revision,
    CreatedAt,
    UpdatedAt,
}

#[derive(DeriveIden, Clone, Copy)]
enum PluginCapabilityGrants {
    Table,
    Id,
    PluginId,
    OwnerUserId,
    Capability,
    Operation,
    ResourceType,
    ResourceId,
    GrantKeyHash,
    ConstraintsJson,
    Revision,
    CreatedAt,
    UpdatedAt,
    RevokedAt,
}

#[derive(DeriveIden, Clone, Copy)]
enum PluginKv {
    Table,
    PluginId,
    OwnerUserId,
    Key,
    Value,
    ValueSizeBytes,
    Revision,
    CreatedAt,
    UpdatedAt,
}

#[derive(DeriveIden)]
enum Users {
    Table,
    Id,
}
