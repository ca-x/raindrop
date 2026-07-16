use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct CreateRssCounters;

#[async_trait::async_trait]
impl MigrationTrait for CreateRssCounters {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(RssCounters::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(RssCounters::Key)
                            .string_len(32)
                            .primary_key(),
                    )
                    .col(ColumnDef::new(RssCounters::Value).big_integer().not_null())
                    .to_owned(),
            )
            .await?;

        manager
            .exec_stmt(
                Query::insert()
                    .into_table(RssCounters::Table)
                    .columns([RssCounters::Key, RssCounters::Value])
                    .values_panic(["INGEST_GENERATION".into(), 0_i64.into()])
                    .on_conflict(OnConflict::column(RssCounters::Key).do_nothing().to_owned())
                    .to_owned(),
            )
            .await?;

        let backend = manager.get_database_backend();
        let generation_query = Query::select()
            .column(RssCounters::Value)
            .from(RssCounters::Table)
            .and_where(Expr::col(RssCounters::Key).eq("INGEST_GENERATION"))
            .to_owned();
        let generation = manager
            .get_connection()
            .query_one(backend.build(&generation_query))
            .await?
            .ok_or_else(|| DbErr::Migration("INGEST_GENERATION seed row is missing".to_owned()))?;
        let value: i64 = generation.try_get("", "value")?;
        if value < 0 {
            return Err(DbErr::Migration(
                "INGEST_GENERATION must be a non-negative BIGINT".to_owned(),
            ));
        }

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(
                Table::drop()
                    .table(RssCounters::Table)
                    .if_exists()
                    .to_owned(),
            )
            .await
    }
}

#[derive(DeriveIden)]
enum RssCounters {
    Table,
    Key,
    Value,
}
