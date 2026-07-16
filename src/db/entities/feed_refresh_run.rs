use sea_orm::entity::prelude::*;
use time::OffsetDateTime;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "feed_refresh_runs")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: String,
    pub feed_id: String,
    pub requested_by_user_id: Option<String>,
    pub trigger_kind: String,
    pub status: String,
    pub idempotency_key: String,
    pub lease_token: Option<i64>,
    pub commit_generation: Option<i64>,
    pub queued_at: OffsetDateTime,
    pub started_at: Option<OffsetDateTime>,
    pub fetched_at: Option<OffsetDateTime>,
    pub persisted_at: Option<OffsetDateTime>,
    pub completed_at: Option<OffsetDateTime>,
    pub http_status: Option<i32>,
    pub new_count: i32,
    pub updated_count: i32,
    pub dropped_count: i32,
    pub error_code: Option<String>,
    pub retry_at: Option<OffsetDateTime>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
