#[allow(dead_code)]
mod support;

use raindrop::db::{
    entities::{user, user_font, user_preference},
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

    let large_font = vec![0x5a; 70 * 1024];
    user_font::ActiveModel {
        id: Set("00000000-0000-4000-8000-000000000701".to_owned()),
        user_id: Set(USER_A_ID.to_owned()),
        display_name: Set("Large Reader Font".to_owned()),
        normalized_name: Set("large reader font".to_owned()),
        content_hash: Set("a".repeat(64)),
        byte_size: Set(i32::try_from(large_font.len()).expect("font fixture size should fit")),
        font_bytes: Set(large_font.clone()),
        created_at: Set(CONTRACT_AT),
        updated_at: Set(CONTRACT_AT),
    }
    .insert(&database)
    .await
    .expect("font bytes above the MySQL BLOB limit should insert");
    let stored_font = user_font::Entity::find_by_id("00000000-0000-4000-8000-000000000701")
        .one(&database)
        .await
        .expect("large font should query")
        .expect("large font should exist");
    assert_eq!(stored_font.font_bytes, large_font);

    preference_model(
        USER_A_ID, "zh-CN", "SYSTEM", "BALANCED", 100, "SERIF", "AUTO", "NEW_TAB",
    )
    .insert(&database)
    .await
    .expect("valid preferences should insert");
    assert!(
        preference_model(
            USER_A_ID,
            "en",
            "DARK",
            "COMPACT",
            90,
            "SANS",
            "SEPIA",
            "CURRENT_TAB",
        )
        .insert(&database)
        .await
        .is_err(),
        "one preference row must exist per user"
    );

    for invalid in [
        preference_model(
            USER_A_ID, "fr", "SYSTEM", "BALANCED", 100, "SERIF", "AUTO", "NEW_TAB",
        ),
        preference_model(
            USER_A_ID, "en", "AUTO", "BALANCED", 100, "SERIF", "AUTO", "NEW_TAB",
        ),
        preference_model(
            USER_A_ID, "en", "SYSTEM", "DENSE", 100, "SERIF", "AUTO", "NEW_TAB",
        ),
        preference_model(
            USER_A_ID, "en", "SYSTEM", "BALANCED", 84, "SERIF", "AUTO", "NEW_TAB",
        ),
        preference_model(
            USER_A_ID, "en", "SYSTEM", "BALANCED", 131, "SERIF", "AUTO", "NEW_TAB",
        ),
        preference_model(
            USER_A_ID, "en", "SYSTEM", "BALANCED", 100, "MONO", "AUTO", "NEW_TAB",
        ),
        preference_model(
            USER_A_ID, "en", "SYSTEM", "BALANCED", 100, "SERIF", "BLUE", "NEW_TAB",
        ),
        preference_model(
            USER_A_ID, "en", "SYSTEM", "BALANCED", 100, "SERIF", "AUTO", "POPUP",
        ),
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
    assert_eq!(
        user_font::Entity::find()
            .count(&database)
            .await
            .expect("fonts should count after owner deletion"),
        0,
        "deleting a user must cascade to custom fonts"
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
        "reading_font_family",
        "reading_custom_font_id",
        "reading_color_scheme",
        "link_open_mode",
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
    assert!(
        manager
            .has_table("user_fonts")
            .await
            .expect("custom font table should inspect")
    );
    for column in [
        "id",
        "user_id",
        "display_name",
        "normalized_name",
        "content_hash",
        "font_bytes",
        "byte_size",
        "created_at",
        "updated_at",
    ] {
        assert!(
            manager
                .has_column("user_fonts", column)
                .await
                .unwrap_or_else(|_| panic!("custom font column {column} should inspect")),
            "custom font column {column} should exist"
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn preference_model(
    user_id: &str,
    locale: &str,
    theme_mode: &str,
    layout_density: &str,
    reading_font_scale: i32,
    reading_font_family: &str,
    reading_color_scheme: &str,
    link_open_mode: &str,
) -> user_preference::ActiveModel {
    user_preference::ActiveModel {
        user_id: Set(user_id.to_owned()),
        locale: Set(locale.to_owned()),
        theme_mode: Set(theme_mode.to_owned()),
        layout_density: Set(layout_density.to_owned()),
        reading_font_scale: Set(reading_font_scale),
        reading_font_family: Set(reading_font_family.to_owned()),
        reading_custom_font_id: Set(None),
        reading_color_scheme: Set(reading_color_scheme.to_owned()),
        link_open_mode: Set(link_open_mode.to_owned()),
        created_at: Set(CONTRACT_AT),
        updated_at: Set(CONTRACT_AT),
    }
}
