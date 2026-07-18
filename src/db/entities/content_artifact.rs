use sea_orm::entity::prelude::*;
use time::OffsetDateTime;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "content_artifacts")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: String,
    pub user_id: String,
    pub entry_id: String,
    pub producer_job_id: String,
    pub kind: String,
    pub locale: Option<String>,
    pub schema_id: String,
    pub entry_content_hash: String,
    pub input_hash: String,
    pub config_hash: String,
    pub processor_key: String,
    pub processor_version: String,
    pub component_digest: String,
    pub provider_binding_id: String,
    pub provider_kind: String,
    pub provider_model: String,
    pub provider_revision: i64,
    pub provider_label: String,
    pub prompt_version: String,
    pub mcp_provenance_hash: String,
    pub identity_hash: String,
    pub payload_json: String,
    pub provenance_json: String,
    pub payload_size_bytes: i32,
    pub created_at: OffsetDateTime,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
