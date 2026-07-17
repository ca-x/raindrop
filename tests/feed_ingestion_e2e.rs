#[allow(dead_code)]
mod support;

use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use std::time::Duration;

use async_trait::async_trait;
use http::HeaderValue;
use raindrop::db::{
    entities::{entry, entry_state, feed, feed_refresh_run, lifecycle_outbox, subscription},
    migrate,
};
use raindrop::feeds::{
    ClaimRequest, EntryListState, ExactClaimResult, FeedFetchError, FeedParser, FeedRepository,
    FeedService, FeedTransport, FeedUrlPolicy, FetchOutcome, FetchRequest, FetchedDocument,
    JitterSource, ListEntriesQuery, OpaqueValidator, PersistFeed, QueueRefreshRequest,
    RefreshFailure, RefreshRepositoryError, RefreshResult, RefreshSchedule, RefreshTrigger,
    SubscribeInput,
};
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, ConnectionTrait, DatabaseConnection,
    EntityTrait, PaginatorTrait, QueryFilter, QueryOrder,
};
use secrecy::SecretString;
use tokio::sync::Notify;
use uuid::Uuid;

use support::database::{
    FEED_ID, USER_A_ID, USER_B_ID, connect_for_contract, insert_feed, insert_user,
};

const FEED_URL: &str = "https://feeds.example.test/synthetic.xml";

#[derive(Clone)]
struct ControlledTransport {
    calls: Arc<AtomicUsize>,
    first_entered: Arc<Notify>,
    first_release: Arc<Notify>,
    second_entered: Arc<Notify>,
    second_release: Arc<Notify>,
    body: Arc<Vec<u8>>,
}

impl ControlledTransport {
    fn new(body: Vec<u8>) -> Self {
        Self {
            calls: Arc::new(AtomicUsize::new(0)),
            first_entered: Arc::new(Notify::new()),
            first_release: Arc::new(Notify::new()),
            second_entered: Arc::new(Notify::new()),
            second_release: Arc::new(Notify::new()),
            body: Arc::new(body),
        }
    }
}

#[async_trait]
impl FeedTransport for ControlledTransport {
    async fn fetch(&self, request: FetchRequest) -> Result<FetchOutcome, FeedFetchError> {
        let call = self.calls.fetch_add(1, Ordering::SeqCst);
        match call {
            0 => {
                self.first_entered.notify_one();
                self.first_release.notified().await;
            }
            1 => {
                self.second_entered.notify_one();
                self.second_release.notified().await;
            }
            _ => panic!("deterministic E2E must make exactly two feed requests"),
        }

        Ok(FetchOutcome::Document {
            url: request.url().clone(),
            document: self.body.as_ref().clone(),
            content_type: Some("application/rss+xml; charset=utf-8".to_owned()),
            etag: Some(
                OpaqueValidator::from_header(HeaderValue::from_static("\"synthetic-v1\""))
                    .expect("fixture ETag is valid"),
            ),
            last_modified: None,
        })
    }
}

struct ZeroJitter;

impl JitterSource for ZeroJitter {
    fn sample_inclusive_us(&mut self, _upper_bound_us: u64) -> u64 {
        0
    }
}

#[tokio::test]
async fn two_users_securely_ingest_share_query_and_deduplicate_sixty_entries() {
    let data = tempfile::tempdir().expect("temporary directory should be created");
    let database_url = format!(
        "sqlite://{}?mode=rwc",
        data.path().join("feed-ingestion-e2e.db").display()
    );
    let database = connect_for_contract(SecretString::from(database_url)).await;
    migrate(&database)
        .await
        .expect("RSS migrations should apply");
    insert_user(&database, USER_A_ID, "reader-a").await;
    insert_user(&database, USER_B_ID, "reader-b").await;

    let repository = FeedRepository::new(database.clone());
    let transport = ControlledTransport::new(synthetic_feed(60));
    let service = Arc::new(FeedService::with_jitter(
        repository.clone(),
        FeedUrlPolicy::new(false),
        transport.clone(),
        ZeroJitter,
    ));

    let first_service = Arc::clone(&service);
    let first = tokio::spawn(async move {
        first_service
            .subscribe(
                USER_A_ID,
                SubscribeInput {
                    url: FEED_URL.to_owned(),
                },
            )
            .await
    });
    transport.first_entered.notified().await;

    let second_service = Arc::clone(&service);
    let second = tokio::spawn(async move {
        second_service
            .subscribe(
                USER_B_ID,
                SubscribeInput {
                    url: FEED_URL.to_owned(),
                },
            )
            .await
    });
    wait_for_subscription_count(&database, 2).await;
    let boundaries = subscription::Entity::find()
        .order_by_asc(subscription::Column::UserId)
        .all(&database)
        .await
        .expect("subscription boundaries should query");
    assert_eq!(boundaries.len(), 2);
    assert!(
        boundaries
            .iter()
            .all(|row| row.start_sequence == 0 && row.read_through_sequence == 0),
        "both subscriptions must commit before the first feed persistence"
    );

    transport.first_release.notify_one();
    transport.second_entered.notified().await;
    wait_for_entry_count(&database, 60).await;
    let before_second_200 = entry::Entity::find()
        .order_by_asc(entry::Column::Id)
        .all(&database)
        .await
        .expect("first persistence should be inspectable");
    transport.second_release.notify_one();

    let first = first
        .await
        .expect("first subscribe task should join")
        .expect("first subscribe should succeed");
    let second = second
        .await
        .expect("second subscribe task should join")
        .expect("second subscribe should succeed");

    assert_eq!(first.refresh.status.as_str(), "SUCCESS");
    assert_eq!(first.refresh.new_count, 60);
    assert_eq!(second.refresh.status.as_str(), "SUCCESS");
    assert_eq!(second.refresh.new_count, 0);
    assert_eq!(second.refresh.updated_count, 0);
    assert_eq!(transport.calls.load(Ordering::SeqCst), 2);
    assert_eq!(feed::Entity::find().count(&database).await.unwrap(), 1);
    assert_eq!(
        subscription::Entity::find().count(&database).await.unwrap(),
        2
    );
    assert_eq!(entry::Entity::find().count(&database).await.unwrap(), 60);
    let persisted_feed = feed::Entity::find()
        .one(&database)
        .await
        .expect("feed schedule should query")
        .expect("shared feed should exist");
    assert_eq!(persisted_feed.consecutive_failures, 0);
    assert!(persisted_feed.retry_after_at.is_none());
    assert!(persisted_feed.last_error_code.is_none());
    assert!(
        persisted_feed
            .last_success_at
            .is_some_and(|last_success| persisted_feed.next_fetch_at > last_success),
        "successful persistence must atomically advance the next fetch time"
    );
    for index in 0..200_u128 {
        let user_id =
            Uuid::from_u128(0x1000_0000_0000_4000_8000_0000_0000_0000 + index).to_string();
        insert_user(&database, &user_id, &format!("noise-{index:03}")).await;
        subscription::ActiveModel {
            id: Set(Uuid::new_v4().to_string()),
            user_id: Set(user_id),
            feed_id: Set(first.feed_id.clone()),
            title_override: Set(None),
            position: Set(0),
            start_sequence: Set(60),
            read_through_sequence: Set(60),
            state_revision: Set(0),
            created_at: Set(time::OffsetDateTime::now_utc()),
            updated_at: Set(time::OffsetDateTime::now_utc()),
        }
        .insert(&database)
        .await
        .expect("noise subscription should insert");
    }
    database
        .execute_unprepared("ANALYZE")
        .await
        .expect("SQLite statistics should collect");
    for state in [EntryListState::All, EntryListState::Unread] {
        for feed_id in [None, Some(first.feed_id.clone())] {
            let plan = repository
                .explain_list_for_user(
                    USER_A_ID,
                    ListEntriesQuery {
                        state,
                        feed_id,
                        ..ListEntriesQuery::default()
                    },
                )
                .await
                .expect("SQLite list EXPLAIN should execute");
            let joined = plan.join("\n");
            assert!(
                joined.contains("subscriptions") && joined.contains("INDEX"),
                "subscription-first lookup must use a user-leading index: {joined}"
            );
            assert!(
                joined.contains("idx_entries_feed_list")
                    || joined.contains("uq_entries_feed_seq")
                    || joined.contains("idx_entries_snapshot"),
                "entry lookup must use an existing feed-leading index: {joined}"
            );
            assert!(
                !joined.contains("SCAN e"),
                "authenticated list must not full-scan entries: {joined}"
            );
        }
    }

    let after_second_200 = entry::Entity::find()
        .order_by_asc(entry::Column::Id)
        .all(&database)
        .await
        .expect("second persistence should be inspectable");
    assert_eq!(after_second_200.len(), before_second_200.len());
    for (before, after) in before_second_200.iter().zip(&after_second_200) {
        assert_eq!(after.id, before.id, "dedup must preserve entry IDs");
        assert_eq!(
            after.inserted_at, before.inserted_at,
            "dedup must preserve insertion timestamps"
        );
    }

    let page_a = repository
        .list_for_user(USER_A_ID, ListEntriesQuery::default())
        .await
        .expect("reader A should list entries");
    let page_b = repository
        .list_for_user(USER_B_ID, ListEntriesQuery::default())
        .await
        .expect("reader B should list entries");
    assert_eq!(page_a.items.len(), 50);
    assert_eq!(page_b.items.len(), 50);
    assert_eq!(page_a.items[0].entry_id, page_b.items[0].entry_id);
    assert!(page_a.items.iter().all(|item| !item.is_read));
    assert!(page_a.items.iter().all(|item| !item.is_starred));

    let cursor = page_a
        .next_cursor
        .clone()
        .expect("60 entries need page two");
    let page_two = repository
        .list_for_user(
            USER_A_ID,
            ListEntriesQuery {
                cursor: Some(cursor.clone()),
                ..ListEntriesQuery::default()
            },
        )
        .await
        .expect("reader A should use its cursor");
    assert_eq!(page_two.items.len(), 10);
    assert!(page_two.next_cursor.is_none());
    assert!(
        repository
            .list_for_user(
                USER_B_ID,
                ListEntriesQuery {
                    cursor: Some(cursor),
                    ..ListEntriesQuery::default()
                },
            )
            .await
            .is_err(),
        "cursor reuse across users must be rejected"
    );
    assert!(
        repository
            .list_for_user(
                USER_A_ID,
                ListEntriesQuery {
                    cursor: Some("eyJ2IjoxfQ==".to_owned()),
                    ..ListEntriesQuery::default()
                },
            )
            .await
            .is_err(),
        "padded or incomplete cursors must be rejected"
    );

    let entry_id = &page_a.items[0].entry_id;
    let detail_a = repository
        .get_detail_for_user(USER_A_ID, entry_id)
        .await
        .expect("reader A detail query should succeed")
        .expect("reader A should see the shared entry");
    let detail_b = repository
        .get_detail_for_user(USER_B_ID, entry_id)
        .await
        .expect("reader B detail query should succeed")
        .expect("reader B should see the shared entry");
    assert_eq!(detail_a.entry_id, detail_b.entry_id);
    assert_secure_detail(&detail_a.content_html);
    assert_eq!(detail_a.inert_images.len(), 1);
    assert_eq!(
        detail_a.inert_images[0].source_url,
        "https://images.example.test/hero-060.jpg"
    );
    assert!(
        repository
            .get_detail_for_user("00000000-0000-4000-8000-000000000099", entry_id)
            .await
            .expect("opaque ID lookup should remain typed")
            .is_none(),
        "guessing an entry ID must not authorize access"
    );

    let visible_entry = entry::Entity::find_by_id(entry_id)
        .one(&database)
        .await
        .expect("visible entry should query")
        .expect("visible entry should exist");
    subscription::Entity::update_many()
        .col_expr(subscription::Column::ReadThroughSequence, 60_i64.into())
        .filter(subscription::Column::UserId.eq(USER_A_ID))
        .exec(&database)
        .await
        .expect("test should advance reader A's read frontier");
    entry_state::ActiveModel {
        user_id: Set(USER_A_ID.to_owned()),
        entry_id: Set(visible_entry.id.clone()),
        feed_id: Set(visible_entry.feed_id.clone()),
        feed_sequence: Set(visible_entry.feed_sequence),
        read_override: Set(Some(false)),
        is_starred: Set(true),
        starred_at: Set(None),
        revision: Set(1),
        updated_at: Set(time::OffsetDateTime::now_utc()),
    }
    .insert(&database)
    .await
    .expect("sparse reader state should insert");
    let unread_override = repository
        .list_for_user(USER_A_ID, ListEntriesQuery::default())
        .await
        .expect("explicit unread override should query");
    assert_eq!(unread_override.items.len(), 1);
    assert_eq!(unread_override.items[0].entry_id, visible_entry.id);
    assert!(!unread_override.items[0].is_read);
    assert!(unread_override.items[0].is_starred);
    let starred = repository
        .list_for_user(
            USER_A_ID,
            ListEntriesQuery {
                state: EntryListState::Starred,
                ..ListEntriesQuery::default()
            },
        )
        .await
        .expect("starred state should use the sparse state join");
    assert_eq!(starred.items.len(), 1);
    assert_eq!(starred.items[0].entry_id, visible_entry.id);

    subscription::Entity::update_many()
        .col_expr(subscription::Column::StartSequence, 60_i64.into())
        .col_expr(subscription::Column::ReadThroughSequence, 60_i64.into())
        .filter(subscription::Column::UserId.eq(USER_B_ID))
        .exec(&database)
        .await
        .expect("test should move reader B's visibility boundary");
    assert!(
        repository
            .get_detail_for_user(USER_B_ID, entry_id)
            .await
            .expect("bounded detail lookup should remain typed")
            .is_none(),
        "entries at or before subscription start must stay invisible"
    );
}

#[tokio::test]
async fn exact_run_claim_never_consumes_an_older_unrelated_run() {
    let data = tempfile::tempdir().expect("temporary directory should be created");
    let database_url = format!(
        "sqlite://{}?mode=rwc",
        data.path().join("exact-run-claim.db").display()
    );
    let database = connect_for_contract(SecretString::from(database_url)).await;
    migrate(&database).await.expect("migrations should apply");
    insert_feed(&database, time::OffsetDateTime::now_utc()).await;
    let model = feed::Entity::find_by_id(FEED_ID)
        .one(&database)
        .await
        .expect("feed should query")
        .expect("feed should exist");
    let mut active: feed::ActiveModel = model.into();
    active.lease_owner = Set(None);
    active.lease_until = Set(None);
    active.update(&database).await.expect("feed should unlock");

    let repository = FeedRepository::new(database.clone());
    let older = seed_queued_refresh_run(&database, "older-unrelated").await;
    let target = seed_queued_refresh_run(&database, "target-exact").await;
    let result = repository
        .claim_run(
            &target.id,
            ClaimRequest {
                owner: "exact-worker".to_owned(),
                lease_duration: Duration::from_secs(30),
            },
        )
        .await
        .expect("exact claim should remain typed");
    let ExactClaimResult::Claimed(claim) = result else {
        panic!("target run should be claimed exactly");
    };
    assert_eq!(claim.run_id, target.id);
    let older_status = feed_refresh_run::Entity::find_by_id(older.id)
        .one(&database)
        .await
        .expect("older run should query")
        .expect("older run should exist")
        .status;
    assert_eq!(older_status, "QUEUED");
    repository
        .cancel_running(&claim)
        .await
        .expect("target run should cancel and release its lease");
    assert!(matches!(
        repository
            .claim_run(
                &target.id,
                ClaimRequest {
                    owner: "exact-worker".to_owned(),
                    lease_duration: Duration::from_secs(30),
                },
            )
            .await
            .expect("terminal exact state should query"),
        ExactClaimResult::Existing(raindrop::feeds::RefreshStatus::Cancelled)
    ));
    assert!(matches!(
        repository
            .claim_run(
                "00000000-0000-4000-8000-000000000999",
                ClaimRequest {
                    owner: "exact-worker".to_owned(),
                    lease_duration: Duration::from_secs(30),
                },
            )
            .await,
        Err(RefreshRepositoryError::RunNotFound)
    ));

    let disabled_run = seed_queued_refresh_run(&database, "disabled-exact").await;
    feed::Entity::update_many()
        .col_expr(feed::Column::IsDisabled, true.into())
        .filter(feed::Column::Id.eq(FEED_ID))
        .exec(&database)
        .await
        .expect("feed should disable");
    assert!(matches!(
        repository
            .claim_run(
                &disabled_run.id,
                ClaimRequest {
                    owner: "exact-worker".to_owned(),
                    lease_duration: Duration::from_secs(30),
                },
            )
            .await
            .expect("disabled exact state should query"),
        ExactClaimResult::FeedDisabled
    ));
}

#[tokio::test]
async fn redirected_304_never_rebinds_old_validator_bytes() {
    let data = tempfile::tempdir().expect("temporary directory should be created");
    let database_url = format!(
        "sqlite://{}?mode=rwc",
        data.path().join("redirected-304.db").display()
    );
    let database = connect_for_contract(SecretString::from(database_url)).await;
    migrate(&database).await.expect("migrations should apply");
    insert_feed(&database, time::OffsetDateTime::now_utc()).await;
    let model = feed::Entity::find_by_id(FEED_ID)
        .one(&database)
        .await
        .expect("feed should query")
        .expect("feed should exist");
    let mut active: feed::ActiveModel = model.into();
    active.lease_owner = Set(None);
    active.lease_until = Set(None);
    active.orphaned_at = Set(None);
    active.update(&database).await.expect("feed should unlock");
    let repository = FeedRepository::new(database.clone());
    let run = repository
        .queue_refresh(QueueRefreshRequest {
            feed_id: FEED_ID.to_owned(),
            requested_by_user_id: None,
            trigger: RefreshTrigger::Manual,
            idempotency_key: "redirected-304".to_owned(),
        })
        .await
        .expect("304 run should queue");
    let ExactClaimResult::Claimed(claim) = repository
        .claim_run(
            &run.id,
            ClaimRequest {
                owner: "validator-worker".to_owned(),
                lease_duration: Duration::from_secs(30),
            },
        )
        .await
        .expect("304 run should claim")
    else {
        panic!("304 run should claim exactly");
    };
    let redirected = FeedUrlPolicy::new(false)
        .normalize("https://redirected.example.test/feed.xml")
        .expect("redirect URL should normalize");
    let schedule = RefreshSchedule::new(ZeroJitter)
        .after_result(
            time::OffsetDateTime::now_utc(),
            0,
            RefreshResult::NotModified,
        )
        .expect("304 schedule should compute");
    repository
        .complete_not_modified_scheduled(&claim, &redirected, None, None, schedule)
        .await
        .expect("304 should commit atomically");

    let feed = feed::Entity::find_by_id(FEED_ID)
        .one(&database)
        .await
        .expect("feed should query")
        .expect("feed should exist");
    assert_eq!(feed.fetch_url, "https://redirected.example.test/feed.xml");
    assert_eq!(
        feed.validator_url.as_deref(),
        Some("https://redirected.example.test/feed.xml")
    );
    assert!(feed.etag.is_none(), "old URL ETag must be cleared");
    assert!(
        feed.last_modified.is_none(),
        "old URL Last-Modified must be cleared"
    );
    assert_eq!(feed.consecutive_failures, 0);
    assert!(feed.lease_owner.is_none());
    let completed = feed_refresh_run::Entity::find_by_id(run.id)
        .one(&database)
        .await
        .expect("run should query")
        .expect("run should exist");
    assert_eq!(completed.status, "NOT_MODIFIED");
    assert_eq!(completed.http_status, Some(304));
    assert!(completed.fetched_at.is_some());
    assert_eq!(
        lifecycle_outbox::Entity::find()
            .count(&database)
            .await
            .expect("outbox should count"),
        1
    );
}

#[tokio::test]
async fn scheduled_terminal_paths_roll_back_feed_run_entries_and_outbox_together() {
    let (_data, database, repository, claim) = atomic_claim("scheduled-200-rollback").await;
    insert_conflicting_completed_event(&database, &claim).await;
    let before_feed = feed::Entity::find_by_id(FEED_ID)
        .one(&database)
        .await
        .unwrap()
        .unwrap();
    let before_run = feed_refresh_run::Entity::find_by_id(&claim.run_id)
        .one(&database)
        .await
        .unwrap()
        .unwrap();
    let parsed = FeedParser::new()
        .parse(
            FetchedDocument::try_from(FetchOutcome::Document {
                url: FeedUrlPolicy::new(false)
                    .normalize("https://example.com/feed.xml")
                    .unwrap(),
                document: synthetic_feed(1),
                content_type: Some("application/rss+xml".to_owned()),
                etag: None,
                last_modified: None,
            })
            .unwrap(),
        )
        .await
        .expect("rollback feed should parse");
    let schedule = success_schedule();
    assert!(matches!(
        repository
            .persist_feed_scheduled(&claim, PersistFeed::try_from(parsed).unwrap(), schedule)
            .await,
        Err(RefreshRepositoryError::LifecycleEventConflict)
    ));
    assert_atomic_snapshot(&database, &before_feed, &before_run, 1).await;

    let (_data, database, repository, claim) = atomic_claim("scheduled-304-rollback").await;
    insert_conflicting_completed_event(&database, &claim).await;
    let before_feed = feed::Entity::find_by_id(FEED_ID)
        .one(&database)
        .await
        .unwrap()
        .unwrap();
    let before_run = feed_refresh_run::Entity::find_by_id(&claim.run_id)
        .one(&database)
        .await
        .unwrap()
        .unwrap();
    let final_url = FeedUrlPolicy::new(false)
        .normalize("https://redirected.example.test/feed.xml")
        .unwrap();
    assert!(matches!(
        repository
            .complete_not_modified_scheduled(&claim, &final_url, None, None, success_schedule())
            .await,
        Err(RefreshRepositoryError::LifecycleEventConflict)
    ));
    assert_atomic_snapshot(&database, &before_feed, &before_run, 1).await;

    let (_data, database, repository, claim) = atomic_claim("scheduled-error-rollback").await;
    insert_conflicting_completed_event(&database, &claim).await;
    let before_feed = feed::Entity::find_by_id(FEED_ID)
        .one(&database)
        .await
        .unwrap()
        .unwrap();
    let before_run = feed_refresh_run::Entity::find_by_id(&claim.run_id)
        .one(&database)
        .await
        .unwrap()
        .unwrap();
    let failure_schedule = RefreshSchedule::new(ZeroJitter)
        .after_result(
            time::OffsetDateTime::now_utc(),
            0,
            RefreshResult::TransientFailure { retry_after: None },
        )
        .unwrap();
    assert!(matches!(
        repository
            .complete_failure_scheduled(
                &claim,
                RefreshFailure {
                    error_code: "FETCH_FAILED".to_owned(),
                    http_status: None,
                    retry_at: Some(failure_schedule.next_at()),
                },
                failure_schedule,
            )
            .await,
        Err(RefreshRepositoryError::LifecycleEventConflict)
    ));
    assert_atomic_snapshot(&database, &before_feed, &before_run, 1).await;
}

fn success_schedule() -> raindrop::feeds::ScheduleOutcome {
    RefreshSchedule::new(ZeroJitter)
        .after_result(time::OffsetDateTime::now_utc(), 0, RefreshResult::Success)
        .unwrap()
}

async fn atomic_claim(
    name: &str,
) -> (
    tempfile::TempDir,
    DatabaseConnection,
    FeedRepository,
    raindrop::feeds::RefreshClaim,
) {
    let data = tempfile::tempdir().expect("temporary directory should be created");
    let database_url = format!(
        "sqlite://{}?mode=rwc",
        data.path().join(format!("{name}.db")).display()
    );
    let database = connect_for_contract(SecretString::from(database_url)).await;
    migrate(&database).await.unwrap();
    insert_feed(&database, time::OffsetDateTime::now_utc()).await;
    let model = feed::Entity::find_by_id(FEED_ID)
        .one(&database)
        .await
        .unwrap()
        .unwrap();
    let mut active: feed::ActiveModel = model.into();
    active.entry_sequence_head = Set(0);
    active.lease_owner = Set(None);
    active.lease_until = Set(None);
    active.orphaned_at = Set(None);
    active.update(&database).await.unwrap();
    let repository = FeedRepository::new(database.clone());
    let run = repository
        .queue_refresh(QueueRefreshRequest {
            feed_id: FEED_ID.to_owned(),
            requested_by_user_id: None,
            trigger: RefreshTrigger::Manual,
            idempotency_key: name.to_owned(),
        })
        .await
        .unwrap();
    let ExactClaimResult::Claimed(claim) = repository
        .claim_run(
            &run.id,
            ClaimRequest {
                owner: format!("worker-{name}"),
                lease_duration: Duration::from_secs(30),
            },
        )
        .await
        .unwrap()
    else {
        panic!("atomic run should claim");
    };
    (data, database, repository, claim)
}

async fn insert_conflicting_completed_event(
    database: &DatabaseConnection,
    claim: &raindrop::feeds::RefreshClaim,
) {
    let now = time::OffsetDateTime::now_utc();
    lifecycle_outbox::ActiveModel {
        id: Set(Uuid::new_v4().to_string()),
        event_type: Set("feed.refresh.completed".to_owned()),
        aggregate_type: Set("FEED".to_owned()),
        aggregate_id: Set(claim.feed_id.clone()),
        refresh_id: Set(claim.run_id.clone()),
        event_sequence: Set(20),
        payload_version: Set(1),
        payload_json: Set("{\"conflict\":true}".to_owned()),
        idempotency_key: Set(format!("refresh:{}:completed:v1", claim.run_id)),
        status: Set("PENDING".to_owned()),
        available_at: Set(now),
        attempts: Set(0),
        lease_owner: Set(None),
        lease_until: Set(None),
        created_at: Set(now),
        completed_at: Set(None),
    }
    .insert(database)
    .await
    .expect("conflicting event should insert");
}

async fn seed_queued_refresh_run(
    database: &DatabaseConnection,
    idempotency_key: &str,
) -> feed_refresh_run::Model {
    feed_refresh_run::ActiveModel {
        id: Set(Uuid::new_v4().to_string()),
        feed_id: Set(FEED_ID.to_owned()),
        requested_by_user_id: Set(None),
        trigger_kind: Set(RefreshTrigger::Manual.as_str().to_owned()),
        status: Set("QUEUED".to_owned()),
        idempotency_key: Set(idempotency_key.to_owned()),
        lease_token: Set(None),
        commit_generation: Set(None),
        queued_at: Set(time::OffsetDateTime::now_utc()),
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
    .expect("queued refresh fixture should insert")
}

async fn assert_atomic_snapshot(
    database: &DatabaseConnection,
    before_feed: &feed::Model,
    before_run: &feed_refresh_run::Model,
    outbox_count: u64,
) {
    assert_eq!(entry::Entity::find().count(database).await.unwrap(), 0);
    assert_eq!(
        feed::Entity::find_by_id(FEED_ID)
            .one(database)
            .await
            .unwrap()
            .unwrap(),
        *before_feed
    );
    assert_eq!(
        feed_refresh_run::Entity::find_by_id(&before_run.id)
            .one(database)
            .await
            .unwrap()
            .unwrap(),
        *before_run
    );
    assert_eq!(
        lifecycle_outbox::Entity::find()
            .count(database)
            .await
            .unwrap(),
        outbox_count
    );
}

fn assert_secure_detail(html: &str) {
    let lower = html.to_ascii_lowercase();
    for forbidden in [
        "<rss", "<script", "<style", "onclick", "onerror", "<iframe", "<form", "<svg", "class=",
        "data-", "style=", " src=", "srcset=", "poster=",
    ] {
        assert!(
            !lower.contains(forbidden),
            "found unsafe HTML token {forbidden}"
        );
    }
}

fn synthetic_feed(count: usize) -> Vec<u8> {
    let mut body = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?><rss version=\"2.0\"><channel><title>Synthetic Security Feed</title><link>https://news.example.test/</link><description>Generated test data</description>",
    );
    for index in 1..=count {
        body.push_str(&format!(
            "<item><guid isPermaLink=\"false\">synthetic-{index:03}</guid><title>Generated item {index:03}</title><link>https://news.example.test/items/{index:03}</link><author>author-{index:03}@example.test</author><pubDate>Fri, 17 Jul 2026 12:{minute:02}:00 GMT</pubDate><description><![CDATA[<p class=\"publisher\" style=\"color:red\" onclick=\"steal()\">Generated summary {index:03}</p><script>steal()</script><style>body{{display:none}}</style><iframe src=\"https://evil.example/\"></iframe><form action=\"https://evil.example/\"><input></form><svg onload=\"steal()\"></svg><img class=\"tracking\" data-publisher=\"yes\" src=\"https://images.example.test/hero-{index:03}.jpg\" alt=\"Hero {index:03}\" width=\"640\" height=\"480\">]]></description><enclosure url=\"https://media.example.test/audio-{index:03}.mp3\" type=\"audio/mpeg\" length=\"{index}\" /></item>",
            minute = (index - 1) % 60,
        ));
    }
    body.push_str("</channel></rss>");
    body.into_bytes()
}

async fn wait_for_subscription_count(database: &sea_orm::DatabaseConnection, expected: u64) {
    for _ in 0..200 {
        let count = subscription::Entity::find()
            .count(database)
            .await
            .expect("row count should query");
        if count == expected {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("timed out waiting for row count {expected}");
}

async fn wait_for_entry_count(database: &sea_orm::DatabaseConnection, expected: u64) {
    for _ in 0..200 {
        let count = entry::Entity::find()
            .count(database)
            .await
            .expect("row count should query");
        if count == expected {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("timed out waiting for row count {expected}");
}
