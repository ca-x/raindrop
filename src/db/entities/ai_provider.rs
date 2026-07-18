use sea_orm::entity::prelude::*;
use time::OffsetDateTime;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "ai_providers")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: String,
    pub owner_user_id: Option<String>,
    pub display_name: String,
    pub kind: String,
    pub endpoint: String,
    pub model: String,
    pub encrypted_secret: String,
    pub supports_usage: bool,
    pub supports_idempotency: bool,
    pub supports_streaming: bool,
    pub max_concurrency: i32,
    pub requests_per_minute: Option<i32>,
    pub max_input_tokens_per_request: i32,
    pub max_output_tokens_per_request: i32,
    pub input_cost_micros_per_million_tokens: Option<i64>,
    pub output_cost_micros_per_million_tokens: Option<i64>,
    pub max_cost_micros_per_request: Option<i64>,
    pub is_enabled: bool,
    pub revision: i64,
    pub created_at: OffsetDateTime,
    pub updated_at: OffsetDateTime,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
