use sea_orm::entity::prelude::*;
use time::OffsetDateTime;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "plugin_kv")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub plugin_id: String,
    #[sea_orm(primary_key, auto_increment = false)]
    pub owner_user_id: String,
    #[sea_orm(primary_key, auto_increment = false)]
    pub key: String,
    pub value: Vec<u8>,
    pub value_size_bytes: i32,
    pub revision: i64,
    pub created_at: OffsetDateTime,
    pub updated_at: OffsetDateTime,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
