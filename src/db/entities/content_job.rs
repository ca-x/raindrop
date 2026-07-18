use sea_orm::entity::prelude::*;
use time::OffsetDateTime;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "content_jobs")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: String,
    pub user_id: String,
    pub entry_id: String,
    pub operation: String,
    pub artifact_kind: String,
    pub target_locale: Option<String>,
    pub trigger_kind: String,
    pub plugin_key: String,
    pub plugin_version: String,
    pub component_digest: String,
    pub provider_binding_id: String,
    pub provider_kind: String,
    pub provider_model: String,
    pub provider_revision: i64,
    pub prompt_version: String,
    pub schema_id: String,
    pub entry_content_hash: String,
    pub input_hash: String,
    pub config_hash: String,
    pub mcp_provenance_hash: String,
    pub artifact_identity_hash: String,
    pub idempotency_key: String,
    pub idempotency_key_hash: String,
    pub request_hash: String,
    pub call_chain_id: String,
    pub remaining_depth: i32,
    pub status: String,
    pub attempts: i32,
    pub max_attempts: i32,
    pub timeout_seconds: i32,
    pub next_attempt_at: OffsetDateTime,
    pub lease_owner: Option<String>,
    pub lease_token: i64,
    pub lease_until: Option<OffsetDateTime>,
    pub attempt_deadline_at: Option<OffsetDateTime>,
    pub last_error_code: Option<String>,
    pub created_at: OffsetDateTime,
    pub started_at: Option<OffsetDateTime>,
    pub completed_at: Option<OffsetDateTime>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
