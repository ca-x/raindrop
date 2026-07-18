#[allow(dead_code)]
mod support;

use std::{sync::Arc, time::Duration};

use raindrop::{
    db::{
        entities::{entry, feed, feed_refresh_run, lifecycle_outbox, subscription, user},
        migrate, rollback,
    },
    feeds::{FeedRepository, FeedUrlPolicy},
};
use sea_orm::{ActiveModelTrait, ActiveValue::Set, EntityTrait, PaginatorTrait};
use secrecy::SecretString;
use time::OffsetDateTime;
use uuid::Uuid;

use support::database::connect_for_contract;

#[tokio::test]
async fn sqlite_old_orphan_feed_is_deleted_while_outbox_survives() {
    let data = tempfile::tempdir().expect("temporary directory should be created");
    let url = format!(
        "sqlite://{}?mode=rwc",
        data.path().join("feed-retention.db").display()
    );
    let database = connect_for_contract(SecretString::from(url)).await;
    migrate(&database)
        .await
        .expect("retention migrations should apply");
    let feed_id = Uuid::new_v4().to_string();
    let entry_id = Uuid::new_v4().to_string();
    let refresh_id = Uuid::new_v4().to_string();
    let event_id = Uuid::new_v4().to_string();
    let old = OffsetDateTime::now_utc() - time::Duration::days(10);

    seed_orphan_feed(&database, &feed_id, old).await;
    seed_entry(&database, &entry_id, &feed_id, old).await;
    seed_terminal_refresh(&database, &refresh_id, &feed_id, old).await;
    seed_outbox(&database, &event_id, &refresh_id, &feed_id, old).await;

    let repository = FeedRepository::new(database.clone());
    assert_eq!(
        repository
            .purge_orphaned_feeds(Duration::from_secs(86_400), 100)
            .await
            .expect("eligible orphan should purge"),
        1
    );

    assert_eq!(
        feed::Entity::find()
            .count(&database)
            .await
            .expect("feeds should count"),
        0
    );
    assert_eq!(
        entry::Entity::find()
            .count(&database)
            .await
            .expect("entries should count"),
        0
    );
    assert_eq!(
        feed_refresh_run::Entity::find()
            .count(&database)
            .await
            .expect("refresh runs should count"),
        0
    );
    assert_eq!(
        lifecycle_outbox::Entity::find()
            .count(&database)
            .await
            .expect("outbox should count"),
        1
    );
    assert_eq!(
        repository
            .purge_orphaned_feeds(Duration::from_secs(86_400), 100)
            .await
            .expect("repeated retention should be idempotent"),
        0
    );
}

#[tokio::test]
async fn sqlite_retention_preserves_recent_subscribed_and_active_feeds() {
    let data = tempfile::tempdir().expect("temporary directory should be created");
    let url = format!(
        "sqlite://{}?mode=rwc",
        data.path().join("feed-retention-preserve.db").display()
    );
    let database = connect_for_contract(SecretString::from(url)).await;
    migrate(&database)
        .await
        .expect("retention migrations should apply");
    let old = OffsetDateTime::now_utc() - time::Duration::days(10);
    let recent = OffsetDateTime::now_utc() - time::Duration::hours(12);
    let eligible_feed = Uuid::new_v4().to_string();
    let recent_feed = Uuid::new_v4().to_string();
    let subscribed_feed = Uuid::new_v4().to_string();
    let active_feed = Uuid::new_v4().to_string();

    seed_orphan_feed(&database, &eligible_feed, old).await;
    seed_orphan_feed(&database, &recent_feed, recent).await;
    seed_orphan_feed(&database, &subscribed_feed, old).await;
    seed_orphan_feed(&database, &active_feed, old).await;
    seed_subscription(&database, &subscribed_feed, old).await;
    seed_active_refresh(&database, &active_feed, old).await;

    let repository = FeedRepository::new(database.clone());
    assert_eq!(
        repository
            .purge_orphaned_feeds(Duration::from_secs(86_400), 100)
            .await
            .expect("retention preservation pass should succeed"),
        1
    );

    for feed_id in [&recent_feed, &subscribed_feed, &active_feed] {
        assert!(
            feed::Entity::find_by_id(feed_id)
                .one(&database)
                .await
                .expect("preserved feed should query")
                .is_some(),
            "feed {feed_id} should be preserved"
        );
    }
    assert!(
        feed::Entity::find_by_id(eligible_feed)
            .one(&database)
            .await
            .expect("eligible feed should query")
            .is_none()
    );
}

#[tokio::test]
async fn sqlite_retention_honors_candidate_batch_limit() {
    let data = tempfile::tempdir().expect("temporary directory should be created");
    let url = format!(
        "sqlite://{}?mode=rwc",
        data.path().join("feed-retention-limit.db").display()
    );
    let database = connect_for_contract(SecretString::from(url)).await;
    migrate(&database)
        .await
        .expect("retention migrations should apply");
    let old = OffsetDateTime::now_utc() - time::Duration::days(10);
    for _ in 0..3 {
        seed_orphan_feed(&database, &Uuid::new_v4().to_string(), old).await;
    }
    let repository = FeedRepository::new(database.clone());

    assert_eq!(
        repository
            .purge_orphaned_feeds(Duration::from_secs(86_400), 2)
            .await
            .expect("first bounded pass should succeed"),
        2
    );
    assert_eq!(
        feed::Entity::find()
            .count(&database)
            .await
            .expect("remaining feeds should count"),
        1
    );
    assert_eq!(
        repository
            .purge_orphaned_feeds(Duration::from_secs(86_400), 2)
            .await
            .expect("second bounded pass should succeed"),
        1
    );
}

#[tokio::test]
async fn sqlite_subscribe_retries_when_retention_deletes_discovered_feed() {
    let data = tempfile::tempdir().expect("temporary directory should be created");
    let url = format!(
        "sqlite://{}?mode=rwc",
        data.path().join("feed-retention-resubscribe.db").display()
    );
    let database = connect_for_contract(SecretString::from(url.clone())).await;
    migrate(&database)
        .await
        .expect("retention migrations should apply");
    let second_database = connect_for_contract(SecretString::from(url)).await;
    let old = OffsetDateTime::now_utc() - time::Duration::days(10);
    let feed_id = Uuid::new_v4().to_string();
    let user_id = seed_user(&database, old).await;
    let source_url = retention_feed_url(&feed_id);
    seed_orphan_feed(&database, &feed_id, old).await;
    let normalized = FeedUrlPolicy::new(false)
        .normalize(&source_url)
        .expect("retention feed URL should normalize");
    let subscribe_repository = FeedRepository::new(database.clone());
    let retention_repository = FeedRepository::new(second_database);
    let scanned = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Notify::new());
    let subscribe_scanned = Arc::clone(&scanned);
    let subscribe_release = Arc::clone(&release);
    let subscribe_user = user_id.clone();
    let subscribe_url = source_url.clone();
    let subscribe = tokio::spawn(async move {
        subscribe_repository
            .subscribe_after_feed_scan(
                &subscribe_user,
                &subscribe_url,
                &normalized,
                subscribe_scanned,
                subscribe_release,
            )
            .await
    });

    scanned.notified().await;
    assert_eq!(
        retention_repository
            .purge_orphaned_feeds(Duration::from_secs(86_400), 100)
            .await
            .expect("retention should delete the discovered orphan"),
        1
    );
    release.notify_one();
    let outcome = subscribe
        .await
        .expect("subscription task should join")
        .expect("subscription should retry after retention");

    assert!(outcome.created);
    assert_eq!(
        feed::Entity::find()
            .count(&database)
            .await
            .expect("replacement feeds should count"),
        1
    );
    assert_eq!(
        subscription::Entity::find()
            .count(&database)
            .await
            .expect("replacement subscriptions should count"),
        1
    );
    assert!(
        feed::Entity::find_by_id(feed_id)
            .one(&database)
            .await
            .expect("deleted feed should query")
            .is_none()
    );
}

#[tokio::test]
async fn sqlite_two_retention_workers_converge_without_double_counting() {
    let data = tempfile::tempdir().expect("temporary directory should be created");
    let url = format!(
        "sqlite://{}?mode=rwc",
        data.path()
            .join("feed-retention-multi-instance.db")
            .display()
    );
    let database = connect_for_contract(SecretString::from(url.clone())).await;
    migrate(&database)
        .await
        .expect("retention migrations should apply");
    let second_database = connect_for_contract(SecretString::from(url)).await;
    let old = OffsetDateTime::now_utc() - time::Duration::days(10);
    for _ in 0..4 {
        seed_orphan_feed(&database, &Uuid::new_v4().to_string(), old).await;
    }
    let first = FeedRepository::new(database.clone());
    let second = FeedRepository::new(second_database);
    let barrier = Arc::new(tokio::sync::Barrier::new(2));
    let first_barrier = Arc::clone(&barrier);
    let second_barrier = Arc::clone(&barrier);

    let (first_deleted, second_deleted) = tokio::time::timeout(Duration::from_secs(10), async {
        tokio::join!(
            async move {
                first_barrier.wait().await;
                first
                    .purge_orphaned_feeds(Duration::from_secs(86_400), 100)
                    .await
            },
            async move {
                second_barrier.wait().await;
                second
                    .purge_orphaned_feeds(Duration::from_secs(86_400), 100)
                    .await
            }
        )
    })
    .await
    .expect("concurrent retention should not deadlock");

    assert_eq!(
        first_deleted.expect("first retention worker should succeed")
            + second_deleted.expect("second retention worker should succeed"),
        4
    );
    assert_eq!(
        feed::Entity::find()
            .count(&database)
            .await
            .expect("remaining feeds should count"),
        0
    );
}

#[tokio::test]
async fn postgres_feed_retention_contract() {
    let Ok(url) = std::env::var("RAINDROP_TEST_POSTGRES_URL") else {
        eprintln!("postgres feed retention contract skipped: test database URL is not configured");
        return;
    };
    backend_feed_retention_contract(&url, "postgres").await;
}

#[tokio::test]
async fn mysql_feed_retention_contract() {
    let Ok(url) = std::env::var("RAINDROP_TEST_MYSQL_URL") else {
        eprintln!("mysql feed retention contract skipped: test database URL is not configured");
        return;
    };
    backend_feed_retention_contract(&url, "mysql").await;
}

async fn backend_feed_retention_contract(url: &str, backend_name: &str) {
    let database = connect_for_contract(SecretString::from(url.to_owned())).await;
    rollback(&database)
        .await
        .unwrap_or_else(|_| panic!("{backend_name} retention database should reset"));
    migrate(&database)
        .await
        .unwrap_or_else(|_| panic!("{backend_name} retention migrations should apply"));
    let old = OffsetDateTime::now_utc() - time::Duration::days(10);
    let recent = OffsetDateTime::now_utc() - time::Duration::hours(12);
    let eligible_feed = Uuid::new_v4().to_string();
    let recent_feed = Uuid::new_v4().to_string();
    let subscribed_feed = Uuid::new_v4().to_string();
    let active_feed = Uuid::new_v4().to_string();
    let entry_id = Uuid::new_v4().to_string();
    let refresh_id = Uuid::new_v4().to_string();
    let event_id = Uuid::new_v4().to_string();

    seed_orphan_feed(&database, &eligible_feed, old).await;
    seed_entry(&database, &entry_id, &eligible_feed, old).await;
    seed_terminal_refresh(&database, &refresh_id, &eligible_feed, old).await;
    seed_outbox(&database, &event_id, &refresh_id, &eligible_feed, old).await;
    seed_orphan_feed(&database, &recent_feed, recent).await;
    seed_orphan_feed(&database, &subscribed_feed, old).await;
    seed_subscription(&database, &subscribed_feed, old).await;
    seed_orphan_feed(&database, &active_feed, old).await;
    seed_active_refresh(&database, &active_feed, old).await;

    let repository = FeedRepository::new(database.clone());
    assert_eq!(
        repository
            .purge_orphaned_feeds(Duration::from_secs(86_400), 100)
            .await
            .unwrap_or_else(|_| panic!("{backend_name} retention pass should succeed")),
        1
    );
    assert!(
        feed::Entity::find_by_id(eligible_feed)
            .one(&database)
            .await
            .unwrap_or_else(|_| panic!("{backend_name} eligible feed should query"))
            .is_none()
    );
    for feed_id in [recent_feed, subscribed_feed, active_feed] {
        assert!(
            feed::Entity::find_by_id(feed_id)
                .one(&database)
                .await
                .unwrap_or_else(|_| panic!("{backend_name} protected feed should query"))
                .is_some()
        );
    }
    assert_eq!(
        lifecycle_outbox::Entity::find()
            .count(&database)
            .await
            .unwrap_or_else(|_| panic!("{backend_name} outbox should count")),
        1
    );
    database
        .close()
        .await
        .unwrap_or_else(|_| panic!("{backend_name} retention database should close"));
}

async fn seed_orphan_feed(
    database: &sea_orm::DatabaseConnection,
    feed_id: &str,
    at: OffsetDateTime,
) {
    let normalized_url = retention_feed_url(feed_id);
    feed::ActiveModel {
        id: Set(feed_id.to_owned()),
        source_url: Set(normalized_url.clone()),
        normalized_url: Set(normalized_url.clone()),
        normalized_url_hash: Set(blake3::hash(normalized_url.as_bytes()).to_hex().to_string()),
        fetch_url: Set(normalized_url),
        title: Set(Some("Retention feed".to_owned())),
        site_url: Set(None),
        validator_url: Set(None),
        etag: Set(None),
        last_modified: Set(None),
        response_content_hash: Set(None),
        entry_sequence_head: Set(1),
        last_attempt_at: Set(Some(at)),
        last_success_at: Set(Some(at)),
        last_changed_at: Set(Some(at)),
        next_fetch_at: Set(at),
        retry_after_at: Set(None),
        consecutive_failures: Set(0),
        last_error_code: Set(None),
        is_disabled: Set(false),
        orphaned_at: Set(Some(at)),
        lease_owner: Set(None),
        lease_token: Set(0),
        lease_until: Set(None),
        created_at: Set(at),
        updated_at: Set(at),
    }
    .insert(database)
    .await
    .expect("orphan feed should insert");
}

fn retention_feed_url(feed_id: &str) -> String {
    format!("https://{feed_id}.example.test/feed.xml")
}

async fn seed_entry(
    database: &sea_orm::DatabaseConnection,
    entry_id: &str,
    feed_id: &str,
    at: OffsetDateTime,
) {
    entry::ActiveModel {
        id: Set(entry_id.to_owned()),
        feed_id: Set(feed_id.to_owned()),
        feed_sequence: Set(1),
        ingest_generation: Set(1),
        identity_kind: Set("GUID".to_owned()),
        identity: Set(format!("urn:retention:{entry_id}")),
        identity_hash: Set(blake3::hash(entry_id.as_bytes()).to_hex().to_string()),
        canonical_url: Set(None),
        title: Set(Some("Retention entry".to_owned())),
        author: Set(None),
        sanitized_content: Set("rdsc:v1:{\"html\":\"<p>Safe</p>\",\"inertImages\":[]}".to_owned()),
        search_text: Set("retention entry safe".to_owned()),
        summary: Set(None),
        published_at_us: Set(None),
        sort_at_us: Set(1),
        inserted_at: Set(at),
        updated_at: Set(at),
        source_content_hash: Set("a".repeat(64)),
        content_hash: Set("b".repeat(64)),
        pipeline_version: Set("sanitize-v1".to_owned()),
        direction: Set(None),
        enclosure_json: Set(None),
    }
    .insert(database)
    .await
    .expect("retention entry should insert");
}

async fn seed_terminal_refresh(
    database: &sea_orm::DatabaseConnection,
    refresh_id: &str,
    feed_id: &str,
    at: OffsetDateTime,
) {
    feed_refresh_run::ActiveModel {
        id: Set(refresh_id.to_owned()),
        feed_id: Set(feed_id.to_owned()),
        requested_by_user_id: Set(None),
        trigger_kind: Set("SCHEDULED".to_owned()),
        status: Set("SUCCESS".to_owned()),
        idempotency_key: Set(format!("retention:{refresh_id}")),
        lease_token: Set(Some(1)),
        commit_generation: Set(Some(1)),
        queued_at: Set(at),
        started_at: Set(Some(at)),
        fetched_at: Set(Some(at)),
        persisted_at: Set(Some(at)),
        completed_at: Set(Some(at)),
        http_status: Set(Some(200)),
        new_count: Set(1),
        updated_count: Set(0),
        dropped_count: Set(0),
        error_code: Set(None),
        retry_at: Set(None),
    }
    .insert(database)
    .await
    .expect("terminal refresh should insert");
}

async fn seed_active_refresh(
    database: &sea_orm::DatabaseConnection,
    feed_id: &str,
    at: OffsetDateTime,
) {
    let refresh_id = Uuid::new_v4().to_string();
    feed_refresh_run::ActiveModel {
        id: Set(refresh_id.clone()),
        feed_id: Set(feed_id.to_owned()),
        requested_by_user_id: Set(None),
        trigger_kind: Set("SCHEDULED".to_owned()),
        status: Set("QUEUED".to_owned()),
        idempotency_key: Set(format!("retention-active:{refresh_id}")),
        lease_token: Set(None),
        commit_generation: Set(None),
        queued_at: Set(at),
        started_at: Set(None),
        fetched_at: Set(None),
        persisted_at: Set(None),
        completed_at: Set(None),
        http_status: Set(None),
        new_count: Set(0),
        updated_count: Set(0),
        dropped_count: Set(0),
        error_code: Set(None),
        retry_at: Set(None),
    }
    .insert(database)
    .await
    .expect("active refresh should insert");
}

async fn seed_subscription(
    database: &sea_orm::DatabaseConnection,
    feed_id: &str,
    at: OffsetDateTime,
) {
    let user_id = seed_user(database, at).await;
    subscription::ActiveModel {
        id: Set(Uuid::new_v4().to_string()),
        user_id: Set(user_id),
        feed_id: Set(feed_id.to_owned()),
        category_id: Set(None),
        title_override: Set(None),
        position: Set(0),
        start_sequence: Set(0),
        read_through_sequence: Set(0),
        state_revision: Set(0),
        created_at: Set(at),
        updated_at: Set(at),
    }
    .insert(database)
    .await
    .expect("retention subscription should insert");
}

async fn seed_user(database: &sea_orm::DatabaseConnection, at: OffsetDateTime) -> String {
    let user_id = Uuid::new_v4().to_string();
    user::ActiveModel {
        id: Set(user_id.clone()),
        username: Set(format!("retention-{}", &user_id[..8])),
        normalized_username: Set(format!("retention-{}", &user_id[..8])),
        email: Set(None),
        password_hash: Set("test-hash".to_owned()),
        is_disabled: Set(false),
        created_at: Set(at),
        last_login_at: Set(None),
    }
    .insert(database)
    .await
    .expect("retention user should insert");
    user_id
}

async fn seed_outbox(
    database: &sea_orm::DatabaseConnection,
    event_id: &str,
    refresh_id: &str,
    feed_id: &str,
    at: OffsetDateTime,
) {
    lifecycle_outbox::ActiveModel {
        id: Set(event_id.to_owned()),
        event_type: Set("feed.refresh.completed".to_owned()),
        aggregate_type: Set("FEED".to_owned()),
        aggregate_id: Set(feed_id.to_owned()),
        refresh_id: Set(refresh_id.to_owned()),
        event_sequence: Set(1),
        payload_version: Set(1),
        payload_json: Set("{\"schemaVersion\":1}".to_owned()),
        idempotency_key: Set(format!("retention:{refresh_id}:completed:v1")),
        status: Set("PENDING".to_owned()),
        available_at: Set(at),
        attempts: Set(0),
        lease_owner: Set(None),
        lease_until: Set(None),
        created_at: Set(at),
        completed_at: Set(None),
    }
    .insert(database)
    .await
    .expect("retention outbox should insert");
}
