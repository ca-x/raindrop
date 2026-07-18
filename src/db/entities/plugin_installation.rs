use sea_orm::entity::prelude::*;
use time::OffsetDateTime;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "plugin_installations")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: String,
    pub plugin_key: String,
    pub version: String,
    pub abi_version: String,
    pub distribution: String,
    pub component_digest: String,
    pub manifest_json: String,
    pub signature_key_id: String,
    pub signature: String,
    pub system_state: String,
    pub revision: i64,
    pub installed_at: OffsetDateTime,
    pub updated_at: OffsetDateTime,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
