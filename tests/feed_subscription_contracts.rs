#[allow(dead_code)]
mod support;

use base64::Engine;
use raindrop::db::{
    entities::{entry, entry_state, feed, feed_refresh_run, subscription},
    migrate, rollback,
};
use raindrop::feeds::{
    FeedRepository, FeedUrlPolicy, ListSubscriptionsQuery, PatchValue, QueueSubscriptionRefresh,
    RefreshRepositoryError, RepositoryError, UpdateSubscription,
};
use raindrop::organization::{CategoryRepository, CreateCategory};
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, ConnectionTrait, DatabaseConnection,
    DbBackend, EntityTrait, PaginatorTrait, QueryFilter, Statement,
};
use sea_orm_migration::SchemaManager;
use secrecy::SecretString;
use std::{sync::Arc, time::Duration as StdDuration};
use tempfile::TempDir;
use time::{OffsetDateTime, macros::datetime};
use uuid::Uuid;

use support::database::{
    SUBSCRIPTION_A_ID, SUBSCRIPTION_B_ID, USER_A_ID, USER_B_ID, connect_for_contract, insert_user,
};

const SECOND_FEED_ID: &str = "00000000-0000-4000-8000-000000000102";
const QUOTA_TARGET_FEED_ID: &str = "00000000-0000-4000-8000-000000000104";
const NOISE_FEED_ID: &str = "00000000-0000-4000-8000-000000000103";
const SECOND_SUBSCRIPTION_A_ID: &str = "00000000-0000-4000-8000-000000000203";
const ENTRY_1_ID: &str = "00000000-0000-4000-8000-000000000301";
const ENTRY_2_ID: &str = "00000000-0000-4000-8000-000000000302";
const ENTRY_3_ID: &str = "00000000-0000-4000-8000-000000000303";
const REFRESH_RUN_1_ID: &str = "00000000-0000-4000-8000-000000000401";
const REFRESH_RUN_2_ID: &str = "00000000-0000-4000-8000-000000000402";
const REFRESH_RUN_3_ID: &str = "00000000-0000-4000-8000-000000000400";
const MANUAL_REQUEST_A_ID: &str = "00000000-0000-4000-8000-000000000501";
const MANUAL_REQUEST_B_ID: &str = "00000000-0000-4000-8000-000000000502";
const SHARED_FEED_URL: &str = "https://shared.example.test/feed.xml";
const SECOND_FEED_URL: &str = "https://second.example.test/feed.xml";
const QUOTA_TARGET_FEED_URL: &str = "https://quota-target.example.test/feed.xml";
const FIXTURE_AT: OffsetDateTime = datetime!(2026-07-16 12:00:00 UTC);

struct SubscriptionFixture {
    _data: TempDir,
    database_url: String,
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

#[tokio::test]
async fn postgres_subscription_command_concurrency_contract() {
    let Ok(url) = std::env::var("RAINDROP_TEST_POSTGRES_URL") else {
        eprintln!("skipping PostgreSQL subscription command contract; test URL is unset");
        return;
    };
    backend_subscription_command_contract(&url, "postgres").await;
}

#[tokio::test]
async fn mysql_subscription_command_concurrency_contract() {
    let Ok(url) = std::env::var("RAINDROP_TEST_MYSQL_URL") else {
        eprintln!("skipping MySQL subscription command contract; test URL is unset");
        return;
    };
    backend_subscription_command_contract(&url, "mysql").await;
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
    let manager = SchemaManager::new(&database);
    for (table, index) in [
        ("subscriptions", "uq_subscriptions_user_feed"),
        ("subscriptions", "idx_subscriptions_user_pos"),
        ("entries", "idx_entries_feed_list"),
        ("entries", "uq_entries_feed_seq"),
        ("entries", "idx_entries_snapshot"),
        ("feed_refresh_runs", "idx_refresh_runs_feed"),
        ("feed_refresh_runs", "uq_refresh_runs_idem"),
    ] {
        assert!(
            manager
                .has_index(table, index)
                .await
                .unwrap_or_else(|_| panic!("{backend_name} index should query")),
            "{backend_name} index should exist: {table}.{index}"
        );
    }
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
    assert!(
        !plan.is_empty(),
        "{backend_name} subscription EXPLAIN should return a plan"
    );
    database.close().await.expect("database should close");
}

async fn backend_subscription_command_contract(url: &str, backend_name: &str) {
    backend_queue_race_contract(url, backend_name).await;
    backend_quota_contract(url, backend_name).await;
    backend_orphan_resubscribe_contract(url, backend_name).await;
}

async fn reset_backend_database(url: &str, backend_name: &str) -> DatabaseConnection {
    let database = connect_for_contract(SecretString::from(url.to_owned())).await;
    rollback(&database)
        .await
        .unwrap_or_else(|_| panic!("{backend_name} command database should reset"));
    migrate(&database)
        .await
        .unwrap_or_else(|_| panic!("{backend_name} command migrations should apply"));
    database
}

async fn backend_queue_race_contract(url: &str, backend_name: &str) {
    let database = reset_backend_database(url, backend_name).await;
    insert_user(&database, USER_A_ID, "backend-command-a").await;
    insert_user(&database, USER_B_ID, "backend-command-b").await;
    let mut feed = feed_model(
        support::database::FEED_ID,
        SHARED_FEED_URL,
        Some("Shared feed"),
        None,
    );
    feed.entry_sequence_head = Set(0);
    feed.next_fetch_at = Set(FIXTURE_AT);
    feed.insert(&database)
        .await
        .unwrap_or_else(|_| panic!("{backend_name} due feed should insert"));
    let second_database = connect_for_contract(SecretString::from(url.to_owned())).await;
    let first = FeedRepository::new(database.clone());
    let second = FeedRepository::new(second_database.clone());
    let barrier = Arc::new(tokio::sync::Barrier::new(2));
    let first_barrier = Arc::clone(&barrier);
    let second_barrier = Arc::clone(&barrier);
    let normalized_a = FeedUrlPolicy::new(false)
        .normalize(SHARED_FEED_URL)
        .unwrap();
    let normalized_b = normalized_a.clone();
    let (first, second) = tokio::time::timeout(StdDuration::from_secs(10), async move {
        tokio::join!(
            async move {
                first_barrier.wait().await;
                first
                    .subscribe(USER_A_ID, SHARED_FEED_URL, &normalized_a)
                    .await
            },
            async move {
                second_barrier.wait().await;
                second
                    .subscribe(USER_B_ID, SHARED_FEED_URL, &normalized_b)
                    .await
            }
        )
    })
    .await
    .unwrap_or_else(|_| panic!("{backend_name} queue race must not deadlock"));
    let first = first.unwrap_or_else(|_| panic!("{backend_name} first subscribe should succeed"));
    let second =
        second.unwrap_or_else(|_| panic!("{backend_name} second subscribe should succeed"));
    assert_eq!(
        first
            .subscription
            .refresh
            .expect("due backend feed should queue")
            .run_id,
        second
            .subscription
            .refresh
            .expect("backend active run should be referenced")
            .run_id
    );
    assert_eq!(
        active_feed_run_count(&database, support::database::FEED_ID).await,
        1,
        "{backend_name} queue race must persist exactly one active run"
    );
    database.close().await.expect("database should close");
    second_database
        .close()
        .await
        .expect("second database should close");
}

async fn backend_quota_contract(url: &str, backend_name: &str) {
    let database = reset_backend_database(url, backend_name).await;
    insert_user(&database, USER_A_ID, "backend-quota-a").await;
    seed_subscription_quota(&database, 999).await;
    for (feed_id, feed_url) in [
        (support::database::FEED_ID, SHARED_FEED_URL),
        (QUOTA_TARGET_FEED_ID, QUOTA_TARGET_FEED_URL),
    ] {
        let mut feed = feed_model(feed_id, feed_url, Some("Quota target"), None);
        feed.entry_sequence_head = Set(0);
        feed.next_fetch_at = Set(FIXTURE_AT + time::Duration::days(3_650));
        feed.insert(&database)
            .await
            .unwrap_or_else(|_| panic!("{backend_name} quota target should insert"));
    }
    let second_database = connect_for_contract(SecretString::from(url.to_owned())).await;
    let first = FeedRepository::new(database.clone());
    let second = FeedRepository::new(second_database.clone());
    let barrier = Arc::new(tokio::sync::Barrier::new(2));
    let first_barrier = Arc::clone(&barrier);
    let second_barrier = Arc::clone(&barrier);
    let shared = FeedUrlPolicy::new(false)
        .normalize(SHARED_FEED_URL)
        .unwrap();
    let target = FeedUrlPolicy::new(false)
        .normalize(QUOTA_TARGET_FEED_URL)
        .unwrap();
    let (shared, target) = tokio::time::timeout(StdDuration::from_secs(15), async move {
        tokio::join!(
            async move {
                first_barrier.wait().await;
                first.subscribe(USER_A_ID, SHARED_FEED_URL, &shared).await
            },
            async move {
                second_barrier.wait().await;
                second
                    .subscribe(USER_A_ID, QUOTA_TARGET_FEED_URL, &target)
                    .await
            }
        )
    })
    .await
    .unwrap_or_else(|_| panic!("{backend_name} subscription quota must not deadlock"));
    assert!(matches!(
        (&shared, &target),
        (Ok(_), Err(RefreshRepositoryError::SubscriptionLimit))
            | (Err(RefreshRepositoryError::SubscriptionLimit), Ok(_))
    ));
    assert_eq!(
        user_subscription_count(&database, USER_A_ID).await,
        1_000,
        "{backend_name} subscription quota race must persist the exact ceiling"
    );
    database.close().await.expect("database should close");
    second_database
        .close()
        .await
        .expect("second database should close");

    let database = reset_backend_database(url, backend_name).await;
    seed_fixture(&database).await;
    seed_active_user_refresh_quota(&database, 19).await;
    let second_database = connect_for_contract(SecretString::from(url.to_owned())).await;
    let first = FeedRepository::new(database.clone());
    let second = FeedRepository::new(second_database.clone());
    let barrier = Arc::new(tokio::sync::Barrier::new(2));
    let first_barrier = Arc::clone(&barrier);
    let second_barrier = Arc::clone(&barrier);
    let (shared, second) = tokio::time::timeout(StdDuration::from_secs(10), async move {
        tokio::join!(
            async move {
                first_barrier.wait().await;
                first
                    .queue_subscription_refresh(
                        USER_A_ID,
                        SUBSCRIPTION_A_ID,
                        QueueSubscriptionRefresh {
                            request_id: MANUAL_REQUEST_A_ID.to_owned(),
                        },
                    )
                    .await
            },
            async move {
                second_barrier.wait().await;
                second
                    .queue_subscription_refresh(
                        USER_A_ID,
                        SECOND_SUBSCRIPTION_A_ID,
                        QueueSubscriptionRefresh {
                            request_id: MANUAL_REQUEST_B_ID.to_owned(),
                        },
                    )
                    .await
            }
        )
    })
    .await
    .unwrap_or_else(|_| panic!("{backend_name} active quota must not deadlock"));
    let (accepted, rejected, accepted_subscription, accepted_request) = match (shared, second) {
        (Ok(accepted), Err(rejected)) => {
            (accepted, rejected, SUBSCRIPTION_A_ID, MANUAL_REQUEST_A_ID)
        }
        (Err(rejected), Ok(accepted)) => (
            accepted,
            rejected,
            SECOND_SUBSCRIPTION_A_ID,
            MANUAL_REQUEST_B_ID,
        ),
        results => panic!("{backend_name} exactly one manual refresh should fit: {results:?}"),
    };
    assert!(matches!(
        rejected,
        RefreshRepositoryError::ActiveRefreshLimit
    ));
    assert_eq!(
        active_user_requested_run_count(&database, USER_A_ID).await,
        20,
        "{backend_name} active quota race must persist the exact ceiling"
    );
    let run_count = feed_refresh_run::Entity::find()
        .count(&database)
        .await
        .expect("backend refresh runs should count before replay");
    let subscription_count = user_subscription_count(&database, USER_A_ID).await;
    let replay_repository = FeedRepository::new(database.clone());
    for replay_index in 1..=2 {
        let replay = replay_repository
            .queue_subscription_refresh(
                USER_A_ID,
                accepted_subscription,
                QueueSubscriptionRefresh {
                    request_id: accepted_request.to_owned(),
                },
            )
            .await
            .unwrap_or_else(|_| {
                panic!("{backend_name} exact replay {replay_index} should succeed")
            });
        assert_eq!(replay.run_id, accepted.run_id);
        assert_eq!(
            feed_refresh_run::Entity::find()
                .count(&database)
                .await
                .expect("backend refresh runs should count after replay"),
            run_count,
            "{backend_name} exact replay {replay_index} must not insert a run"
        );
        assert_eq!(
            user_subscription_count(&database, USER_A_ID).await,
            subscription_count,
            "{backend_name} exact replay {replay_index} must not change subscriptions"
        );
    }
    database.close().await.expect("database should close");
    second_database
        .close()
        .await
        .expect("second database should close");
}

async fn backend_orphan_resubscribe_contract(url: &str, backend_name: &str) {
    let database = reset_backend_database(url, backend_name).await;
    insert_user(&database, USER_A_ID, "backend-orphan-a").await;
    insert_user(&database, USER_B_ID, "backend-orphan-b").await;
    let mut feed = feed_model(
        support::database::FEED_ID,
        SHARED_FEED_URL,
        Some("Shared feed"),
        None,
    );
    feed.entry_sequence_head = Set(0);
    feed.next_fetch_at = Set(FIXTURE_AT + time::Duration::days(3_650));
    feed.insert(&database)
        .await
        .unwrap_or_else(|_| panic!("{backend_name} orphan feed should insert"));
    let repository = FeedRepository::new(database.clone());
    let normalized = FeedUrlPolicy::new(false)
        .normalize(SHARED_FEED_URL)
        .unwrap();
    let existing = repository
        .subscribe(USER_B_ID, SHARED_FEED_URL, &normalized)
        .await
        .unwrap_or_else(|_| panic!("{backend_name} initial subscription should succeed"));
    let second_database = connect_for_contract(SecretString::from(url.to_owned())).await;
    let second = FeedRepository::new(second_database.clone());
    let barrier = Arc::new(tokio::sync::Barrier::new(2));
    let unsubscribe_barrier = Arc::clone(&barrier);
    let subscribe_barrier = Arc::clone(&barrier);
    let subscription_id = existing.subscription.subscription_id;
    let (unsubscribed, resubscribed) =
        tokio::time::timeout(StdDuration::from_secs(10), async move {
            tokio::join!(
                async move {
                    unsubscribe_barrier.wait().await;
                    repository.unsubscribe(USER_B_ID, &subscription_id).await
                },
                async move {
                    subscribe_barrier.wait().await;
                    second
                        .subscribe(USER_A_ID, SHARED_FEED_URL, &normalized)
                        .await
                }
            )
        })
        .await
        .unwrap_or_else(|_| panic!("{backend_name} orphan/resubscribe must not deadlock"));
    assert!(unsubscribed.unwrap_or_else(|_| panic!("{backend_name} unsubscribe should succeed")));
    assert!(
        resubscribed
            .unwrap_or_else(|_| panic!("{backend_name} resubscribe should succeed"))
            .created
    );
    let feed = feed::Entity::find_by_id(support::database::FEED_ID)
        .one(&database)
        .await
        .expect("feed should query")
        .expect("feed should exist");
    assert!(feed.orphaned_at.is_none());
    database.close().await.expect("database should close");
    second_database
        .close()
        .await
        .expect("second database should close");
}

impl SubscriptionFixture {
    async fn new() -> Self {
        let data = tempfile::tempdir().expect("temporary directory should be created");
        let database_url = format!(
            "sqlite://{}?mode=rwc",
            data.path().join("feed-subscription-contracts.db").display()
        );
        let database = connect_for_contract(SecretString::from(database_url.clone())).await;
        migrate(&database)
            .await
            .expect("RSS migrations should apply");
        seed_fixture(&database).await;
        Self {
            _data: data,
            database_url,
            repository: FeedRepository::new(database.clone()),
            database,
        }
    }

    async fn with_feed_head(head: i64) -> Self {
        let data = tempfile::tempdir().expect("temporary directory should be created");
        let database_url = format!(
            "sqlite://{}?mode=rwc",
            data.path()
                .join("feed-subscription-command-contracts.db")
                .display()
        );
        let database = connect_for_contract(SecretString::from(database_url.clone())).await;
        migrate(&database)
            .await
            .expect("RSS migrations should apply");
        insert_user(&database, USER_A_ID, "subscription-command-reader-a").await;
        insert_user(&database, USER_B_ID, "subscription-command-reader-b").await;
        let mut feed = feed_model(
            support::database::FEED_ID,
            SHARED_FEED_URL,
            Some("Shared feed"),
            Some("https://shared.example.test"),
        );
        feed.entry_sequence_head = Set(head);
        feed.next_fetch_at = Set(FIXTURE_AT + time::Duration::days(3_650));
        feed.insert(&database)
            .await
            .expect("shared feed should insert");
        for sequence in 1..=head {
            entry_model(
                &Uuid::from_u128(0x7000_0000_0000_4000_8000_0000_0000_0000 + sequence as u128)
                    .to_string(),
                sequence,
            )
            .insert(&database)
            .await
            .expect("existing feed entry should insert");
        }
        Self {
            _data: data,
            database_url,
            repository: FeedRepository::new(database.clone()),
            database,
        }
    }

    async fn subscribe_user_b(&self) -> raindrop::feeds::SubscribeOutcome {
        self.subscribe_user(USER_B_ID).await
    }

    async fn subscribe_user(&self, user_id: &str) -> raindrop::feeds::SubscribeOutcome {
        let normalized = FeedUrlPolicy::new(false)
            .normalize(SHARED_FEED_URL)
            .expect("fixture feed URL should normalize");
        self.repository
            .subscribe(user_id, SHARED_FEED_URL, &normalized)
            .await
            .expect("subscription should succeed")
    }

    async fn subscription_row(&self, user_id: &str) -> subscription::Model {
        subscription::Entity::find()
            .filter(subscription::Column::UserId.eq(user_id))
            .one(&self.database)
            .await
            .expect("subscription should query")
            .expect("subscription should exist")
    }

    async fn second_repository(&self) -> FeedRepository {
        let database = connect_for_contract(SecretString::from(self.database_url.clone())).await;
        FeedRepository::new(database)
    }
}

#[tokio::test]
async fn sqlite_new_subscription_sees_at_most_one_hundred_existing_entries_as_unread() {
    let fixture = SubscriptionFixture::with_feed_head(150).await;
    let outcome = fixture.subscribe_user_b().await;
    assert!(outcome.created);
    assert_eq!(outcome.subscription.unread_count, 100);
    let row = fixture.subscription_row(USER_B_ID).await;
    assert_eq!(row.start_sequence, 50);
    assert_eq!(row.read_through_sequence, 50);
}

#[tokio::test]
async fn sqlite_two_users_share_feed_and_fresh_history_without_duplicate_run() {
    let fixture = SubscriptionFixture::with_feed_head(60).await;
    let first = fixture.subscribe_user(USER_A_ID).await;
    let second = fixture.subscribe_user(USER_B_ID).await;
    assert!(first.created);
    assert!(second.created);
    assert_eq!(first.subscription.unread_count, 60);
    assert_eq!(second.subscription.unread_count, 60);
    assert!(first.subscription.refresh.is_none());
    assert!(second.subscription.refresh.is_none());
    assert_eq!(
        feed_refresh_run::Entity::find()
            .count(&fixture.database)
            .await
            .expect("refresh runs should count"),
        0
    );
}

#[tokio::test]
async fn sqlite_existing_head_sixty_exposes_sixty_unread_entries() {
    let fixture = SubscriptionFixture::with_feed_head(60).await;
    let outcome = fixture.subscribe_user_b().await;
    assert_eq!(outcome.subscription.unread_count, 60);
    let row = fixture.subscription_row(USER_B_ID).await;
    assert_eq!(row.start_sequence, 0);
    assert_eq!(row.read_through_sequence, 0);
}

#[tokio::test]
async fn sqlite_same_user_concurrent_subscribe_creates_one_relationship() {
    let fixture = SubscriptionFixture::with_feed_head(60).await;
    let second = fixture.second_repository().await;
    let first = fixture.repository.clone();
    let barrier = Arc::new(tokio::sync::Barrier::new(2));
    let first_barrier = Arc::clone(&barrier);
    let second_barrier = Arc::clone(&barrier);
    let normalized_a = FeedUrlPolicy::new(false)
        .normalize(SHARED_FEED_URL)
        .unwrap();
    let normalized_b = normalized_a.clone();
    let (first, second) = tokio::join!(
        async move {
            first_barrier.wait().await;
            first
                .subscribe(USER_B_ID, SHARED_FEED_URL, &normalized_a)
                .await
        },
        async move {
            second_barrier.wait().await;
            second
                .subscribe(USER_B_ID, SHARED_FEED_URL, &normalized_b)
                .await
        }
    );
    let first = first.expect("first concurrent subscribe should succeed");
    let second = second.expect("second concurrent subscribe should succeed");
    assert_ne!(first.created, second.created);
    assert_eq!(
        first.subscription.subscription_id,
        second.subscription.subscription_id
    );
    let page = fixture
        .repository
        .list_subscriptions_for_user(USER_B_ID, ListSubscriptionsQuery::default())
        .await
        .expect("subscriptions should list");
    assert_eq!(page.items.len(), 1);
}

#[tokio::test]
async fn sqlite_due_feed_concurrent_subscribe_creates_one_active_run() {
    let fixture = SubscriptionFixture::with_feed_head(60).await;
    let feed = feed::Entity::find_by_id(support::database::FEED_ID)
        .one(&fixture.database)
        .await
        .expect("feed should query")
        .expect("feed should exist");
    let mut feed: feed::ActiveModel = feed.into();
    feed.next_fetch_at = Set(FIXTURE_AT);
    feed.update(&fixture.database)
        .await
        .expect("feed should become due");
    let second = fixture.second_repository().await;
    let first = fixture.repository.clone();
    let barrier = Arc::new(tokio::sync::Barrier::new(2));
    let first_barrier = Arc::clone(&barrier);
    let second_barrier = Arc::clone(&barrier);
    let normalized_a = FeedUrlPolicy::new(false)
        .normalize(SHARED_FEED_URL)
        .unwrap();
    let normalized_b = normalized_a.clone();
    let (first, second) = tokio::join!(
        async move {
            first_barrier.wait().await;
            first
                .subscribe(USER_A_ID, SHARED_FEED_URL, &normalized_a)
                .await
        },
        async move {
            second_barrier.wait().await;
            second
                .subscribe(USER_B_ID, SHARED_FEED_URL, &normalized_b)
                .await
        }
    );
    let first = first.expect("first due subscribe should succeed");
    let second = second.expect("second due subscribe should succeed");
    let first_refresh = first.subscription.refresh.expect("due feed should queue");
    let second_refresh = second
        .subscription
        .refresh
        .expect("active run should be referenced");
    assert_eq!(first_refresh.run_id, second_refresh.run_id);
    assert_eq!(first_refresh.status.as_str(), "QUEUED");
    assert_eq!(
        active_feed_run_count(&fixture.database, support::database::FEED_ID).await,
        1
    );
}

#[tokio::test]
async fn sqlite_manual_exact_request_replays_terminal_run_before_cooldown() {
    let fixture = SubscriptionFixture::new().await;
    let accepted = fixture
        .repository
        .queue_subscription_refresh(
            USER_A_ID,
            SUBSCRIPTION_A_ID,
            QueueSubscriptionRefresh {
                request_id: MANUAL_REQUEST_A_ID.to_owned(),
            },
        )
        .await
        .expect("manual refresh should queue");
    let run = feed_refresh_run::Entity::find_by_id(&accepted.run_id)
        .one(&fixture.database)
        .await
        .expect("manual run should query")
        .expect("manual run should exist");
    let mut run: feed_refresh_run::ActiveModel = run.into();
    let completed_at = FIXTURE_AT + time::Duration::minutes(20);
    run.status = Set("ERROR".to_owned());
    run.error_code = Set(Some("FETCH_FAILED".to_owned()));
    run.retry_at = Set(Some(FIXTURE_AT + time::Duration::days(3_650)));
    run.completed_at = Set(Some(completed_at));
    run.update(&fixture.database)
        .await
        .expect("manual run should become terminal");
    let feed = feed::Entity::find_by_id(support::database::FEED_ID)
        .one(&fixture.database)
        .await
        .expect("feed should query")
        .expect("feed should exist");
    let mut feed: feed::ActiveModel = feed.into();
    feed.last_attempt_at = Set(Some(FIXTURE_AT + time::Duration::days(3_650)));
    feed.update(&fixture.database)
        .await
        .expect("feed should enter cooldown");

    let replay = fixture
        .repository
        .queue_subscription_refresh(
            USER_A_ID,
            SUBSCRIPTION_A_ID,
            QueueSubscriptionRefresh {
                request_id: MANUAL_REQUEST_A_ID.to_owned(),
            },
        )
        .await
        .expect("exact terminal request should replay before cooldown");
    assert_eq!(replay.run_id, accepted.run_id);
    assert_eq!(replay.status.as_str(), "ERROR");
    assert_eq!(replay.error_code.as_deref(), Some("FETCH_FAILED"));
    assert_eq!(replay.completed_at, Some(completed_at));
}

#[tokio::test]
async fn sqlite_manual_different_request_rejects_while_active() {
    let fixture = SubscriptionFixture::new().await;
    let accepted = fixture
        .repository
        .queue_subscription_refresh(
            USER_A_ID,
            SUBSCRIPTION_A_ID,
            QueueSubscriptionRefresh {
                request_id: MANUAL_REQUEST_A_ID.to_owned(),
            },
        )
        .await
        .expect("first manual refresh should queue");
    let error = fixture
        .repository
        .queue_subscription_refresh(
            USER_A_ID,
            SUBSCRIPTION_A_ID,
            QueueSubscriptionRefresh {
                request_id: MANUAL_REQUEST_B_ID.to_owned(),
            },
        )
        .await
        .expect_err("different request should not queue beside active work");
    assert!(matches!(
        &error,
        RefreshRepositoryError::RefreshInProgress { operation_id }
            if operation_id == &accepted.run_id
    ));
    assert!(!format!("{error:?}").contains(&accepted.run_id));
}

#[tokio::test]
async fn sqlite_manual_key_fits_all_backend_limits() {
    let fixture = SubscriptionFixture::new().await;
    let accepted = fixture
        .repository
        .queue_subscription_refresh(
            USER_A_ID,
            SUBSCRIPTION_A_ID,
            QueueSubscriptionRefresh {
                request_id: MANUAL_REQUEST_A_ID.to_owned(),
            },
        )
        .await
        .expect("framed manual key should fit the persisted backend limit");
    assert_eq!(accepted.status.as_str(), "QUEUED");
    let request = QueueSubscriptionRefresh {
        request_id: MANUAL_REQUEST_A_ID.to_owned(),
    };
    assert!(!format!("{request:?}").contains(MANUAL_REQUEST_A_ID));
}

#[tokio::test]
async fn sqlite_manual_cooldown_respects_retry_after() {
    let fixture = SubscriptionFixture::new().await;
    let database_now = sqlite_database_now(&fixture.database).await;
    let retry_at = database_now + time::Duration::seconds(60) + time::Duration::milliseconds(900);
    let feed = feed::Entity::find_by_id(support::database::FEED_ID)
        .one(&fixture.database)
        .await
        .expect("feed should query")
        .expect("feed should exist");
    let mut feed: feed::ActiveModel = feed.into();
    feed.last_attempt_at = Set(None);
    feed.retry_after_at = Set(Some(retry_at));
    feed.update(&fixture.database)
        .await
        .expect("feed retry-after should update");
    let error = fixture
        .repository
        .queue_subscription_refresh(
            USER_A_ID,
            SUBSCRIPTION_A_ID,
            QueueSubscriptionRefresh {
                request_id: MANUAL_REQUEST_A_ID.to_owned(),
            },
        )
        .await
        .expect_err("retry-after should govern manual cooldown");
    match &error {
        RefreshRepositoryError::RefreshCooldown {
            retry_at: actual,
            retry_after_seconds,
        } => {
            assert_eq!(*actual, retry_at);
            assert_eq!(*retry_after_seconds, 61);
        }
        error => panic!("expected cooldown error, got {error:?}"),
    }
    assert_eq!(
        format!("{error:?}"),
        "RefreshRepositoryError::RefreshCooldown([REDACTED])"
    );
}

async fn sqlite_database_now(database: &DatabaseConnection) -> OffsetDateTime {
    database
        .query_one(Statement::from_string(
            DbBackend::Sqlite,
            "SELECT strftime('%Y-%m-%dT%H:%M:%f000Z','now') AS database_now".to_owned(),
        ))
        .await
        .expect("database time should query")
        .expect("database time row should exist")
        .try_get("", "database_now")
        .expect("database time should decode")
}

#[tokio::test]
async fn sqlite_unsubscribe_is_idempotent_and_marks_only_last_feed_orphan() {
    let fixture = SubscriptionFixture::new().await;
    assert!(
        fixture
            .repository
            .unsubscribe(USER_A_ID, SUBSCRIPTION_A_ID)
            .await
            .expect("owned unsubscribe should succeed")
    );
    let feed = feed::Entity::find_by_id(support::database::FEED_ID)
        .one(&fixture.database)
        .await
        .expect("feed should query")
        .expect("feed should exist");
    assert!(feed.orphaned_at.is_none());
    assert!(
        !fixture
            .repository
            .unsubscribe(USER_A_ID, SUBSCRIPTION_A_ID)
            .await
            .expect("repeated unsubscribe should be idempotent")
    );
    assert!(
        !fixture
            .repository
            .unsubscribe(USER_A_ID, SUBSCRIPTION_B_ID)
            .await
            .expect("cross-tenant unsubscribe should stay opaque")
    );
    assert!(
        fixture
            .repository
            .unsubscribe(USER_B_ID, SUBSCRIPTION_B_ID)
            .await
            .expect("last unsubscribe should succeed")
    );
    let feed = feed::Entity::find_by_id(support::database::FEED_ID)
        .one(&fixture.database)
        .await
        .expect("feed should query")
        .expect("feed should exist");
    assert!(feed.orphaned_at.is_some());
}

#[tokio::test]
async fn sqlite_concurrent_resubscribe_clears_orphan() {
    let fixture = SubscriptionFixture::with_feed_head(60).await;
    let existing = fixture.subscribe_user_b().await;
    let second = fixture.second_repository().await;
    let first = fixture.repository.clone();
    let barrier = Arc::new(tokio::sync::Barrier::new(2));
    let unsubscribe_barrier = Arc::clone(&barrier);
    let subscribe_barrier = Arc::clone(&barrier);
    let normalized = FeedUrlPolicy::new(false)
        .normalize(SHARED_FEED_URL)
        .unwrap();
    let (unsubscribed, resubscribed) = tokio::join!(
        async move {
            unsubscribe_barrier.wait().await;
            first
                .unsubscribe(USER_B_ID, &existing.subscription.subscription_id)
                .await
        },
        async move {
            subscribe_barrier.wait().await;
            second
                .subscribe(USER_A_ID, SHARED_FEED_URL, &normalized)
                .await
        }
    );
    assert!(unsubscribed.expect("concurrent unsubscribe should succeed"));
    let resubscribed = resubscribed.expect("concurrent resubscribe should succeed");
    assert!(resubscribed.created);
    let feed = feed::Entity::find_by_id(support::database::FEED_ID)
        .one(&fixture.database)
        .await
        .expect("feed should query")
        .expect("feed should exist");
    assert!(feed.orphaned_at.is_none());
    assert!(
        fixture
            .repository
            .get_subscription_for_user(USER_A_ID, &resubscribed.subscription.subscription_id,)
            .await
            .expect("resubscribed relationship should query")
            .is_some()
    );
}

#[tokio::test]
async fn sqlite_subscribe_outcome_uses_the_locked_transaction_snapshot() {
    let fixture = SubscriptionFixture::new().await;
    fixture
        .database
        .execute_unprepared(&format!(
            "CREATE TRIGGER slow_atomic_subscribe_user_lock
             AFTER UPDATE OF is_disabled ON users
             WHEN NEW.id = '{USER_B_ID}'
             BEGIN
                 SELECT length(randomblob(50000000));
             END"
        ))
        .await
        .expect("subscription lock trigger should install");
    let first = fixture.repository.clone();
    let delete_database = fixture.database.clone();
    let normalized = FeedUrlPolicy::new(false)
        .normalize(SHARED_FEED_URL)
        .unwrap();
    let subscribe = tokio::spawn(async move {
        first
            .subscribe(USER_B_ID, SHARED_FEED_URL, &normalized)
            .await
    });
    tokio::time::sleep(StdDuration::from_millis(10)).await;
    let unsubscribe = tokio::spawn(async move {
        delete_database
            .execute_unprepared(&format!(
                "DELETE FROM subscriptions
                 WHERE id = '{SUBSCRIPTION_B_ID}' AND user_id = '{USER_B_ID}'"
            ))
            .await
    });
    let (outcome, unsubscribed) = tokio::time::timeout(StdDuration::from_secs(10), async {
        tokio::join!(subscribe, unsubscribe)
    })
    .await
    .expect("subscribe/unsubscribe snapshot race must not deadlock");
    let outcome = outcome
        .expect("subscribe task should join")
        .expect("subscribe outcome must survive the later unsubscribe");
    assert!(!outcome.created);
    assert_eq!(outcome.subscription.subscription_id, SUBSCRIPTION_B_ID);
    assert_eq!(
        unsubscribed
            .expect("unsubscribe task should join")
            .expect("concurrent unsubscribe should execute")
            .rows_affected(),
        1
    );
}

#[tokio::test]
async fn sqlite_subscription_and_active_run_quotas_are_atomic() {
    let subscription_fixture = SubscriptionFixture::with_feed_head(0).await;
    seed_subscription_quota(&subscription_fixture.database, 999).await;
    let mut quota_target = feed_model(
        QUOTA_TARGET_FEED_ID,
        QUOTA_TARGET_FEED_URL,
        Some("Quota target"),
        None,
    );
    quota_target.entry_sequence_head = Set(0);
    quota_target.next_fetch_at = Set(FIXTURE_AT + time::Duration::days(3_650));
    quota_target
        .insert(&subscription_fixture.database)
        .await
        .expect("quota target feed should insert");
    let second = subscription_fixture.second_repository().await;
    let first = subscription_fixture.repository.clone();
    let barrier = Arc::new(tokio::sync::Barrier::new(2));
    let first_barrier = Arc::clone(&barrier);
    let second_barrier = Arc::clone(&barrier);
    let shared = FeedUrlPolicy::new(false)
        .normalize(SHARED_FEED_URL)
        .unwrap();
    let target = FeedUrlPolicy::new(false)
        .normalize(QUOTA_TARGET_FEED_URL)
        .unwrap();
    let (shared_result, target_result) = tokio::join!(
        async move {
            first_barrier.wait().await;
            first.subscribe(USER_A_ID, SHARED_FEED_URL, &shared).await
        },
        async move {
            second_barrier.wait().await;
            second
                .subscribe(USER_A_ID, QUOTA_TARGET_FEED_URL, &target)
                .await
        }
    );
    let (created, rejected, created_url) = match (shared_result, target_result) {
        (Ok(created), Err(rejected)) => (created, rejected, SHARED_FEED_URL),
        (Err(rejected), Ok(created)) => (created, rejected, QUOTA_TARGET_FEED_URL),
        results => panic!("exactly one subscription should fit the quota: {results:?}"),
    };
    assert!(created.created);
    assert!(matches!(
        rejected,
        RefreshRepositoryError::SubscriptionLimit
    ));
    let normalized = FeedUrlPolicy::new(false).normalize(created_url).unwrap();
    let duplicate = subscription_fixture
        .repository
        .subscribe(USER_A_ID, created_url, &normalized)
        .await
        .expect("duplicate subscription should replay at the ceiling");
    assert!(!duplicate.created);
    assert_eq!(
        duplicate.subscription.subscription_id,
        created.subscription.subscription_id
    );
    assert_eq!(
        user_subscription_count(&subscription_fixture.database, USER_A_ID).await,
        1_000
    );

    let refresh_fixture = SubscriptionFixture::new().await;
    seed_active_user_refresh_quota(&refresh_fixture.database, 19).await;
    let second = refresh_fixture.second_repository().await;
    let first = refresh_fixture.repository.clone();
    let barrier = Arc::new(tokio::sync::Barrier::new(2));
    let first_barrier = Arc::clone(&barrier);
    let second_barrier = Arc::clone(&barrier);
    let (shared_result, second_result) = tokio::join!(
        async move {
            first_barrier.wait().await;
            first
                .queue_subscription_refresh(
                    USER_A_ID,
                    SUBSCRIPTION_A_ID,
                    QueueSubscriptionRefresh {
                        request_id: MANUAL_REQUEST_A_ID.to_owned(),
                    },
                )
                .await
        },
        async move {
            second_barrier.wait().await;
            second
                .queue_subscription_refresh(
                    USER_A_ID,
                    SECOND_SUBSCRIPTION_A_ID,
                    QueueSubscriptionRefresh {
                        request_id: MANUAL_REQUEST_B_ID.to_owned(),
                    },
                )
                .await
        }
    );
    let (accepted, rejected, accepted_subscription, accepted_request) =
        match (shared_result, second_result) {
            (Ok(accepted), Err(rejected)) => {
                (accepted, rejected, SUBSCRIPTION_A_ID, MANUAL_REQUEST_A_ID)
            }
            (Err(rejected), Ok(accepted)) => (
                accepted,
                rejected,
                SECOND_SUBSCRIPTION_A_ID,
                MANUAL_REQUEST_B_ID,
            ),
            results => panic!("exactly one manual refresh should fit the quota: {results:?}"),
        };
    assert_eq!(accepted.status.as_str(), "QUEUED");
    assert!(matches!(
        rejected,
        RefreshRepositoryError::ActiveRefreshLimit
    ));
    assert_eq!(
        active_user_requested_run_count(&refresh_fixture.database, USER_A_ID).await,
        20
    );
    let run_count = feed_refresh_run::Entity::find()
        .count(&refresh_fixture.database)
        .await
        .expect("refresh runs should count before exact replay");
    let subscription_count = user_subscription_count(&refresh_fixture.database, USER_A_ID).await;
    let replay = refresh_fixture
        .repository
        .queue_subscription_refresh(
            USER_A_ID,
            accepted_subscription,
            QueueSubscriptionRefresh {
                request_id: accepted_request.to_owned(),
            },
        )
        .await
        .expect("exact manual replay should win at the active-run ceiling");
    assert_eq!(replay.run_id, accepted.run_id);
    let replay_again = refresh_fixture
        .repository
        .queue_subscription_refresh(
            USER_A_ID,
            accepted_subscription,
            QueueSubscriptionRefresh {
                request_id: accepted_request.to_owned(),
            },
        )
        .await
        .expect("second exact manual replay should remain idempotent");
    assert_eq!(replay_again.run_id, accepted.run_id);
    assert_eq!(
        feed_refresh_run::Entity::find()
            .count(&refresh_fixture.database)
            .await
            .expect("refresh runs should count after exact replays"),
        run_count
    );
    assert_eq!(
        user_subscription_count(&refresh_fixture.database, USER_A_ID).await,
        subscription_count
    );
    assert_eq!(
        active_user_requested_run_count(&refresh_fixture.database, USER_A_ID).await,
        20
    );
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
async fn sqlite_subscription_patch_assigns_clears_and_hides_category_ownership() {
    let fixture = SubscriptionFixture::new().await;
    let categories = CategoryRepository::new(fixture.database.clone());
    let owned_category = categories
        .create(
            USER_A_ID,
            CreateCategory {
                title: "Technology".to_owned(),
            },
        )
        .await
        .expect("owned category should create");
    let foreign_category = categories
        .create(
            USER_B_ID,
            CreateCategory {
                title: "Private".to_owned(),
            },
        )
        .await
        .expect("foreign category should create");

    let assigned = fixture
        .repository
        .update_subscription_for_user(
            USER_A_ID,
            SUBSCRIPTION_A_ID,
            UpdateSubscription {
                category_id: PatchValue::Value(owned_category.category_id.clone()),
                title_override: PatchValue::Value("Focused title".to_owned()),
                position: Some(512),
            },
        )
        .await
        .expect("owned subscription patch should execute")
        .expect("owned subscription should update");
    assert_eq!(
        assigned.category_id,
        Some(owned_category.category_id.clone())
    );
    assert_eq!(assigned.title_override.as_deref(), Some("Focused title"));
    assert_eq!(assigned.title, "Focused title");
    assert_eq!(assigned.position, 512);

    let foreign_assignment = fixture
        .repository
        .update_subscription_for_user(
            USER_A_ID,
            SUBSCRIPTION_A_ID,
            UpdateSubscription {
                category_id: PatchValue::Value(foreign_category.category_id),
                title_override: PatchValue::Missing,
                position: None,
            },
        )
        .await
        .expect("foreign category lookup should remain typed");
    assert!(foreign_assignment.is_none());
    let unchanged = fixture
        .repository
        .get_subscription_for_user(USER_A_ID, SUBSCRIPTION_A_ID)
        .await
        .expect("unchanged subscription should query")
        .expect("unchanged subscription should remain visible");
    assert_eq!(unchanged.category_id, Some(owned_category.category_id));
    assert_eq!(unchanged.title_override.as_deref(), Some("Focused title"));
    assert_eq!(unchanged.position, 512);

    assert!(
        fixture
            .repository
            .update_subscription_for_user(
                USER_A_ID,
                SUBSCRIPTION_B_ID,
                UpdateSubscription {
                    category_id: PatchValue::Null,
                    title_override: PatchValue::Null,
                    position: Some(1024),
                },
            )
            .await
            .expect("foreign subscription lookup should remain typed")
            .is_none()
    );

    let cleared = fixture
        .repository
        .update_subscription_for_user(
            USER_A_ID,
            SUBSCRIPTION_A_ID,
            UpdateSubscription {
                category_id: PatchValue::Null,
                title_override: PatchValue::Null,
                position: None,
            },
        )
        .await
        .expect("clear patch should execute")
        .expect("owned subscription should clear");
    assert!(cleared.category_id.is_none());
    assert!(cleared.title_override.is_none());
    assert_eq!(cleared.title, "Shared feed");
    assert_eq!(cleared.position, 512);
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
    assert_eq!(refresh.queued_at, FIXTURE_AT + time::Duration::minutes(5));
    assert_eq!(
        refresh.started_at,
        Some(FIXTURE_AT + time::Duration::minutes(5))
    );
    assert_eq!(
        refresh.completed_at,
        Some(FIXTURE_AT + time::Duration::minutes(5))
    );
    assert!(refresh.error_code.is_none());
    assert!(refresh.retry_at.is_none());
    let debug = format!("{refresh:?}");
    assert!(!debug.contains(REFRESH_RUN_2_ID));
    assert!(!debug.contains("2026-07-16"));

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

#[tokio::test]
async fn sqlite_subscription_projection_bounds_refresh_fanout_to_selected_subscriptions() {
    let fixture = SubscriptionFixture::new().await;
    seed_target_user_refresh_fanout(&fixture.database).await;
    fixture
        .database
        .execute_unprepared("ANALYZE")
        .await
        .expect("SQLite statistics should collect");

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
        .expect("bounded subscription page should query");
    assert_eq!(page.items.len(), 1);
    assert_eq!(page.items[0].subscription_id, SUBSCRIPTION_A_ID);
    assert_eq!(
        page.items[0]
            .refresh
            .as_ref()
            .expect("selected feed should retain its latest refresh")
            .run_id,
        REFRESH_RUN_2_ID
    );
    let detail = fixture
        .repository
        .get_subscription_for_user(USER_A_ID, SUBSCRIPTION_A_ID)
        .await
        .expect("bounded subscription detail should query")
        .expect("selected subscription should remain visible");
    assert_eq!(detail.subscription_id, SUBSCRIPTION_A_ID);
    assert_eq!(
        detail
            .refresh
            .expect("selected detail should retain its latest refresh")
            .run_id,
        REFRESH_RUN_2_ID
    );

    let list_plan = fixture
        .repository
        .explain_list_subscriptions_for_user(
            USER_A_ID,
            ListSubscriptionsQuery {
                cursor: None,
                limit: 1,
            },
        )
        .await
        .expect("bounded list EXPLAIN should execute")
        .join("\n");
    assert!(
        list_plan.contains("selected_subscriptions"),
        "list must bind latest-run work to the requested page: {list_plan}"
    );
    let detail_plan = fixture
        .repository
        .explain_subscription_detail_for_user(USER_A_ID, SUBSCRIPTION_A_ID)
        .await
        .expect("bounded detail EXPLAIN should execute")
        .join("\n");
    assert!(
        detail_plan.contains("selected_subscriptions"),
        "detail must bind latest-run work to the selected subscription: {detail_plan}"
    );
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

async fn active_feed_run_count(database: &DatabaseConnection, feed_id: &str) -> u64 {
    feed_refresh_run::Entity::find()
        .filter(feed_refresh_run::Column::FeedId.eq(feed_id))
        .filter(feed_refresh_run::Column::Status.is_in(["QUEUED", "RUNNING"]))
        .count(database)
        .await
        .expect("active feed runs should count")
}

async fn user_subscription_count(database: &DatabaseConnection, user_id: &str) -> u64 {
    subscription::Entity::find()
        .filter(subscription::Column::UserId.eq(user_id))
        .count(database)
        .await
        .expect("user subscriptions should count")
}

async fn active_user_requested_run_count(database: &DatabaseConnection, user_id: &str) -> u64 {
    feed_refresh_run::Entity::find()
        .filter(feed_refresh_run::Column::RequestedByUserId.eq(user_id))
        .filter(feed_refresh_run::Column::Status.is_in(["QUEUED", "RUNNING"]))
        .count(database)
        .await
        .expect("user-requested active refresh runs should count")
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

async fn seed_target_user_refresh_fanout(database: &DatabaseConnection) {
    for feed_index in 0..12_u128 {
        let feed_id =
            Uuid::from_u128(0x6000_0000_0000_4000_8000_0000_0000_0000 + feed_index).to_string();
        feed_model(
            &feed_id,
            &format!("https://fanout-{feed_index:03}.example.test/feed.xml"),
            Some("Fan-out feed"),
            None,
        )
        .insert(database)
        .await
        .expect("fan-out feed should insert");
        subscription_model(
            &Uuid::from_u128(0x6100_0000_0000_4000_8000_0000_0000_0000 + feed_index).to_string(),
            USER_A_ID,
            &feed_id,
            None,
            FIXTURE_AT - time::Duration::days(1) - time::Duration::seconds(feed_index as i64),
            0,
        )
        .insert(database)
        .await
        .expect("fan-out subscription should insert");
        for run_index in 0..16_u128 {
            let generation = 20_000 + (feed_index * 16 + run_index) as i64;
            let mut run = refresh_run_model(
                &Uuid::from_u128(
                    0x6200_0000_0000_4000_8000_0000_0000_0000 + feed_index * 16 + run_index,
                )
                .to_string(),
                "SUCCESS",
                generation,
            );
            run.feed_id = Set(feed_id.clone());
            run.queued_at = Set(FIXTURE_AT + time::Duration::seconds(run_index as i64));
            run.insert(database)
                .await
                .expect("fan-out refresh run should insert");
        }
    }
}

async fn seed_subscription_quota(database: &DatabaseConnection, count: u128) {
    for index in 0..count {
        let feed_id =
            Uuid::from_u128(0x8000_0000_0000_4000_8000_0000_0000_0000 + index).to_string();
        let subscription_id =
            Uuid::from_u128(0x8100_0000_0000_4000_8000_0000_0000_0000 + index).to_string();
        let url = format!("https://quota-{index:04}.example.test/feed.xml");
        feed_model(&feed_id, &url, Some("Quota feed"), None)
            .insert(database)
            .await
            .expect("quota feed should insert");
        subscription_model(&subscription_id, USER_A_ID, &feed_id, None, FIXTURE_AT, 0)
            .insert(database)
            .await
            .expect("quota subscription should insert");
    }
}

async fn seed_active_user_refresh_quota(database: &DatabaseConnection, count: u128) {
    for index in 0..count {
        let feed_id =
            Uuid::from_u128(0x8200_0000_0000_4000_8000_0000_0000_0000 + index).to_string();
        let run_id = Uuid::from_u128(0x8300_0000_0000_4000_8000_0000_0000_0000 + index).to_string();
        let url = format!("https://active-quota-{index:02}.example.test/feed.xml");
        feed_model(&feed_id, &url, Some("Active quota feed"), None)
            .insert(database)
            .await
            .expect("active quota feed should insert");
        feed_refresh_run::ActiveModel {
            id: Set(run_id),
            feed_id: Set(feed_id),
            requested_by_user_id: Set(Some(USER_A_ID.to_owned())),
            trigger_kind: Set("MANUAL".to_owned()),
            status: Set("QUEUED".to_owned()),
            idempotency_key: Set(format!("active-quota-{index}")),
            lease_token: Set(None),
            commit_generation: Set(None),
            queued_at: Set(FIXTURE_AT + time::Duration::seconds(index as i64)),
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
        .expect("active quota run should insert");
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
        category_id: Set(None),
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
