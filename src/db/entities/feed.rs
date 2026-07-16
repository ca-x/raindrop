use sea_orm::entity::prelude::*;
use time::OffsetDateTime;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "feeds")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: String,
    pub source_url: String,
    pub normalized_url: String,
    pub normalized_url_hash: String,
    pub fetch_url: String,
    pub validator_url: Option<String>,
    pub etag: Option<String>,
    pub last_modified: Option<String>,
    pub response_hash: Option<String>,
    pub entry_sequence_head: i64,
    pub last_attempt_at: Option<OffsetDateTime>,
    pub last_success_at: Option<OffsetDateTime>,
    pub last_changed_at: Option<OffsetDateTime>,
    pub next_fetch_at: OffsetDateTime,
    pub retry_after: Option<OffsetDateTime>,
    pub failure_count: i64,
    pub last_error: Option<String>,
    pub is_disabled: bool,
    pub orphaned_at: Option<OffsetDateTime>,
    pub lease_owner: Option<String>,
    pub lease_token: i64,
    pub lease_until: Option<OffsetDateTime>,
    pub created_at: OffsetDateTime,
    pub updated_at: OffsetDateTime,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
