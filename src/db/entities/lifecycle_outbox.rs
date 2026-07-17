use sea_orm::entity::prelude::*;
use time::OffsetDateTime;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "lifecycle_outbox")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: String,
    pub event_type: String,
    pub aggregate_type: String,
    pub aggregate_id: String,
    pub refresh_id: String,
    pub event_sequence: i32,
    pub payload_version: i32,
    pub payload_json: String,
    pub idempotency_key: String,
    pub status: String,
    pub available_at: OffsetDateTime,
    pub attempts: i32,
    pub lease_owner: Option<String>,
    pub lease_until: Option<OffsetDateTime>,
    pub created_at: OffsetDateTime,
    pub completed_at: Option<OffsetDateTime>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
