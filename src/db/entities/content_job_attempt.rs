use sea_orm::entity::prelude::*;
use time::OffsetDateTime;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "content_job_attempts")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: String,
    pub job_id: String,
    pub attempt: i32,
    pub lease_token: i64,
    pub status: String,
    pub started_at: OffsetDateTime,
    pub deadline_at: OffsetDateTime,
    pub completed_at: Option<OffsetDateTime>,
    pub error_code: Option<String>,
    pub retryable: Option<bool>,
    pub outcome_unknown: bool,
    pub provider_request_count: i32,
    pub mcp_call_count: i32,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub estimated_cost_micros: i64,
    pub execution_metadata_json: String,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
