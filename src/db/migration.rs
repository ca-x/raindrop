use sea_orm::{DatabaseConnection, DbBackend};
use sea_orm_migration::prelude::*;

use super::DbError;

mod ai_providers;
mod bootstrap_state;
mod content_jobs;
mod organization;
mod preferences;
mod rss;

pub async fn migrate(database: &DatabaseConnection) -> Result<(), DbError> {
    Migrator::up(database, None).await.map_err(DbError::from)
}

pub async fn rollback(database: &DatabaseConnection) -> Result<(), DbError> {
    Migrator::down(database, None).await.map_err(DbError::from)
}

pub(super) fn operational_timestamp<T>(manager: &SchemaManager<'_>, name: T) -> ColumnDef
where
    T: IntoIden,
{
    let mut column = ColumnDef::new(name);
    if manager.get_database_backend() == DbBackend::MySql {
        column.custom(Alias::new("datetime(6)"));
    } else {
        column.timestamp_with_time_zone();
    }
    column
}

struct Migrator;

#[async_trait::async_trait]
impl MigratorTrait for Migrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![
            Box::new(CreateIdentityTables),
            Box::new(rss::counters::CreateRssCounters),
            Box::new(rss::feeds::CreateFeeds),
            Box::new(rss::subscriptions::CreateSubscriptions),
            Box::new(rss::entries::CreateEntries),
            Box::new(rss::entry_states::CreateEntryStates),
            Box::new(rss::refresh_runs::CreateRefreshRuns),
            Box::new(bootstrap_state::CreateBootstrapState),
            Box::new(rss::entry_storage::EntryStorage),
            Box::new(rss::entry_search::EntrySearch),
            Box::new(rss::feed_metadata::FeedMetadata),
            Box::new(rss::retention::CreateFeedRetention),
            Box::new(rss::outbox::CreateLifecycleOutbox),
            Box::new(organization::CreateOrganizationTables),
            Box::new(preferences::CreateUserPreferences),
            Box::new(ai_providers::CreateAiProviders),
            Box::new(content_jobs::CreateContentJobs),
        ]
    }
}

#[derive(DeriveMigrationName)]
struct CreateIdentityTables;

#[async_trait::async_trait]
impl MigrationTrait for CreateIdentityTables {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(Users::Table)
                    .if_not_exists()
                    .col(ColumnDef::new(Users::Id).string_len(36).primary_key())
                    .col(ColumnDef::new(Users::Username).string_len(64).not_null())
                    .col(
                        ColumnDef::new(Users::NormalizedUsername)
                            .string_len(64)
                            .not_null(),
                    )
                    .col(ColumnDef::new(Users::Email).string_len(320).null())
                    .col(ColumnDef::new(Users::PasswordHash).text().not_null())
                    .col(
                        ColumnDef::new(Users::IsDisabled)
                            .boolean()
                            .not_null()
                            .default(false),
                    )
                    .col(
                        ColumnDef::new(Users::CreatedAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(Users::LastLoginAt)
                            .timestamp_with_time_zone()
                            .null(),
                    )
                    .to_owned(),
            )
            .await?;
        manager
            .create_index(
                Index::create()
                    .name("uq_users_normalized_username")
                    .table(Users::Table)
                    .col(Users::NormalizedUsername)
                    .unique()
                    .if_not_exists()
                    .to_owned(),
            )
            .await?;
        manager
            .create_index(
                Index::create()
                    .name("uq_users_email")
                    .table(Users::Table)
                    .col(Users::Email)
                    .unique()
                    .if_not_exists()
                    .to_owned(),
            )
            .await?;

        manager
            .create_table(
                Table::create()
                    .table(UserRoles::Table)
                    .if_not_exists()
                    .col(ColumnDef::new(UserRoles::UserId).string_len(36).not_null())
                    .col(ColumnDef::new(UserRoles::Role).string_len(32).not_null())
                    .primary_key(
                        Index::create()
                            .name("pk_user_roles")
                            .col(UserRoles::UserId)
                            .col(UserRoles::Role),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_user_roles_user")
                            .from(UserRoles::Table, UserRoles::UserId)
                            .to(Users::Table, Users::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            )
            .await?;

        manager
            .create_table(
                Table::create()
                    .table(Sessions::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(Sessions::TokenHash)
                            .string_len(64)
                            .primary_key(),
                    )
                    .col(ColumnDef::new(Sessions::UserId).string_len(36).not_null())
                    .col(ColumnDef::new(Sessions::CsrfHash).string_len(64).not_null())
                    .col(
                        ColumnDef::new(Sessions::CreatedAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(Sessions::LastSeenAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(Sessions::ExpiresAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_sessions_user")
                            .from(Sessions::Table, Sessions::UserId)
                            .to(Users::Table, Users::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            )
            .await?;
        manager
            .create_index(
                Index::create()
                    .name("idx_sessions_expires_at")
                    .table(Sessions::Table)
                    .col(Sessions::ExpiresAt)
                    .if_not_exists()
                    .to_owned(),
            )
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(Sessions::Table).if_exists().to_owned())
            .await?;
        manager
            .drop_table(Table::drop().table(UserRoles::Table).if_exists().to_owned())
            .await?;
        manager
            .drop_table(Table::drop().table(Users::Table).if_exists().to_owned())
            .await?;
        Ok(())
    }
}

#[derive(DeriveIden)]
enum Users {
    Table,
    Id,
    Username,
    NormalizedUsername,
    Email,
    PasswordHash,
    IsDisabled,
    CreatedAt,
    LastLoginAt,
}

#[derive(DeriveIden)]
enum UserRoles {
    Table,
    UserId,
    Role,
}

#[derive(DeriveIden)]
enum Sessions {
    Table,
    TokenHash,
    UserId,
    CsrfHash,
    CreatedAt,
    LastSeenAt,
    ExpiresAt,
}
