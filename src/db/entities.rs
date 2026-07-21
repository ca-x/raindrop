pub mod ai_provider;
pub mod category;
pub mod content_artifact;
pub mod content_job;
pub mod content_job_attempt;
pub mod content_job_result;
pub mod entry;
pub mod entry_state;
pub mod feed;
pub mod feed_refresh_run;
pub mod lifecycle_outbox;
pub mod plugin_capability_grant;
pub mod plugin_config;
pub mod plugin_installation;
pub mod plugin_kv;
pub mod rss_counter;
pub mod subscription;
pub mod user_font;
pub mod user_preference;

pub mod translation_config {
    use sea_orm::entity::prelude::*;
    use time::OffsetDateTime;

    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "translation_configs")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub user_id: String,
        pub engine: String,
        pub display_mode: String,
        pub is_enabled: bool,
        pub default_target_locale: String,
        pub open_ai_provider_id: Option<String>,
        pub open_ai_max_output_tokens: i32,
        pub open_ai_profile: String,
        pub open_ai_custom_system_prompt: Option<String>,
        pub open_ai_custom_prompt: Option<String>,
        pub deep_lx_display_name: String,
        pub deep_lx_description: Option<String>,
        pub deep_lx_base_url: Option<String>,
        pub encrypted_deep_lx_api_key: Option<String>,
        pub revision: i64,
        pub created_at: OffsetDateTime,
        pub updated_at: OffsetDateTime,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

pub mod bootstrap_state {
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "bootstrap_state")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: i32,
        pub administrator_user_id: String,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

pub mod user {
    use sea_orm::entity::prelude::*;
    use time::OffsetDateTime;

    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "users")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: String,
        pub username: String,
        pub normalized_username: String,
        pub display_name: Option<String>,
        pub email: Option<String>,
        pub password_hash: String,
        pub is_disabled: bool,
        pub created_at: OffsetDateTime,
        pub last_login_at: Option<OffsetDateTime>,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

pub mod user_role {
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "user_roles")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub user_id: String,
        #[sea_orm(primary_key, auto_increment = false)]
        pub role: String,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

pub mod session {
    use sea_orm::entity::prelude::*;
    use time::OffsetDateTime;

    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "sessions")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub token_hash: String,
        pub user_id: String,
        pub csrf_hash: String,
        pub created_at: OffsetDateTime,
        pub last_seen_at: OffsetDateTime,
        pub expires_at: OffsetDateTime,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}
