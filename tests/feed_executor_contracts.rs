#[allow(dead_code)]
mod support;

use std::sync::{
    Arc, Mutex as StdMutex,
    atomic::{AtomicUsize, Ordering},
};
use std::time::Duration;

use async_trait::async_trait;
use raindrop::db::{entities::feed, migrate};
use raindrop::feeds::{
    ClaimRequest, EntryListState, FeedCommandService, FeedExecutor, FeedFetchError, FeedTransport,
    FeedUrlPolicy, FetchOutcome, FetchRequest, HttpFeedTransport, JitterSource, ListEntriesQuery,
    QueueSubscriptionRefresh, RefreshStatus, SubscribeInput,
};
use sea_orm::{ActiveModelTrait, ActiveValue::Set, EntityTrait};
use secrecy::SecretString;
use tokio::sync::Notify;

use support::database::{
    FEED_ID, SUBSCRIPTION_A_ID, USER_A_ID, connect_for_contract, insert_feed, insert_subscription,
    insert_user,
};

const FEED_URL: &str = "https://feeds.example.test/command-only.xml";

#[derive(Clone)]
struct BlockedTransport {
    calls: Arc<AtomicUsize>,
    entered: Arc<Notify>,
    release: Arc<Notify>,
}

impl BlockedTransport {
    fn new() -> Self {
        Self {
            calls: Arc::new(AtomicUsize::new(0)),
            entered: Arc::new(Notify::new()),
            release: Arc::new(Notify::new()),
        }
    }
}

#[async_trait]
impl FeedTransport for BlockedTransport {
    async fn fetch(&self, request: FetchRequest) -> Result<FetchOutcome, FeedFetchError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.entered.notify_one();
        self.release.notified().await;
        Ok(FetchOutcome::Document {
            url: request.url().clone(),
            document: Vec::new(),
            content_type: None,
            etag: None,
            last_modified: None,
        })
    }
}

#[derive(Clone)]
struct StaticTransport {
    calls: Arc<AtomicUsize>,
    body: Arc<Vec<u8>>,
}

#[derive(Clone)]
struct NotModifiedTransport {
    calls: Arc<AtomicUsize>,
}

struct FailureTransport {
    error: StdMutex<Option<FeedFetchError>>,
}

#[async_trait]
impl FeedTransport for FailureTransport {
    async fn fetch(&self, _request: FetchRequest) -> Result<FetchOutcome, FeedFetchError> {
        Err(self
            .error
            .lock()
            .expect("failure transport lock should remain healthy")
            .take()
            .expect("failure transport must execute once"))
    }
}

#[async_trait]
impl FeedTransport for NotModifiedTransport {
    async fn fetch(&self, request: FetchRequest) -> Result<FetchOutcome, FeedFetchError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(FetchOutcome::NotModified {
            url: request.url().clone(),
            etag: None,
            last_modified: None,
        })
    }
}

#[async_trait]
impl FeedTransport for StaticTransport {
    async fn fetch(&self, request: FetchRequest) -> Result<FetchOutcome, FeedFetchError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(FetchOutcome::Document {
            url: request.url().clone(),
            document: self.body.as_ref().clone(),
            content_type: Some("application/rss+xml; charset=utf-8".to_owned()),
            etag: None,
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
async fn command_service_never_calls_transport() {
    let data = tempfile::tempdir().expect("temporary directory should be created");
    let database_url = format!(
        "sqlite://{}?mode=rwc",
        data.path().join("command-only.db").display()
    );
    let database = connect_for_contract(SecretString::from(database_url)).await;
    migrate(&database).await.expect("migrations should apply");
    insert_user(&database, USER_A_ID, "command-reader").await;

    let repository = raindrop::feeds::FeedRepository::new(database);
    let command = FeedCommandService::new(repository, FeedUrlPolicy::new(false));
    let transport = BlockedTransport::new();

    let outcome = tokio::time::timeout(
        Duration::from_secs(1),
        command.subscribe(
            USER_A_ID,
            SubscribeInput {
                url: FEED_URL.to_owned(),
            },
        ),
    )
    .await
    .expect("queue-only subscribe must return without waiting on transport")
    .expect("queue-only subscribe should succeed");

    assert!(outcome.created);
    assert_eq!(transport.calls.load(Ordering::SeqCst), 0);
    assert!(
        tokio::time::timeout(Duration::from_millis(20), transport.entered.notified())
            .await
            .is_err(),
        "queue-only subscribe must not enter the transport"
    );
}

#[tokio::test]
async fn executor_success_persists_and_returns_refresh() {
    let data = tempfile::tempdir().expect("temporary directory should be created");
    let database_url = format!(
        "sqlite://{}?mode=rwc",
        data.path().join("executor-success.db").display()
    );
    let database = connect_for_contract(SecretString::from(database_url)).await;
    migrate(&database).await.expect("migrations should apply");
    insert_user(&database, USER_A_ID, "executor-reader").await;

    let repository = raindrop::feeds::FeedRepository::new(database.clone());
    let policy = FeedUrlPolicy::new(false);
    let command = FeedCommandService::new(repository.clone(), policy);
    let transport = StaticTransport {
        calls: Arc::new(AtomicUsize::new(0)),
        body: Arc::new(one_item_feed()),
    };
    let executor =
        FeedExecutor::with_jitter(repository.clone(), policy, transport.clone(), ZeroJitter);

    let queued = command
        .subscribe(
            USER_A_ID,
            SubscribeInput {
                url: FEED_URL.to_owned(),
            },
        )
        .await
        .expect("subscribe command should queue");
    assert_eq!(
        queued
            .subscription
            .refresh
            .as_ref()
            .expect("new feed should have a queued refresh")
            .status,
        RefreshStatus::Queued
    );
    let claim = repository
        .claim_due(ClaimRequest {
            owner: "executor-success".to_owned(),
            lease_duration: Duration::from_secs(30),
        })
        .await
        .expect("queued refresh should remain claimable")
        .expect("queued refresh should be due");

    let refresh = executor
        .execute_claim(claim)
        .await
        .expect("claimed refresh should execute");

    assert_eq!(refresh.status, RefreshStatus::Success);
    assert_eq!(refresh.new_count, 1);
    assert_eq!(transport.calls.load(Ordering::SeqCst), 1);
    let page = repository
        .list_for_user(
            USER_A_ID,
            ListEntriesQuery {
                state: EntryListState::All,
                ..ListEntriesQuery::default()
            },
        )
        .await
        .expect("persisted entry should be user-visible");
    assert_eq!(page.items.len(), 1);
    assert_eq!(
        raindrop::db::entities::entry::Entity::find()
            .all(&database)
            .await
            .expect("entry rows should query")
            .len(),
        1
    );
}

#[tokio::test]
async fn executor_not_modified_never_parses_body() {
    let data = tempfile::tempdir().expect("temporary directory should be created");
    let database_url = format!(
        "sqlite://{}?mode=rwc",
        data.path().join("executor-not-modified.db").display()
    );
    let database = connect_for_contract(SecretString::from(database_url)).await;
    migrate(&database).await.expect("migrations should apply");
    insert_user(&database, USER_A_ID, "not-modified-reader").await;

    let repository = raindrop::feeds::FeedRepository::new(database.clone());
    let policy = FeedUrlPolicy::new(false);
    let command = FeedCommandService::new(repository.clone(), policy);
    command
        .subscribe(
            USER_A_ID,
            SubscribeInput {
                url: FEED_URL.to_owned(),
            },
        )
        .await
        .expect("not-modified feed should queue");
    let claim = repository
        .claim_due(ClaimRequest {
            owner: "executor-not-modified".to_owned(),
            lease_duration: Duration::from_secs(30),
        })
        .await
        .expect("not-modified refresh should remain claimable")
        .expect("not-modified refresh should be due");
    let transport = NotModifiedTransport {
        calls: Arc::new(AtomicUsize::new(0)),
    };
    let executor = FeedExecutor::with_jitter(repository, policy, transport.clone(), ZeroJitter);

    let refresh = executor
        .execute_claim(claim)
        .await
        .expect("not-modified refresh should complete");

    assert_eq!(refresh.status, RefreshStatus::NotModified);
    assert_eq!(refresh.new_count, 0);
    assert_eq!(transport.calls.load(Ordering::SeqCst), 1);
    assert!(
        raindrop::db::entities::entry::Entity::find()
            .all(&database)
            .await
            .expect("entries should query")
            .is_empty()
    );
}

#[tokio::test]
async fn executor_fetch_parse_content_failures_end_with_stable_internal_codes() {
    let fetch_error = match HttpFeedTransport::new_observed(FeedUrlPolicy::new(false), 0) {
        Ok(_) => panic!("zero request budget must be rejected"),
        Err(error) => error,
    };
    assert_failure_code(
        "fetch",
        FailureTransport {
            error: StdMutex::new(Some(fetch_error)),
        },
        "FETCH_FAILED",
    )
    .await;
    assert_failure_code(
        "parse",
        StaticTransport {
            calls: Arc::new(AtomicUsize::new(0)),
            body: Arc::new(b"not a feed".to_vec()),
        },
        "PARSE_FAILED",
    )
    .await;
    assert_failure_code(
        "content",
        StaticTransport {
            calls: Arc::new(AtomicUsize::new(0)),
            body: Arc::new(vec![b'x'; 10 * 1024 * 1024 + 1]),
        },
        "DOCUMENT_REJECTED",
    )
    .await;
}

#[tokio::test]
async fn executor_rejects_claim_feed_mismatch_without_network() {
    let data = tempfile::tempdir().expect("temporary directory should be created");
    let database_url = format!(
        "sqlite://{}?mode=rwc",
        data.path().join("executor-mismatch.db").display()
    );
    let database = connect_for_contract(SecretString::from(database_url)).await;
    migrate(&database).await.expect("migrations should apply");
    insert_user(&database, USER_A_ID, "mismatch-reader").await;

    let repository = raindrop::feeds::FeedRepository::new(database);
    let policy = FeedUrlPolicy::new(false);
    let command = FeedCommandService::new(repository.clone(), policy);
    command
        .subscribe(
            USER_A_ID,
            SubscribeInput {
                url: "https://feeds.example.test/claim-source.xml".to_owned(),
            },
        )
        .await
        .expect("source feed should queue");
    let mut claim = repository
        .claim_due(ClaimRequest {
            owner: "executor-mismatch".to_owned(),
            lease_duration: Duration::from_secs(30),
        })
        .await
        .expect("source refresh should remain claimable")
        .expect("source refresh should be due");
    let other = command
        .subscribe(
            USER_A_ID,
            SubscribeInput {
                url: "https://feeds.example.test/other-feed.xml".to_owned(),
            },
        )
        .await
        .expect("other feed should queue");
    claim.feed_id = other.subscription.feed_id;

    let transport = BlockedTransport::new();
    let executor = FeedExecutor::with_jitter(repository, policy, transport.clone(), ZeroJitter);
    let result = tokio::time::timeout(Duration::from_millis(100), executor.execute_claim(claim))
        .await
        .expect("mismatched claim must be rejected before transport");

    assert!(matches!(
        result,
        Err(raindrop::feeds::FeedServiceError::RunMismatch)
    ));
    assert_eq!(transport.calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn command_subscribe_and_manual_refresh_only_queue() {
    let data = tempfile::tempdir().expect("temporary directory should be created");
    let database_url = format!(
        "sqlite://{}?mode=rwc",
        data.path().join("command-queue-only.db").display()
    );
    let database = connect_for_contract(SecretString::from(database_url)).await;
    migrate(&database).await.expect("migrations should apply");
    insert_user(&database, USER_A_ID, "queue-reader").await;
    let fixture_at = time::OffsetDateTime::now_utc() - time::Duration::hours(1);
    insert_feed(&database, fixture_at).await;
    let model = feed::Entity::find_by_id(FEED_ID)
        .one(&database)
        .await
        .expect("manual feed should query")
        .expect("manual feed should exist");
    let mut active: feed::ActiveModel = model.into();
    active.last_attempt_at = Set(None);
    active.retry_after_at = Set(None);
    active.orphaned_at = Set(None);
    active.lease_owner = Set(None);
    active.lease_until = Set(None);
    active
        .update(&database)
        .await
        .expect("manual feed should unlock");
    insert_subscription(&database, SUBSCRIPTION_A_ID, USER_A_ID, fixture_at).await;

    let repository = raindrop::feeds::FeedRepository::new(database);
    let command = FeedCommandService::new(repository, FeedUrlPolicy::new(false));
    let transport = BlockedTransport::new();

    let subscribed = command
        .subscribe(
            USER_A_ID,
            SubscribeInput {
                url: FEED_URL.to_owned(),
            },
        )
        .await
        .expect("subscribe command should queue");
    let manual = command
        .queue_subscription_refresh(
            USER_A_ID,
            SUBSCRIPTION_A_ID,
            QueueSubscriptionRefresh {
                request_id: "00000000-0000-4000-8000-000000000501".to_owned(),
            },
        )
        .await
        .expect("manual command should queue");

    assert_eq!(
        subscribed
            .subscription
            .refresh
            .expect("new subscription should reference queued work")
            .status,
        RefreshStatus::Queued
    );
    assert_eq!(manual.status, RefreshStatus::Queued);
    assert_eq!(transport.calls.load(Ordering::SeqCst), 0);
}

fn one_item_feed() -> Vec<u8> {
    br#"<?xml version="1.0" encoding="UTF-8"?>
<rss version="2.0"><channel>
<title>Executor Feed</title><link>https://news.example.test/</link><description>Executor</description>
<item><guid isPermaLink="false">executor-001</guid><title>Executor item</title>
<link>https://news.example.test/items/001</link>
<pubDate>Fri, 17 Jul 2026 12:00:00 GMT</pubDate><description>Executor body</description></item>
</channel></rss>"#
        .to_vec()
}

async fn assert_failure_code<T>(name: &str, transport: T, expected_code: &str)
where
    T: FeedTransport,
{
    let data = tempfile::tempdir().expect("temporary directory should be created");
    let database_url = format!(
        "sqlite://{}?mode=rwc",
        data.path()
            .join(format!("executor-{name}-failure.db"))
            .display()
    );
    let database = connect_for_contract(SecretString::from(database_url)).await;
    migrate(&database).await.expect("migrations should apply");
    insert_user(&database, USER_A_ID, &format!("{name}-failure-reader")).await;
    let repository = raindrop::feeds::FeedRepository::new(database);
    let policy = FeedUrlPolicy::new(false);
    let command = FeedCommandService::new(repository.clone(), policy);
    command
        .subscribe(
            USER_A_ID,
            SubscribeInput {
                url: format!("https://feeds.example.test/{name}-failure.xml"),
            },
        )
        .await
        .expect("failure feed should queue");
    let claim = repository
        .claim_due(ClaimRequest {
            owner: format!("executor-{name}-failure"),
            lease_duration: Duration::from_secs(30),
        })
        .await
        .expect("failure refresh should remain claimable")
        .expect("failure refresh should be due");
    let executor = FeedExecutor::with_jitter(repository, policy, transport, ZeroJitter);

    let refresh = executor
        .execute_claim(claim)
        .await
        .expect("operational failure should complete terminally");

    assert_eq!(refresh.status, RefreshStatus::Error);
    assert_eq!(refresh.error_code.as_deref(), Some(expected_code));
}
