#[allow(dead_code)]
mod support;

use raindrop::db::{
    entities::{category, subscription, user},
    migrate, rollback,
};
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, DatabaseConnection, EntityTrait,
    PaginatorTrait, QueryFilter,
};
use sea_orm_migration::SchemaManager;
use secrecy::SecretString;
use support::database::{
    FEED_ID, SUBSCRIPTION_A_ID, USER_A_ID, USER_B_ID, connect_for_contract, insert_feed,
    insert_user, subscription_model,
};
use tempfile::tempdir;
use time::{OffsetDateTime, macros::datetime};

const CATEGORY_A_ID: &str = "00000000-0000-4000-8000-000000000501";
const CATEGORY_B_ID: &str = "00000000-0000-4000-8000-000000000502";
const CATEGORY_DUPLICATE_ID: &str = "00000000-0000-4000-8000-000000000503";
const CONTRACT_AT: OffsetDateTime = datetime!(2040-02-03 04:05:06.123456 UTC);

#[tokio::test]
async fn sqlite_organization_schema_contract() {
    let data = tempdir().expect("temporary directory should be created");
    let url = format!(
        "sqlite://{}?mode=rwc",
        data.path().join("organization-contract.db").display()
    );
    organization_schema_contract(SecretString::from(url)).await;
}

#[tokio::test]
async fn postgres_organization_schema_contract() {
    let Ok(url) = std::env::var("RAINDROP_TEST_POSTGRES_URL") else {
        eprintln!("postgres organization schema contract skipped: database URL is not configured");
        return;
    };
    organization_schema_contract(SecretString::from(url)).await;
}

#[tokio::test]
async fn mysql_organization_schema_contract() {
    let Ok(url) = std::env::var("RAINDROP_TEST_MYSQL_URL") else {
        eprintln!("mysql organization schema contract skipped: database URL is not configured");
        return;
    };
    organization_schema_contract(SecretString::from(url)).await;
}

async fn organization_schema_contract(database_url: SecretString) {
    let database = connect_for_contract(database_url).await;
    rollback(&database)
        .await
        .unwrap_or_else(|_| panic!("dedicated organization database should reset"));
    migrate(&database)
        .await
        .expect("organization migrations should apply");
    migrate(&database)
        .await
        .expect("organization migrations should be idempotent");

    assert_schema(&database).await;
    insert_user(&database, USER_A_ID, "category-a").await;
    insert_user(&database, USER_B_ID, "category-b").await;
    insert_feed(&database, CONTRACT_AT).await;

    category_model(CATEGORY_A_ID, USER_A_ID, "Technology", "technology", 1024)
        .insert(&database)
        .await
        .expect("user A category should insert");
    category_model(CATEGORY_B_ID, USER_B_ID, "Technology", "technology", 1024)
        .insert(&database)
        .await
        .expect("the same normalized title should be allowed for another user");
    assert!(
        category_model(
            CATEGORY_DUPLICATE_ID,
            USER_A_ID,
            "TECHNOLOGY",
            "technology",
            2048,
        )
        .insert(&database)
        .await
        .is_err(),
        "the same normalized title must be unique inside one user scope"
    );

    let mut subscription = subscription_model(SUBSCRIPTION_A_ID, USER_A_ID, CONTRACT_AT);
    subscription.category_id = Set(Some(CATEGORY_A_ID.to_owned()));
    subscription
        .insert(&database)
        .await
        .expect("categorized subscription should insert");

    category::Entity::delete_by_id(CATEGORY_A_ID)
        .exec(&database)
        .await
        .expect("category should delete");
    let stored = subscription::Entity::find_by_id(SUBSCRIPTION_A_ID)
        .one(&database)
        .await
        .expect("subscription should query after category deletion")
        .expect("subscription should survive category deletion");
    assert_eq!(stored.category_id, None);
    assert_eq!(stored.feed_id, FEED_ID);

    user::Entity::delete_by_id(USER_B_ID)
        .exec(&database)
        .await
        .expect("user B should delete");
    assert_eq!(
        category::Entity::find()
            .filter(category::Column::UserId.eq(USER_B_ID))
            .count(&database)
            .await
            .expect("user B categories should count"),
        0
    );

    rollback(&database)
        .await
        .expect("organization database should roll back");
    migrate(&database)
        .await
        .expect("organization migrations should reapply after rollback");
    assert_schema(&database).await;
    rollback(&database)
        .await
        .expect("reapplied organization database should roll back");
    database
        .close()
        .await
        .expect("organization contract database should close");
}

async fn assert_schema(database: &DatabaseConnection) {
    let manager = SchemaManager::new(database);
    assert!(
        manager
            .has_table("categories")
            .await
            .expect("categories table should inspect")
    );
    assert!(
        manager
            .has_column("subscriptions", "category_id")
            .await
            .expect("subscription category column should inspect")
    );
    for index in [
        "uq_categories_user_normalized_title",
        "idx_categories_user_position",
        "idx_subscriptions_user_category_position",
    ] {
        assert!(
            manager
                .has_index(
                    if index.starts_with("idx_subscriptions") {
                        "subscriptions"
                    } else {
                        "categories"
                    },
                    index
                )
                .await
                .unwrap_or_else(|_| panic!("organization index {index} should inspect")),
            "organization index {index} should exist"
        );
    }
}

fn category_model(
    id: &str,
    user_id: &str,
    title: &str,
    normalized_title: &str,
    position: i64,
) -> category::ActiveModel {
    category::ActiveModel {
        id: Set(id.to_owned()),
        user_id: Set(user_id.to_owned()),
        title: Set(title.to_owned()),
        normalized_title: Set(normalized_title.to_owned()),
        position: Set(position),
        created_at: Set(CONTRACT_AT),
        updated_at: Set(CONTRACT_AT),
    }
}
