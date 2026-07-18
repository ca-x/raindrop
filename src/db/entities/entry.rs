use sea_orm::entity::prelude::*;
use time::OffsetDateTime;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "entries")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: String,
    pub feed_id: String,
    pub feed_sequence: i64,
    pub ingest_generation: i64,
    pub identity_kind: String,
    pub identity: String,
    pub identity_hash: String,
    pub canonical_url: Option<String>,
    pub title: Option<String>,
    pub author: Option<String>,
    pub sanitized_content: String,
    pub search_text: String,
    pub summary: Option<String>,
    pub published_at_us: Option<i64>,
    pub sort_at_us: i64,
    pub inserted_at: OffsetDateTime,
    pub updated_at: OffsetDateTime,
    pub source_content_hash: String,
    pub content_hash: String,
    pub pipeline_version: String,
    pub direction: Option<String>,
    pub enclosure_json: Option<String>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
