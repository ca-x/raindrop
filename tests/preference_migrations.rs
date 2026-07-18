#[allow(dead_code)]
mod support;

use raindrop::db::{
    entities::{user, user_preference},
    migrate, rollback,
};
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, DatabaseConnection, EntityTrait, PaginatorTrait,
};
use sea_orm_migration::SchemaManager;
use secrecy::SecretString;
use support::database::{USER_A_ID, USER_B_ID, connect_for_contract, insert_user};
use tempfile::tempdir;
use time::{OffsetDateTime, macros::datetime};

const CONTRACT_AT: OffsetDateTime = datetime!(2040-02-03 04:05:06.123456 UTC);

#[tokio::test]
async fn sqlite_preference_schema_contract() {
    let data = tempdir().expect("temporary directory should be created");
    let url = format!(
        "sqlite://{}?mode=rwc",
        data.path().join("preference-contract.db").display()
    );
    preference_schema_contract(SecretString::from(url)).await;
}

#[tokio::test]
async fn postgres_preference_schema_contract() {
    let Ok(url) = std::env::var("RAINDROP_TEST_POSTGRES_URL") else {
        eprintln!("postgres preference schema contract skipped: database URL is not configured");
        return;
    };
    preference_schema_contract(SecretString::from(url)).await;
}

#[tokio::test]
async fn mysql_preference_schema_contract() {
    let Ok(url) = std::env::var("RAINDROP_TEST_MYSQL_URL") else {
        eprintln!("mysql preference schema contract skipped: database URL is not configured");
        return;
    };
    preference_schema_contract(SecretString::from(url)).await;
}

async fn preference_schema_contract(database_url: SecretString) {
    let database = connect_for_contract(database_url).await;
    rollback(&database)
        .await
        .unwrap_or_else(|_| panic!("dedicated preference database should reset"));
    migrate(&database)
        .await
        .expect("preference migrations should apply");
    migrate(&database)
        .await
        .expect("preference migrations should be idempotent");

    assert_schema(&database).await;
    insert_user(&database, USER_A_ID, "preference-a").await;
    insert_user(&database, USER_B_ID, "preference-b").await;

    preference_model(USER_A_ID, "zh-CN", "SYSTEM", "BALANCED", 100)
        .insert(&database)
        .await
        .expect("valid preferences should insert");
    assert!(
        preference_model(USER_A_ID, "en", "DARK", "COMPACT", 90)
            .insert(&database)
            .await
            .is_err(),
        "one preference row must exist per user"
    );

    for invalid in [
        preference_model(USER_A_ID, "fr", "SYSTEM", "BALANCED", 100),
        preference_model(USER_A_ID, "en", "AUTO", "BALANCED", 100),
        preference_model(USER_A_ID, "en", "SYSTEM", "DENSE", 100),
        preference_model(USER_A_ID, "en", "SYSTEM", "BALANCED", 84),
        preference_model(USER_A_ID, "en", "SYSTEM", "BALANCED", 131),
    ] {
        assert!(
            invalid.update(&database).await.is_err(),
            "database constraints must reject invalid preferences"
        );
    }

    user::Entity::delete_by_id(USER_A_ID)
        .exec(&database)
        .await
        .expect("preference owner should delete");
    assert_eq!(
        user_preference::Entity::find()
            .count(&database)
            .await
            .expect("preferences should count after owner deletion"),
        0,
        "deleting a user must cascade to preferences"
    );

    rollback(&database)
        .await
        .expect("preference database should roll back");
    migrate(&database)
        .await
        .expect("preference migrations should reapply after rollback");
    assert_schema(&database).await;
    rollback(&database)
        .await
        .expect("reapplied preference database should roll back");
    database
        .close()
        .await
        .expect("preference contract database should close");
}

async fn assert_schema(database: &DatabaseConnection) {
    let manager = SchemaManager::new(database);
    assert!(
        manager
            .has_table("user_preferences")
            .await
            .expect("preference table should inspect")
    );
    for column in [
        "user_id",
        "locale",
        "theme_mode",
        "layout_density",
        "reading_font_scale",
        "created_at",
        "updated_at",
    ] {
        assert!(
            manager
                .has_column("user_preferences", column)
                .await
                .unwrap_or_else(|_| panic!("preference column {column} should inspect")),
            "preference column {column} should exist"
        );
    }
}

fn preference_model(
    user_id: &str,
    locale: &str,
    theme_mode: &str,
    layout_density: &str,
    reading_font_scale: i32,
) -> user_preference::ActiveModel {
    user_preference::ActiveModel {
        user_id: Set(user_id.to_owned()),
        locale: Set(locale.to_owned()),
        theme_mode: Set(theme_mode.to_owned()),
        layout_density: Set(layout_density.to_owned()),
        reading_font_scale: Set(reading_font_scale),
        created_at: Set(CONTRACT_AT),
        updated_at: Set(CONTRACT_AT),
    }
}
