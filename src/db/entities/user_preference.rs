use sea_orm::entity::prelude::*;
use time::OffsetDateTime;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "user_preferences")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub user_id: String,
    pub locale: String,
    pub theme_mode: String,
    pub layout_density: String,
    pub reading_font_scale: i32,
    pub reading_font_family: String,
    pub reading_color_scheme: String,
    pub link_open_mode: String,
    pub created_at: OffsetDateTime,
    pub updated_at: OffsetDateTime,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
