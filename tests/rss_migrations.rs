use raindrop::db::{
    DatabaseConfig, connect,
    entities::{entry, entry_state, feed, rss_counter, subscription, user},
    migrate,
};
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter,
};
use secrecy::SecretString;
use tempfile::tempdir;
use time::{OffsetDateTime, macros::datetime};

const USER_A_ID: &str = "00000000-0000-4000-8000-000000000001";
const USER_B_ID: &str = "00000000-0000-4000-8000-000000000002";
const FEED_ID: &str = "00000000-0000-4000-8000-000000000101";
const SUBSCRIPTION_A_ID: &str = "00000000-0000-4000-8000-000000000201";
const SUBSCRIPTION_B_ID: &str = "00000000-0000-4000-8000-000000000202";
const ENTRY_ID: &str = "00000000-0000-4000-8000-000000000301";
const NOW: OffsetDateTime = datetime!(2026-07-16 12:00:00 UTC);
const HASH_A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const HASH_B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
const HASH_C: &str = "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";

#[tokio::test]
async fn sqlite_rss_migrations_are_idempotent_and_seed_generation() {
    let data = tempdir().expect("temporary directory should be created");
    let database_path = data.path().join("raindrop.db");
    let url = format!("sqlite://{}?mode=rwc", database_path.display());
    let database = connect(&DatabaseConfig::new(SecretString::from(url)))
        .await
        .expect("SQLite should connect");

    migrate(&database).await.expect("migration should pass");
    migrate(&database)
        .await
        .expect("migration should be idempotent");

    let generation = rss_counter::Entity::find_by_id("INGEST_GENERATION")
        .one(&database)
        .await
        .expect("generation counter should query")
        .expect("generation counter should exist");
    assert_eq!(generation.value, 0);

    insert_user(&database, USER_A_ID, "reader-a").await;
    insert_feed(&database).await;
    insert_subscription(&database, SUBSCRIPTION_A_ID, USER_A_ID).await;
    insert_entry(&database).await;
    insert_entry_state(&database, USER_A_ID, 1)
        .await
        .expect("valid entry state should insert");
}

#[tokio::test]
async fn rss_schema_shares_feeds_but_rejects_duplicate_user_subscription() {
    let data = tempdir().expect("temporary directory should be created");
    let database_path = data.path().join("raindrop.db");
    let url = format!("sqlite://{}?mode=rwc", database_path.display());
    let database = connect(&DatabaseConfig::new(SecretString::from(url)))
        .await
        .expect("SQLite should connect");
    migrate(&database).await.expect("migration should pass");

    insert_user(&database, USER_A_ID, "reader-a").await;
    insert_user(&database, USER_B_ID, "reader-b").await;
    insert_feed(&database).await;
    insert_subscription(&database, SUBSCRIPTION_A_ID, USER_A_ID).await;
    insert_subscription(&database, SUBSCRIPTION_B_ID, USER_B_ID).await;

    let duplicate = subscription::ActiveModel {
        id: Set("00000000-0000-4000-8000-000000000203".to_owned()),
        user_id: Set(USER_A_ID.to_owned()),
        feed_id: Set(FEED_ID.to_owned()),
        title_override: Set(Some("Duplicate".to_owned())),
        position: Set(1),
        start_sequence: Set(1),
        read_through_sequence: Set(0),
        state_revision: Set(0),
        created_at: Set(NOW),
        updated_at: Set(NOW),
    }
    .insert(&database)
    .await;

    assert!(duplicate.is_err());
    assert_eq!(
        feed::Entity::find()
            .count(&database)
            .await
            .expect("feeds should count"),
        1
    );
}

#[tokio::test]
async fn rss_state_foreign_keys_reject_cross_user_and_mismatched_entry_rows() {
    let data = tempdir().expect("temporary directory should be created");
    let database_path = data.path().join("raindrop.db");
    let url = format!("sqlite://{}?mode=rwc", database_path.display());
    let database = connect(&DatabaseConfig::new(SecretString::from(url)))
        .await
        .expect("SQLite should connect");
    migrate(&database).await.expect("migration should pass");

    insert_user(&database, USER_A_ID, "reader-a").await;
    insert_user(&database, USER_B_ID, "reader-b").await;
    insert_feed(&database).await;
    insert_subscription(&database, SUBSCRIPTION_A_ID, USER_A_ID).await;
    insert_entry(&database).await;

    assert!(insert_entry_state(&database, USER_B_ID, 1).await.is_err());

    insert_subscription(&database, SUBSCRIPTION_B_ID, USER_B_ID).await;
    assert!(insert_entry_state(&database, USER_B_ID, 2).await.is_err());
}

#[tokio::test]
async fn deleting_a_user_cascades_subscription_and_state_without_deleting_shared_feed_entries() {
    let data = tempdir().expect("temporary directory should be created");
    let database_path = data.path().join("raindrop.db");
    let url = format!("sqlite://{}?mode=rwc", database_path.display());
    let database = connect(&DatabaseConfig::new(SecretString::from(url)))
        .await
        .expect("SQLite should connect");
    migrate(&database).await.expect("migration should pass");

    insert_user(&database, USER_A_ID, "reader-a").await;
    insert_user(&database, USER_B_ID, "reader-b").await;
    insert_feed(&database).await;
    insert_subscription(&database, SUBSCRIPTION_A_ID, USER_A_ID).await;
    insert_subscription(&database, SUBSCRIPTION_B_ID, USER_B_ID).await;
    insert_entry(&database).await;
    insert_entry_state(&database, USER_A_ID, 1)
        .await
        .expect("user A state should insert");
    insert_entry_state(&database, USER_B_ID, 1)
        .await
        .expect("user B state should insert");

    user::Entity::delete_by_id(USER_A_ID)
        .exec(&database)
        .await
        .expect("user A should delete");

    assert_eq!(
        subscription::Entity::find()
            .filter(subscription::Column::UserId.eq(USER_A_ID))
            .count(&database)
            .await
            .expect("user A subscriptions should count"),
        0
    );
    assert_eq!(
        entry_state::Entity::find()
            .filter(entry_state::Column::UserId.eq(USER_A_ID))
            .count(&database)
            .await
            .expect("user A states should count"),
        0
    );
    assert_eq!(
        feed::Entity::find()
            .count(&database)
            .await
            .expect("feeds should count"),
        1
    );
    assert_eq!(
        entry::Entity::find()
            .count(&database)
            .await
            .expect("entries should count"),
        1
    );
    assert_eq!(
        subscription::Entity::find()
            .filter(subscription::Column::UserId.eq(USER_B_ID))
            .count(&database)
            .await
            .expect("user B subscriptions should count"),
        1
    );
    assert_eq!(
        entry_state::Entity::find()
            .filter(entry_state::Column::UserId.eq(USER_B_ID))
            .count(&database)
            .await
            .expect("user B states should count"),
        1
    );
}

async fn insert_user(database: &sea_orm::DatabaseConnection, id: &str, username: &str) {
    user::ActiveModel {
        id: Set(id.to_owned()),
        username: Set(username.to_owned()),
        normalized_username: Set(username.to_owned()),
        email: Set(None),
        password_hash: Set("test-hash".to_owned()),
        is_disabled: Set(false),
        created_at: Set(NOW),
        last_login_at: Set(None),
    }
    .insert(database)
    .await
    .expect("user should insert");
}

async fn insert_feed(database: &sea_orm::DatabaseConnection) {
    feed::ActiveModel {
        id: Set(FEED_ID.to_owned()),
        source_url: Set("https://example.com/feed.xml".to_owned()),
        normalized_url: Set("https://example.com/feed.xml".to_owned()),
        normalized_url_hash: Set(HASH_A.to_owned()),
        fetch_url: Set("https://cdn.example.com/feed.xml".to_owned()),
        validator_url: Set(Some("https://cdn.example.com/feed.xml".to_owned())),
        etag: Set(Some("\"feed-v1\"".to_owned())),
        last_modified: Set(Some("Thu, 16 Jul 2026 12:00:00 GMT".to_owned())),
        response_hash: Set(Some(HASH_B.to_owned())),
        entry_sequence_head: Set(1),
        last_attempt_at: Set(Some(NOW)),
        last_success_at: Set(Some(NOW)),
        last_changed_at: Set(Some(NOW)),
        next_fetch_at: Set(NOW + time::Duration::minutes(5)),
        retry_after: Set(None),
        failure_count: Set(0),
        last_error: Set(None),
        is_disabled: Set(false),
        orphaned_at: Set(None),
        lease_owner: Set(None),
        lease_token: Set(0),
        lease_until: Set(None),
        created_at: Set(NOW),
        updated_at: Set(NOW),
    }
    .insert(database)
    .await
    .expect("feed should insert");
}

async fn insert_subscription(database: &sea_orm::DatabaseConnection, id: &str, user_id: &str) {
    subscription::ActiveModel {
        id: Set(id.to_owned()),
        user_id: Set(user_id.to_owned()),
        feed_id: Set(FEED_ID.to_owned()),
        title_override: Set(None),
        position: Set(0),
        start_sequence: Set(1),
        read_through_sequence: Set(0),
        state_revision: Set(0),
        created_at: Set(NOW),
        updated_at: Set(NOW),
    }
    .insert(database)
    .await
    .expect("subscription should insert");
}

async fn insert_entry(database: &sea_orm::DatabaseConnection) {
    entry::ActiveModel {
        id: Set(ENTRY_ID.to_owned()),
        feed_id: Set(FEED_ID.to_owned()),
        feed_sequence: Set(1),
        ingest_generation: Set(1),
        identity_kind: Set("GUID".to_owned()),
        identity_full: Set("urn:example:entry:1".to_owned()),
        identity_hash: Set(HASH_B.to_owned()),
        canonical_url: Set(Some("https://example.com/articles/1".to_owned())),
        title: Set(Some("First entry".to_owned())),
        author: Set(Some("Example Author".to_owned())),
        sanitized_content: Set(Some("<p>Safe content</p>".to_owned())),
        summary: Set(Some("Safe summary".to_owned())),
        published_at_us: Set(Some(1_768_472_800_000_000)),
        sort_at_us: Set(1_768_476_400_000_000),
        inserted_at: Set(NOW),
        updated_at: Set(NOW),
        source_content_hash: Set(HASH_C.to_owned()),
        content_hash: Set(HASH_C.to_owned()),
        pipeline_version: Set(1),
        direction: Set("LTR".to_owned()),
        enclosure_json: Set("{\"version\":1,\"items\":[]}".to_owned()),
    }
    .insert(database)
    .await
    .expect("entry should insert");
}

async fn insert_entry_state(
    database: &sea_orm::DatabaseConnection,
    user_id: &str,
    feed_sequence: i64,
) -> Result<entry_state::Model, sea_orm::DbErr> {
    entry_state::ActiveModel {
        user_id: Set(user_id.to_owned()),
        entry_id: Set(ENTRY_ID.to_owned()),
        feed_id: Set(FEED_ID.to_owned()),
        feed_sequence: Set(feed_sequence),
        read_override: Set(Some(true)),
        is_starred: Set(true),
        starred_at: Set(Some(NOW)),
        revision: Set(1),
        updated_at: Set(NOW),
    }
    .insert(database)
    .await
}
