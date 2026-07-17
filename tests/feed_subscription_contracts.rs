#[allow(dead_code)]
mod support;

use base64::Engine;
use raindrop::db::{
    entities::{entry, entry_state, feed, feed_refresh_run, subscription},
    migrate, rollback,
};
use raindrop::feeds::{FeedRepository, ListSubscriptionsQuery, RepositoryError};
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ConnectionTrait, DatabaseConnection, EntityTrait,
};
use secrecy::SecretString;
use time::{OffsetDateTime, macros::datetime};
use uuid::Uuid;

use support::database::{
    SUBSCRIPTION_A_ID, SUBSCRIPTION_B_ID, USER_A_ID, USER_B_ID, connect_for_contract, insert_user,
};

const SECOND_FEED_ID: &str = "00000000-0000-4000-8000-000000000102";
const NOISE_FEED_ID: &str = "00000000-0000-4000-8000-000000000103";
const SECOND_SUBSCRIPTION_A_ID: &str = "00000000-0000-4000-8000-000000000203";
const ENTRY_1_ID: &str = "00000000-0000-4000-8000-000000000301";
const ENTRY_2_ID: &str = "00000000-0000-4000-8000-000000000302";
const ENTRY_3_ID: &str = "00000000-0000-4000-8000-000000000303";
const REFRESH_RUN_1_ID: &str = "00000000-0000-4000-8000-000000000401";
const REFRESH_RUN_2_ID: &str = "00000000-0000-4000-8000-000000000402";
const REFRESH_RUN_3_ID: &str = "00000000-0000-4000-8000-000000000400";
const SHARED_FEED_URL: &str = "https://shared.example.test/feed.xml";
const SECOND_FEED_URL: &str = "https://second.example.test/feed.xml";
const FIXTURE_AT: OffsetDateTime = datetime!(2026-07-16 12:00:00 UTC);

struct SubscriptionFixture {
    #[allow(dead_code)]
    database: DatabaseConnection,
    repository: FeedRepository,
}

#[tokio::test]
async fn postgres_subscription_projection_explain_and_runtime_contract() {
    let Ok(url) = std::env::var("RAINDROP_TEST_POSTGRES_URL") else {
        eprintln!(
            "skipping PostgreSQL subscription projection contract; RAINDROP_TEST_POSTGRES_URL is unset"
        );
        return;
    };
    backend_projection_contract(SecretString::from(url), "postgres").await;
}

#[tokio::test]
async fn mysql_subscription_projection_explain_and_runtime_contract() {
    let Ok(url) = std::env::var("RAINDROP_TEST_MYSQL_URL") else {
        eprintln!(
            "skipping MySQL subscription projection contract; RAINDROP_TEST_MYSQL_URL is unset"
        );
        return;
    };
    backend_projection_contract(SecretString::from(url), "mysql").await;
}

async fn backend_projection_contract(url: SecretString, backend_name: &str) {
    let database = connect_for_contract(url).await;
    let _ = rollback(&database).await;
    migrate(&database)
        .await
        .unwrap_or_else(|_| panic!("{backend_name} migrations should apply"));
    seed_fixture(&database).await;
    seed_explain_noise(&database).await;
    let statistics = match backend_name {
        "postgres" => "ANALYZE",
        "mysql" => "ANALYZE TABLE subscriptions, feeds, entries, entry_states, feed_refresh_runs",
        _ => unreachable!("backend contract only covers PostgreSQL and MySQL"),
    };
    database
        .execute_unprepared(statistics)
        .await
        .unwrap_or_else(|_| panic!("{backend_name} statistics should collect"));
    let repository = FeedRepository::new(database.clone());

    let page = repository
        .list_subscriptions_for_user(
            USER_A_ID,
            ListSubscriptionsQuery {
                cursor: None,
                limit: 1,
            },
        )
        .await
        .unwrap_or_else(|_| panic!("{backend_name} subscription list should execute"));
    assert_eq!(page.items.len(), 1);
    assert_eq!(page.items[0].subscription_id, SUBSCRIPTION_A_ID);
    assert_eq!(page.items[0].unread_count, 2);
    assert!(
        repository
            .get_subscription_for_user(USER_B_ID, SUBSCRIPTION_A_ID)
            .await
            .unwrap_or_else(|_| panic!("{backend_name} cross-tenant detail should execute"))
            .is_none()
    );

    let plan = repository
        .explain_list_subscriptions_for_user(USER_A_ID, ListSubscriptionsQuery::default())
        .await
        .unwrap_or_else(|_| panic!("{backend_name} subscription EXPLAIN should execute"));
    let joined = plan.join("\n");
    assert!(
        joined.contains("uq_subscriptions_user_feed")
            || joined.contains("idx_subscriptions_user_pos"),
        "{backend_name} must use a user-leading subscription index: {joined}"
    );
    assert!(
        joined.contains("idx_entries_feed_list")
            || joined.contains("uq_entries_feed_seq")
            || joined.contains("idx_entries_snapshot"),
        "{backend_name} must use bounded feed-leading entry access: {joined}"
    );
    assert!(
        joined.contains("feeds_pkey")
            || joined.contains("table=f key=PRIMARY")
            || joined.contains("table=feeds key=PRIMARY"),
        "{backend_name} must use bounded feed access: {joined}"
    );
    assert!(
        joined.contains("idx_refresh_runs_feed") || joined.contains("uq_refresh_runs_idem"),
        "{backend_name} must use bounded latest-run access: {joined}"
    );
    database.close().await.expect("database should close");
}

impl SubscriptionFixture {
    async fn new() -> Self {
        let data = tempfile::tempdir().expect("temporary directory should be created");
        let database_url = format!(
            "sqlite://{}?mode=rwc",
            data.path().join("feed-subscription-contracts.db").display()
        );
        let database = connect_for_contract(SecretString::from(database_url)).await;
        migrate(&database)
            .await
            .expect("RSS migrations should apply");
        seed_fixture(&database).await;
        Self {
            repository: FeedRepository::new(database.clone()),
            database,
        }
    }
}

#[tokio::test]
async fn sqlite_subscription_list_is_user_scoped_and_matches_reader_unread_state() {
    let fixture = SubscriptionFixture::new().await;
    let page = fixture
        .repository
        .list_subscriptions_for_user(
            USER_A_ID,
            ListSubscriptionsQuery {
                cursor: None,
                limit: 1,
            },
        )
        .await
        .unwrap();
    assert_eq!(page.items.len(), 1);
    assert_eq!(page.items[0].subscription_id, SUBSCRIPTION_A_ID);
    assert_eq!(page.items[0].title, "Personal title");
    assert_eq!(page.items[0].unread_count, 2);
    assert!(page.next_cursor.is_some());
}

#[tokio::test]
async fn sqlite_subscription_detail_hides_missing_and_cross_tenant() {
    let fixture = SubscriptionFixture::new().await;
    let detail = fixture
        .repository
        .get_subscription_for_user(USER_A_ID, SUBSCRIPTION_A_ID)
        .await
        .expect("owned subscription detail should query")
        .expect("owned subscription should be visible");
    assert_eq!(detail.subscription_id, SUBSCRIPTION_A_ID);
    assert_eq!(detail.unread_count, 2);

    assert!(
        fixture
            .repository
            .get_subscription_for_user(USER_B_ID, SUBSCRIPTION_A_ID)
            .await
            .expect("cross-tenant lookup should remain opaque")
            .is_none()
    );
    assert!(
        fixture
            .repository
            .get_subscription_for_user(USER_A_ID, "00000000-0000-4000-8000-000000000299",)
            .await
            .expect("missing lookup should remain typed")
            .is_none()
    );
}

#[tokio::test]
async fn sqlite_subscription_list_rejects_invalid_user_limit_and_cursor() {
    let fixture = SubscriptionFixture::new().await;
    assert!(matches!(
        fixture
            .repository
            .list_subscriptions_for_user("not-a-user-id", ListSubscriptionsQuery::default())
            .await,
        Err(RepositoryError::InvalidUserId)
    ));
    for limit in [0, 101] {
        assert!(matches!(
            fixture
                .repository
                .list_subscriptions_for_user(
                    USER_A_ID,
                    ListSubscriptionsQuery {
                        cursor: None,
                        limit,
                    },
                )
                .await,
            Err(RepositoryError::InvalidLimit)
        ));
    }
    assert!(matches!(
        fixture
            .repository
            .list_subscriptions_for_user(
                USER_A_ID,
                ListSubscriptionsQuery {
                    cursor: Some("not+a-cursor".to_owned()),
                    limit: 50,
                },
            )
            .await,
        Err(RepositoryError::InvalidCursor)
    ));
}

#[tokio::test]
async fn sqlite_subscription_cursor_rejects_cross_user_and_noncanonical_reuse() {
    let fixture = SubscriptionFixture::new().await;
    let second = subscription::Entity::find_by_id(SECOND_SUBSCRIPTION_A_ID)
        .one(&fixture.database)
        .await
        .expect("second subscription should query")
        .expect("second subscription should exist");
    let mut second: subscription::ActiveModel = second.into();
    second.created_at = Set(FIXTURE_AT + time::Duration::minutes(3));
    second
        .update(&fixture.database)
        .await
        .expect("tie timestamp should update");

    let first = fixture
        .repository
        .list_subscriptions_for_user(
            USER_A_ID,
            ListSubscriptionsQuery {
                cursor: None,
                limit: 1,
            },
        )
        .await
        .expect("first tied page should query");
    assert_eq!(first.items[0].subscription_id, SECOND_SUBSCRIPTION_A_ID);
    let cursor = first.next_cursor.expect("tied subscriptions need page two");
    let second = fixture
        .repository
        .list_subscriptions_for_user(
            USER_A_ID,
            ListSubscriptionsQuery {
                cursor: Some(cursor.clone()),
                limit: 1,
            },
        )
        .await
        .expect("second tied page should query");
    assert_eq!(second.items[0].subscription_id, SUBSCRIPTION_A_ID);
    assert!(second.next_cursor.is_none());

    assert!(matches!(
        fixture
            .repository
            .list_subscriptions_for_user(
                USER_B_ID,
                ListSubscriptionsQuery {
                    cursor: Some(cursor.clone()),
                    limit: 1,
                },
            )
            .await,
        Err(RepositoryError::InvalidCursor)
    ));
    assert!(matches!(
        fixture
            .repository
            .list_subscriptions_for_user(
                USER_A_ID,
                ListSubscriptionsQuery {
                    cursor: Some(reorder_cursor_json(&cursor)),
                    limit: 1,
                },
            )
            .await,
        Err(RepositoryError::InvalidCursor)
    ));
}

#[tokio::test]
async fn sqlite_subscription_title_falls_back_to_feed_then_host() {
    let fixture = SubscriptionFixture::new().await;
    let subscription = subscription::Entity::find_by_id(SUBSCRIPTION_A_ID)
        .one(&fixture.database)
        .await
        .expect("subscription should query")
        .expect("subscription should exist");
    let mut subscription: subscription::ActiveModel = subscription.into();
    subscription.title_override = Set(Some("   ".to_owned()));
    subscription
        .update(&fixture.database)
        .await
        .expect("title override should update");
    let detail = fixture
        .repository
        .get_subscription_for_user(USER_A_ID, SUBSCRIPTION_A_ID)
        .await
        .expect("feed-title fallback should query")
        .expect("subscription should remain visible");
    assert_eq!(detail.title, "Shared feed");

    let feed = feed::Entity::find_by_id(support::database::FEED_ID)
        .one(&fixture.database)
        .await
        .expect("feed should query")
        .expect("feed should exist");
    let mut feed: feed::ActiveModel = feed.into();
    feed.title = Set(Some("\t".to_owned()));
    feed.update(&fixture.database)
        .await
        .expect("feed title should update");
    let detail = fixture
        .repository
        .get_subscription_for_user(USER_A_ID, SUBSCRIPTION_A_ID)
        .await
        .expect("host fallback should query")
        .expect("subscription should remain visible");
    assert_eq!(detail.title, "shared.example.test");
    let debug = format!("{detail:?}");
    assert!(!debug.contains("shared.example.test"));
    assert!(!debug.contains("https://shared.example.test"));
    assert!(debug.contains("[REDACTED]"));
}

#[tokio::test]
async fn sqlite_subscription_latest_refresh_uses_queued_at_then_run_id() {
    let fixture = SubscriptionFixture::new().await;
    let detail = fixture
        .repository
        .get_subscription_for_user(USER_A_ID, SUBSCRIPTION_A_ID)
        .await
        .expect("latest refresh should query")
        .expect("subscription should be visible");
    let refresh = detail
        .refresh
        .expect("shared feed should have refresh runs");
    assert_eq!(refresh.run_id, REFRESH_RUN_2_ID);
    assert_eq!(refresh.status.as_str(), "ERROR");
    assert_eq!(refresh.generation, Some(2));

    let mut later = refresh_run_model(REFRESH_RUN_3_ID, "PARTIAL", 3);
    later.queued_at = Set(FIXTURE_AT + time::Duration::minutes(6));
    later
        .insert(&fixture.database)
        .await
        .expect("later refresh run should insert");
    let detail = fixture
        .repository
        .get_subscription_for_user(USER_A_ID, SUBSCRIPTION_A_ID)
        .await
        .expect("later refresh should query")
        .expect("subscription should be visible");
    let refresh = detail.refresh.expect("later refresh should project");
    assert_eq!(refresh.run_id, REFRESH_RUN_3_ID);
    assert_eq!(refresh.status.as_str(), "PARTIAL");
    assert_eq!(refresh.generation, Some(3));

    let without_refresh = fixture
        .repository
        .get_subscription_for_user(USER_A_ID, SECOND_SUBSCRIPTION_A_ID)
        .await
        .expect("subscription without a run should query")
        .expect("second subscription should be visible");
    assert!(without_refresh.refresh.is_none());
}

fn reorder_cursor_json(cursor: &str) -> String {
    let json = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(cursor)
        .expect("generated cursor should decode");
    let value: serde_json::Value =
        serde_json::from_slice(&json).expect("generated cursor should contain JSON");
    let reordered = serde_json::to_vec(&value).expect("cursor JSON should reserialize");
    assert_ne!(
        reordered, json,
        "test must produce noncanonical field order"
    );
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(reordered)
}

async fn seed_fixture(database: &DatabaseConnection) {
    insert_user(database, USER_A_ID, "subscription-reader-a").await;
    insert_user(database, USER_B_ID, "subscription-reader-b").await;
    feed_model(
        support::database::FEED_ID,
        SHARED_FEED_URL,
        Some("Shared feed"),
        Some("https://shared.example.test"),
    )
    .insert(database)
    .await
    .expect("shared feed should insert");
    feed_model(SECOND_FEED_ID, SECOND_FEED_URL, Some("Second feed"), None)
        .insert(database)
        .await
        .expect("second feed should insert");

    subscription_model(
        SUBSCRIPTION_A_ID,
        USER_A_ID,
        support::database::FEED_ID,
        Some("Personal title"),
        FIXTURE_AT + time::Duration::minutes(3),
        1,
    )
    .insert(database)
    .await
    .expect("reader A shared subscription should insert");
    subscription_model(
        SECOND_SUBSCRIPTION_A_ID,
        USER_A_ID,
        SECOND_FEED_ID,
        None,
        FIXTURE_AT + time::Duration::minutes(2),
        0,
    )
    .insert(database)
    .await
    .expect("reader A second subscription should insert");
    subscription_model(
        SUBSCRIPTION_B_ID,
        USER_B_ID,
        support::database::FEED_ID,
        Some("Other user's title"),
        FIXTURE_AT + time::Duration::minutes(4),
        3,
    )
    .insert(database)
    .await
    .expect("reader B shared subscription should insert");

    for (id, sequence) in [(ENTRY_1_ID, 1_i64), (ENTRY_2_ID, 2), (ENTRY_3_ID, 3)] {
        entry_model(id, sequence)
            .insert(database)
            .await
            .expect("shared entry should insert");
    }
    entry_state_model(USER_A_ID, ENTRY_1_ID, 1, Some(false))
        .insert(database)
        .await
        .expect("explicit unread override should insert");
    entry_state_model(USER_A_ID, ENTRY_2_ID, 2, Some(true))
        .insert(database)
        .await
        .expect("explicit read override should insert");

    refresh_run_model(REFRESH_RUN_1_ID, "SUCCESS", 1)
        .insert(database)
        .await
        .expect("first refresh run should insert");
    refresh_run_model(REFRESH_RUN_2_ID, "ERROR", 2)
        .insert(database)
        .await
        .expect("second refresh run should insert");
}

async fn seed_explain_noise(database: &DatabaseConnection) {
    feed_model(
        NOISE_FEED_ID,
        "https://noise.example.test/feed.xml",
        Some("Noise feed"),
        None,
    )
    .insert(database)
    .await
    .expect("noise feed should insert");
    for index in 0..24_u128 {
        let user_id =
            Uuid::from_u128(0x1000_0000_0000_4000_8000_0000_0000_0000 + index).to_string();
        let feed_id = if index == 0 {
            NOISE_FEED_ID.to_owned()
        } else {
            let feed_id =
                Uuid::from_u128(0x5000_0000_0000_4000_8000_0000_0000_0000 + index).to_string();
            feed_model(
                &feed_id,
                &format!("https://noise-{index:03}.example.test/feed.xml"),
                Some("Noise feed"),
                None,
            )
            .insert(database)
            .await
            .expect("noise feed should insert");
            feed_id
        };
        insert_user(
            database,
            &user_id,
            &format!("subscription-noise-{index:03}"),
        )
        .await;
        subscription_model(
            &Uuid::from_u128(0x2000_0000_0000_4000_8000_0000_0000_0000 + index).to_string(),
            &user_id,
            &feed_id,
            None,
            FIXTURE_AT + time::Duration::seconds(index as i64),
            64,
        )
        .insert(database)
        .await
        .expect("noise subscription should insert");
    }
    for sequence in 1..=256_i64 {
        let mut model = entry_model(
            &Uuid::from_u128(0x3000_0000_0000_4000_8000_0000_0000_0000 + sequence as u128)
                .to_string(),
            sequence,
        );
        model.feed_id = Set(NOISE_FEED_ID.to_owned());
        model
            .insert(database)
            .await
            .expect("noise entry should insert");
    }
    for index in 0..64_i64 {
        let mut model = refresh_run_model(
            &Uuid::from_u128(0x4000_0000_0000_4000_8000_0000_0000_0000 + index as u128).to_string(),
            "SUCCESS",
            1_000 + index,
        );
        model.feed_id = Set(NOISE_FEED_ID.to_owned());
        model.queued_at = Set(FIXTURE_AT + time::Duration::seconds(index));
        model
            .insert(database)
            .await
            .expect("noise refresh run should insert");
    }
}

fn feed_model(
    id: &str,
    normalized_url: &str,
    title: Option<&str>,
    site_url: Option<&str>,
) -> feed::ActiveModel {
    feed::ActiveModel {
        id: Set(id.to_owned()),
        source_url: Set(format!("{normalized_url}?source=private")),
        normalized_url: Set(normalized_url.to_owned()),
        normalized_url_hash: Set(blake3::hash(normalized_url.as_bytes()).to_hex().to_string()),
        fetch_url: Set(format!("{normalized_url}?fetch=private")),
        title: Set(title.map(str::to_owned)),
        site_url: Set(site_url.map(str::to_owned)),
        validator_url: Set(Some(format!("{normalized_url}?validator=private"))),
        etag: Set(Some("\"private-etag\"".to_owned())),
        last_modified: Set(Some("Thu, 16 Jul 2026 12:00:00 GMT".to_owned())),
        response_content_hash: Set(Some("a".repeat(64))),
        entry_sequence_head: Set(3),
        last_attempt_at: Set(Some(FIXTURE_AT)),
        last_success_at: Set(Some(FIXTURE_AT)),
        last_changed_at: Set(Some(FIXTURE_AT)),
        next_fetch_at: Set(FIXTURE_AT + time::Duration::minutes(5)),
        retry_after_at: Set(None),
        consecutive_failures: Set(0),
        last_error_code: Set(None),
        is_disabled: Set(false),
        orphaned_at: Set(None),
        lease_owner: Set(None),
        lease_token: Set(0),
        lease_until: Set(None),
        created_at: Set(FIXTURE_AT),
        updated_at: Set(FIXTURE_AT),
    }
}

fn subscription_model(
    id: &str,
    user_id: &str,
    feed_id: &str,
    title_override: Option<&str>,
    created_at: OffsetDateTime,
    read_through_sequence: i64,
) -> subscription::ActiveModel {
    subscription::ActiveModel {
        id: Set(id.to_owned()),
        user_id: Set(user_id.to_owned()),
        feed_id: Set(feed_id.to_owned()),
        title_override: Set(title_override.map(str::to_owned)),
        position: Set(0),
        start_sequence: Set(0),
        read_through_sequence: Set(read_through_sequence),
        state_revision: Set(0),
        created_at: Set(created_at),
        updated_at: Set(created_at),
    }
}

fn entry_model(id: &str, feed_sequence: i64) -> entry::ActiveModel {
    let identity = format!("subscription-entry-{feed_sequence}");
    entry::ActiveModel {
        id: Set(id.to_owned()),
        feed_id: Set(support::database::FEED_ID.to_owned()),
        feed_sequence: Set(feed_sequence),
        ingest_generation: Set(0),
        identity_kind: Set("GUID".to_owned()),
        identity_hash: Set(blake3::hash(identity.as_bytes()).to_hex().to_string()),
        identity: Set(identity),
        canonical_url: Set(Some(format!(
            "https://shared.example.test/articles/{feed_sequence}"
        ))),
        title: Set(Some(format!("Entry {feed_sequence}"))),
        author: Set(None),
        sanitized_content: Set("rdsc:v1:{\"html\":\"<p>Safe</p>\",\"inertImages\":[]}".to_owned()),
        summary: Set(None),
        published_at_us: Set(Some(1_752_667_200_000_000 + feed_sequence)),
        sort_at_us: Set(1_752_667_200_000_000 + feed_sequence),
        inserted_at: Set(FIXTURE_AT),
        updated_at: Set(FIXTURE_AT),
        source_content_hash: Set("b".repeat(64)),
        content_hash: Set("c".repeat(64)),
        pipeline_version: Set("sanitize-v1".to_owned()),
        direction: Set(Some("LTR".to_owned())),
        enclosure_json: Set(None),
    }
}

fn entry_state_model(
    user_id: &str,
    entry_id: &str,
    feed_sequence: i64,
    read_override: Option<bool>,
) -> entry_state::ActiveModel {
    entry_state::ActiveModel {
        user_id: Set(user_id.to_owned()),
        entry_id: Set(entry_id.to_owned()),
        feed_id: Set(support::database::FEED_ID.to_owned()),
        feed_sequence: Set(feed_sequence),
        read_override: Set(read_override),
        is_starred: Set(false),
        starred_at: Set(None),
        revision: Set(1),
        updated_at: Set(FIXTURE_AT),
    }
}

fn refresh_run_model(id: &str, status: &str, generation: i64) -> feed_refresh_run::ActiveModel {
    feed_refresh_run::ActiveModel {
        id: Set(id.to_owned()),
        feed_id: Set(support::database::FEED_ID.to_owned()),
        requested_by_user_id: Set(Some(USER_A_ID.to_owned())),
        trigger_kind: Set("MANUAL".to_owned()),
        status: Set(status.to_owned()),
        idempotency_key: Set(format!("subscription-contract-{generation}")),
        lease_token: Set(None),
        commit_generation: Set(Some(generation)),
        queued_at: Set(FIXTURE_AT + time::Duration::minutes(5)),
        started_at: Set(Some(FIXTURE_AT + time::Duration::minutes(5))),
        fetched_at: Set(Some(FIXTURE_AT + time::Duration::minutes(5))),
        persisted_at: Set(Some(FIXTURE_AT + time::Duration::minutes(5))),
        completed_at: Set(Some(FIXTURE_AT + time::Duration::minutes(5))),
        http_status: Set(Some(200)),
        new_count: Set(generation as i32),
        updated_count: Set(0),
        dropped_count: Set(0),
        error_code: Set(None),
        retry_at: Set(None),
    }
}
