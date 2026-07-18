use sea_orm::{ConnectionTrait, DatabaseBackend, DbBackend, Statement};
use sea_orm_migration::prelude::*;

use crate::{content::search::build_entry_search_text, feeds::EntryContentDetail};

const MIGRATION_BATCH_SIZE: usize = 32;

#[derive(DeriveMigrationName)]
pub struct EntrySearch;

#[async_trait::async_trait]
impl MigrationTrait for EntrySearch {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let backend = manager.get_database_backend();
        if !manager.has_column("entries", "search_text").await? {
            manager
                .alter_table(add_search_text_statement(backend))
                .await?;
        }
        backfill_search_text(manager).await?;
        if backend == DatabaseBackend::MySql {
            manager
                .alter_table(enforce_mysql_search_text_not_null_statement())
                .await?;
        }
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        if manager.has_column("entries", "search_text").await? {
            manager
                .alter_table(
                    Table::alter()
                        .table(Entries::Table)
                        .drop_column(Entries::SearchText)
                        .to_owned(),
                )
                .await?;
        }
        Ok(())
    }
}

fn add_search_text_statement(backend: DbBackend) -> TableAlterStatement {
    let mut statement = Table::alter();
    statement.table(Entries::Table);
    match backend {
        DatabaseBackend::MySql => {
            statement.add_column(ColumnDef::new(Entries::SearchText).text().null());
        }
        DatabaseBackend::Postgres | DatabaseBackend::Sqlite => {
            statement.add_column(
                ColumnDef::new(Entries::SearchText)
                    .text()
                    .not_null()
                    .default(""),
            );
        }
    }
    statement.to_owned()
}

fn enforce_mysql_search_text_not_null_statement() -> TableAlterStatement {
    Table::alter()
        .table(Entries::Table)
        .modify_column(ColumnDef::new(Entries::SearchText).text().not_null())
        .to_owned()
}

async fn backfill_search_text(manager: &SchemaManager<'_>) -> Result<(), DbErr> {
    let backend = manager.get_database_backend();
    let mut last_id = String::new();
    loop {
        let rows = search_batch(manager, &last_id).await?;
        if rows.is_empty() {
            break;
        }
        for row in rows {
            let id: String = row.try_get("", "id")?;
            let title: Option<String> = row.try_get("", "title")?;
            let author: Option<String> = row.try_get("", "author")?;
            let summary: Option<String> = row.try_get("", "summary")?;
            let storage: String = row.try_get("", "sanitized_content")?;
            let content = EntryContentDetail::decode(&storage)
                .map_err(|_| migration_error("entry search source content is invalid"))?;
            let search_text = build_entry_search_text(
                title.as_deref(),
                author.as_deref(),
                summary.as_deref(),
                content.html(),
            );
            manager
                .get_connection()
                .execute(update_search_statement(backend, &id, &search_text))
                .await?;
            last_id = id;
        }
    }
    Ok(())
}

async fn search_batch(
    manager: &SchemaManager<'_>,
    last_id: &str,
) -> Result<Vec<sea_orm::QueryResult>, DbErr> {
    let backend = manager.get_database_backend();
    let sql = match backend {
        DatabaseBackend::Postgres => {
            "SELECT id, title, author, summary, sanitized_content
             FROM entries WHERE id > $1 ORDER BY id LIMIT 32"
        }
        DatabaseBackend::Sqlite | DatabaseBackend::MySql => {
            "SELECT id, title, author, summary, sanitized_content
             FROM entries WHERE id > ? ORDER BY id LIMIT 32"
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
            "entry search migration batch exceeded its bound",
        ));
    }
    Ok(rows)
}

fn update_search_statement(backend: DbBackend, id: &str, search_text: &str) -> Statement {
    let sql = match backend {
        DatabaseBackend::Postgres => "UPDATE entries SET search_text = $1 WHERE id = $2",
        DatabaseBackend::Sqlite | DatabaseBackend::MySql => {
            "UPDATE entries SET search_text = ? WHERE id = ?"
        }
    };
    Statement::from_sql_and_values(backend, sql, [search_text.into(), id.into()])
}

fn migration_error(message: &str) -> DbErr {
    DbErr::Migration(message.to_owned())
}

#[derive(DeriveIden)]
enum Entries {
    Table,
    SearchText,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mysql_search_text_evolution_avoids_text_defaults() {
        let add = add_search_text_statement(DatabaseBackend::MySql)
            .to_string(MysqlQueryBuilder)
            .to_ascii_uppercase();
        assert!(add.contains("ADD COLUMN"));
        assert!(!add.contains("DEFAULT"));
        assert!(!add.contains("NOT NULL"));

        let enforce = enforce_mysql_search_text_not_null_statement()
            .to_string(MysqlQueryBuilder)
            .to_ascii_uppercase();
        assert!(enforce.contains("MODIFY COLUMN"));
        assert!(enforce.contains("NOT NULL"));
        assert!(!enforce.contains("DEFAULT"));
    }
}
