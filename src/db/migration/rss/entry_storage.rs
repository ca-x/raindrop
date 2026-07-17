use sea_orm::{ConnectionTrait, DatabaseBackend, DbBackend, Statement};
use sea_orm_migration::prelude::*;
use serde::Serialize;

use crate::feeds::EntryContentDetail;

const MIGRATED_STORAGE_PREFIX: &str = "rdsc:v1:";
const MYSQL_TEXT_MAX_BYTES: usize = 65_535;
const MIGRATION_BATCH_SIZE: usize = 32;

#[derive(DeriveMigrationName)]
pub struct EntryStorage;

#[async_trait::async_trait]
impl MigrationTrait for EntryStorage {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        if manager.get_database_backend() == DatabaseBackend::MySql {
            widen_mysql_columns(manager).await?;
        }
        backfill_legacy_content(manager).await?;
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        validate_rollback_content(manager).await?;
        if manager.get_database_backend() == DatabaseBackend::MySql {
            reject_oversized_mysql_legacy_values(manager).await?;
        }
        restore_legacy_content(manager).await?;
        if manager.get_database_backend() == DatabaseBackend::MySql {
            narrow_mysql_columns(manager).await?;
        }
        Ok(())
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct LegacyEnvelope<'a> {
    html: &'a str,
    inert_images: [(); 0],
}

async fn backfill_legacy_content(manager: &SchemaManager<'_>) -> Result<(), DbErr> {
    let backend = manager.get_database_backend();
    let mut last_id = String::new();
    loop {
        let rows = content_batch(manager, &last_id).await?;
        if rows.is_empty() {
            break;
        }
        for row in rows {
            let id: String = row.try_get("", "id")?;
            let content: String = row.try_get("", "sanitized_content")?;
            if content.starts_with(MIGRATED_STORAGE_PREFIX) {
                EntryContentDetail::decode(&content)
                    .map_err(|_| migration_error("existing entry content envelope is invalid"))?;
                last_id = id;
                continue;
            }

            let envelope = encode_legacy_envelope(&content)?;
            manager
                .get_connection()
                .execute(update_content_statement(backend, &id, &envelope))
                .await?;
            last_id = id;
        }
    }
    Ok(())
}

async fn validate_rollback_content(manager: &SchemaManager<'_>) -> Result<(), DbErr> {
    let mut last_id = String::new();
    loop {
        let rows = content_batch(manager, &last_id).await?;
        if rows.is_empty() {
            break;
        }
        for row in rows {
            let id: String = row.try_get("", "id")?;
            let storage: String = row.try_get("", "sanitized_content")?;
            let _ = decode_legacy_html(manager.get_database_backend(), &storage)?;
            last_id = id;
        }
    }
    Ok(())
}

async fn restore_legacy_content(manager: &SchemaManager<'_>) -> Result<(), DbErr> {
    let backend = manager.get_database_backend();
    let mut last_id = String::new();
    loop {
        let rows = content_batch(manager, &last_id).await?;
        if rows.is_empty() {
            break;
        }
        for row in rows {
            let id: String = row.try_get("", "id")?;
            let storage: String = row.try_get("", "sanitized_content")?;
            let html = decode_legacy_html(backend, &storage)?;
            manager
                .get_connection()
                .execute(update_content_statement(backend, &id, &html))
                .await?;
            last_id = id;
        }
    }
    Ok(())
}

async fn content_batch(
    manager: &SchemaManager<'_>,
    last_id: &str,
) -> Result<Vec<sea_orm::QueryResult>, DbErr> {
    let backend = manager.get_database_backend();
    let sql = match backend {
        DatabaseBackend::Postgres => {
            "SELECT id, sanitized_content FROM entries WHERE id > $1 ORDER BY id LIMIT 32"
        }
        DatabaseBackend::Sqlite | DatabaseBackend::MySql => {
            "SELECT id, sanitized_content FROM entries WHERE id > ? ORDER BY id LIMIT 32"
        }
    };
    let rows = manager
        .get_connection()
        .query_all(Statement::from_sql_and_values(
            backend,
            sql,
            [last_id.into()],
        ))
        .await?;
    if rows.len() > MIGRATION_BATCH_SIZE {
        return Err(migration_error(
            "entry content migration batch exceeded its bound",
        ));
    }
    Ok(rows)
}

fn encode_legacy_envelope(content: &str) -> Result<String, DbErr> {
    let json = serde_json::to_string(&LegacyEnvelope {
        html: content,
        inert_images: [],
    })
    .map_err(|_| migration_error("legacy entry content could not be encoded"))?;
    let envelope = format!("rdsc:v1:{json}");
    EntryContentDetail::decode(&envelope)
        .map_err(|_| migration_error("legacy entry content is not valid sanitized HTML"))?;
    Ok(envelope)
}

fn decode_legacy_html(backend: DbBackend, storage: &str) -> Result<String, DbErr> {
    let html = if storage.starts_with(MIGRATED_STORAGE_PREFIX) {
        let detail = EntryContentDetail::decode(storage)
            .map_err(|_| migration_error("entry content envelope is invalid"))?;
        if !detail.inert_images().is_empty() {
            return Err(migration_error(
                "entry content with inert images cannot be rolled back",
            ));
        }
        detail.html().to_owned()
    } else {
        // A previous MySQL down attempt may already have restored this row.
        encode_legacy_envelope(storage)?;
        storage.to_owned()
    };
    if backend == DatabaseBackend::MySql && html.len() > MYSQL_TEXT_MAX_BYTES {
        return Err(migration_error(
            "entry content exceeds the legacy MySQL TEXT capacity",
        ));
    }
    Ok(html)
}

fn update_content_statement(backend: DbBackend, id: &str, content: &str) -> Statement {
    let sql = match backend {
        DatabaseBackend::Postgres => "UPDATE entries SET sanitized_content = $1 WHERE id = $2",
        DatabaseBackend::Sqlite | DatabaseBackend::MySql => {
            "UPDATE entries SET sanitized_content = ? WHERE id = ?"
        }
    };
    Statement::from_sql_and_values(backend, sql, [content.into(), id.into()])
}

struct MySqlColumn {
    name: &'static str,
    wide_type: &'static str,
    nullable: bool,
}

const MYSQL_COLUMNS: [MySqlColumn; 6] = [
    MySqlColumn {
        name: "sanitized_content",
        wide_type: "LONGTEXT",
        nullable: false,
    },
    MySqlColumn {
        name: "identity",
        wide_type: "MEDIUMTEXT",
        nullable: false,
    },
    MySqlColumn {
        name: "title",
        wide_type: "MEDIUMTEXT",
        nullable: true,
    },
    MySqlColumn {
        name: "author",
        wide_type: "MEDIUMTEXT",
        nullable: true,
    },
    MySqlColumn {
        name: "summary",
        wide_type: "MEDIUMTEXT",
        nullable: true,
    },
    MySqlColumn {
        name: "enclosure_json",
        wide_type: "MEDIUMTEXT",
        nullable: true,
    },
];

async fn widen_mysql_columns(manager: &SchemaManager<'_>) -> Result<(), DbErr> {
    for column in &MYSQL_COLUMNS {
        alter_mysql_column_if_needed(manager, column, column.wide_type).await?;
    }
    Ok(())
}

async fn narrow_mysql_columns(manager: &SchemaManager<'_>) -> Result<(), DbErr> {
    for column in &MYSQL_COLUMNS {
        alter_mysql_column_if_needed(manager, column, "TEXT").await?;
    }
    Ok(())
}

async fn alter_mysql_column_if_needed(
    manager: &SchemaManager<'_>,
    column: &MySqlColumn,
    target_type: &str,
) -> Result<(), DbErr> {
    let row = manager
        .get_connection()
        .query_one(Statement::from_sql_and_values(
            DatabaseBackend::MySql,
            "SELECT data_type, is_nullable
             FROM information_schema.columns
             WHERE table_schema = DATABASE() AND table_name = 'entries' AND column_name = ?",
            [column.name.into()],
        ))
        .await?
        .ok_or_else(|| migration_error("expected MySQL entry column is missing"))?;
    let data_type: String = row.try_get("", "data_type")?;
    let is_nullable: String = row.try_get("", "is_nullable")?;
    if (is_nullable == "YES") != column.nullable {
        return Err(migration_error(
            "MySQL entry column nullability does not match the schema contract",
        ));
    }
    if data_type.eq_ignore_ascii_case(target_type) {
        return Ok(());
    }
    let nullability = if column.nullable { "NULL" } else { "NOT NULL" };
    manager
        .get_connection()
        .execute(Statement::from_string(
            DatabaseBackend::MySql,
            format!(
                "ALTER TABLE entries MODIFY COLUMN {} {target_type} {nullability}",
                column.name
            ),
        ))
        .await?;
    Ok(())
}

async fn reject_oversized_mysql_legacy_values(manager: &SchemaManager<'_>) -> Result<(), DbErr> {
    let row = manager
        .get_connection()
        .query_one(Statement::from_string(
            DatabaseBackend::MySql,
            "SELECT COUNT(*) AS oversized_count
             FROM entries
             WHERE OCTET_LENGTH(identity) > 65535
                OR OCTET_LENGTH(title) > 65535
                OR OCTET_LENGTH(author) > 65535
                OR OCTET_LENGTH(summary) > 65535
                OR OCTET_LENGTH(enclosure_json) > 65535"
                .to_owned(),
        ))
        .await?
        .ok_or_else(|| migration_error("MySQL entry width validation did not return a row"))?;
    let oversized_count: i64 = row.try_get("", "oversized_count")?;
    if oversized_count != 0 {
        return Err(migration_error(
            "entry values exceed the legacy MySQL TEXT capacity",
        ));
    }
    Ok(())
}

fn migration_error(message: &str) -> DbErr {
    DbErr::Migration(message.to_owned())
}
