#[allow(dead_code)]
mod support;

use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use async_trait::async_trait;
use raindrop::{
    app::AppState,
    db::{
        entities::{feed, feed_refresh_run, subscription},
        migrate, rollback,
    },
    feeds::{
        FeedCommandService, FeedExecutor, FeedFetchError, FeedRepository, FeedRuntime,
        FeedTransport, FeedUrlPolicy, FetchOutcome, FetchRequest, JitterSource,
        QueueRefreshRequest, QueueSubscriptionRefresh, RefreshStatus, RefreshTrigger,
    },
    setup::{SetupCompleteInput, SetupService},
};
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ConnectionTrait, DatabaseBackend, EntityTrait, Statement,
    TransactionTrait,
};
use secrecy::SecretString;
use support::database::{
    FEED_ID, SUBSCRIPTION_A_ID, USER_A_ID, connect_for_contract, insert_feed, insert_subscription,
    insert_user, subscription_model,
};
use time::{OffsetDateTime, macros::datetime};
use tokio::sync::{Notify, Semaphore};

const EXPIRED_RUN_ID: &str = "00000000-0000-4000-8000-000000000601";
const SECOND_FEED_ID: &str = "00000000-0000-4000-8000-000000000102";
const SECOND_SUBSCRIPTION_ID: &str = "00000000-0000-4000-8000-000000000203";
const FIRST_RUNTIME_RUN_ID: &str = "00000000-0000-4000-8000-000000000611";
const SECOND_RUNTIME_RUN_ID: &str = "00000000-0000-4000-8000-000000000612";
const CLOSED_NOTIFY_RUN_ID: &str = "00000000-0000-4000-8000-000000000613";

struct NeverTransport;

#[derive(Clone)]
struct NotModifiedTransport {
    calls: Arc<AtomicUsize>,
}

#[derive(Clone)]
struct BlockedNotModifiedTransport {
    calls: Arc<AtomicUsize>,
    entered: Arc<Semaphore>,
    release: Arc<Semaphore>,
}

#[derive(Clone)]
struct CancellationTransport {
    calls: Arc<AtomicUsize>,
    entered: Arc<Semaphore>,
    cancelled: Arc<Semaphore>,
}

struct CancellationGuard {
    cancelled: Arc<Semaphore>,
}

impl Drop for CancellationGuard {
    fn drop(&mut self) {
        self.cancelled.add_permits(1);
    }
}

struct ZeroJitter;

impl JitterSource for ZeroJitter {
    fn sample_inclusive_us(&mut self, _upper_bound_us: u64) -> u64 {
        0
    }
}

#[async_trait]
impl FeedTransport for NeverTransport {
    async fn fetch(&self, _request: FetchRequest) -> Result<FetchOutcome, FeedFetchError> {
        panic!("setup-required runtime must not construct or call transport")
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
impl FeedTransport for BlockedNotModifiedTransport {
    async fn fetch(&self, request: FetchRequest) -> Result<FetchOutcome, FeedFetchError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.entered.add_permits(1);
        self.release
            .acquire()
            .await
            .expect("transport release semaphore should remain open")
            .forget();
        Ok(FetchOutcome::NotModified {
            url: request.url().clone(),
            etag: None,
            last_modified: None,
        })
    }
}

#[async_trait]
impl FeedTransport for CancellationTransport {
    async fn fetch(&self, _request: FetchRequest) -> Result<FetchOutcome, FeedFetchError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let _guard = CancellationGuard {
            cancelled: self.cancelled.clone(),
        };
        self.entered.add_permits(1);
        std::future::pending::<()>().await;
        unreachable!("cancellation transport only exits when its future is dropped")
    }
}

#[tokio::test]
async fn setup_required_runtime_makes_zero_transport_calls() {
    let data = tempfile::tempdir().expect("temporary directory should be created");
    let setup = SetupService::required(data.path(), SecretString::from("setup-token"), None);
    let factory_calls = Arc::new(AtomicUsize::new(0));
    let observed = factory_calls.clone();
    let (runtime, handle) = FeedRuntime::<NeverTransport>::new(setup, move |_database| {
        observed.fetch_add(1, Ordering::SeqCst);
        Err(raindrop::feeds::FeedServiceError::CorruptFeed)
            as Result<Arc<FeedExecutor<NeverTransport>>, _>
    });
    let task = tokio::spawn(runtime.run());

    handle.notify();
    tokio::time::sleep(Duration::from_millis(25)).await;
    handle.shutdown();
    task.await
        .expect("runtime task should join")
        .expect("setup-required runtime should stop cleanly");

    assert_eq!(factory_calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn setup_transition_wakes_runtime_without_process_restart() {
    let data = tempfile::tempdir().expect("temporary directory should be created");
    let database_url = format!(
        "sqlite://{}?mode=rwc",
        data.path().join("setup-transition.db").display()
    );
    let setup = SetupService::required(data.path(), SecretString::from("setup-token"), None);
    let factory_calls = Arc::new(AtomicUsize::new(0));
    let transport_calls = Arc::new(AtomicUsize::new(0));
    let observed_factory = factory_calls.clone();
    let observed_transport = transport_calls.clone();
    let (runtime, handle) = FeedRuntime::new(setup.clone(), move |database| {
        observed_factory.fetch_add(1, Ordering::SeqCst);
        Ok(Arc::new(FeedExecutor::with_jitter(
            FeedRepository::new(database),
            FeedUrlPolicy::new(false),
            NotModifiedTransport {
                calls: observed_transport.clone(),
            },
            ZeroJitter,
        )))
    });
    let task = tokio::spawn(runtime.run());

    tokio::time::sleep(Duration::from_millis(25)).await;
    assert_eq!(factory_calls.load(Ordering::SeqCst), 0);
    setup
        .complete(
            "setup-token",
            SetupCompleteInput {
                database_url: SecretString::from(database_url),
                username: "runtime-admin".to_owned(),
                password: SecretString::from("correct horse battery staple".to_owned()),
                email: None,
            },
        )
        .await
        .expect("setup transition should complete");
    let database = setup
        .database()
        .expect("completed setup should expose its database");
    seed_future_subscribed_feed(&database).await;
    let queued = FeedRepository::new(database.clone())
        .queue_refresh(QueueRefreshRequest {
            feed_id: FEED_ID.to_owned(),
            requested_by_user_id: None,
            trigger: RefreshTrigger::Retry,
            idempotency_key: "runtime:setup-transition".to_owned(),
        })
        .await
        .expect("post-setup refresh should queue");

    handle.notify();
    wait_for_terminal(&database, &queued.id, Duration::from_secs(2)).await;
    handle.shutdown();
    task.await
        .expect("setup-transition runtime task should join")
        .expect("setup-transition runtime should stop cleanly");
    assert_eq!(factory_calls.load(Ordering::SeqCst), 1);
    assert_eq!(transport_calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn main_style_shutdown_stops_lanes_while_server_is_still_draining() {
    let (data, database) = ready_runtime_database("main-style-shutdown").await;
    insert_queued_run(
        &database,
        FIRST_RUNTIME_RUN_ID,
        FEED_ID,
        "runtime:main-style-shutdown",
    )
    .await;
    let transport = BlockedNotModifiedTransport {
        calls: Arc::new(AtomicUsize::new(0)),
        entered: Arc::new(Semaphore::new(0)),
        release: Arc::new(Semaphore::new(0)),
    };
    let setup = SetupService::ready(data.path(), None, database.clone());
    let observed = transport.clone();
    let (runtime, handle) = FeedRuntime::new(setup.clone(), move |database| {
        Ok(Arc::new(FeedExecutor::with_jitter(
            FeedRepository::new(database),
            FeedUrlPolicy::new(false),
            observed.clone(),
            ZeroJitter,
        )))
    });
    let state = AppState::with_feed_runtime(setup, handle);
    let runtime_task = tokio::spawn(runtime.run());
    let server_release = Arc::new(Semaphore::new(0));
    let observed_server_release = server_release.clone();
    let server_task = tokio::spawn(async move {
        observed_server_release
            .acquire()
            .await
            .expect("server drain semaphore should remain open")
            .forget();
    });

    wait_for_entries(&transport.entered, 1, Duration::from_secs(1)).await;
    state.feed_runtime.shutdown();
    transport.release.add_permits(1);
    runtime_task
        .await
        .expect("main-style runtime task should join")
        .expect("main-style runtime should stop cleanly");
    assert!(!server_task.is_finished());

    server_release.add_permits(1);
    server_task.await.expect("server drain task should join");
}

#[tokio::test]
async fn closed_runtime_notify_does_not_change_committed_command_outcome() {
    let (data, database) = ready_runtime_database("closed-runtime-notify").await;
    let state = AppState::new(SetupService::ready(data.path(), None, database.clone()));
    state.feed_runtime.shutdown();
    let command = FeedCommandService::new(
        FeedRepository::new(database.clone()),
        FeedUrlPolicy::new(false),
    );

    let committed = command
        .queue_subscription_refresh(
            USER_A_ID,
            SUBSCRIPTION_A_ID,
            QueueSubscriptionRefresh {
                request_id: CLOSED_NOTIFY_RUN_ID.to_owned(),
            },
        )
        .await
        .expect("command should commit before best-effort notification");
    state.feed_runtime.notify();

    let stored = feed_refresh_run::Entity::find_by_id(&committed.run_id)
        .one(&database)
        .await
        .expect("committed command should remain queryable")
        .expect("committed command should remain persisted");
    assert_eq!(stored.id, committed.run_id);
    assert_eq!(stored.status, RefreshStatus::Queued.as_str());
}

#[tokio::test]
async fn notify_and_poll_both_wake_queued_work() {
    let (data, database) = ready_runtime_database("notify-and-poll").await;
    let repository = FeedRepository::new(database.clone());
    let calls = Arc::new(AtomicUsize::new(0));
    let observed = calls.clone();
    let setup = SetupService::ready(data.path(), None, database.clone());
    let (runtime, handle) = FeedRuntime::new(setup, move |database| {
        Ok(Arc::new(FeedExecutor::with_jitter(
            FeedRepository::new(database),
            FeedUrlPolicy::new(false),
            NotModifiedTransport {
                calls: observed.clone(),
            },
            ZeroJitter,
        )))
    });
    let task = tokio::spawn(runtime.run());

    let notified = repository
        .queue_refresh(QueueRefreshRequest {
            feed_id: FEED_ID.to_owned(),
            requested_by_user_id: None,
            trigger: RefreshTrigger::Retry,
            idempotency_key: "runtime:notify".to_owned(),
        })
        .await
        .expect("notify refresh should queue");
    handle.notify();
    wait_for_terminal(&database, &notified.id, Duration::from_millis(500)).await;
    tokio::time::sleep(Duration::from_millis(50)).await;

    let polled = repository
        .queue_refresh(QueueRefreshRequest {
            feed_id: FEED_ID.to_owned(),
            requested_by_user_id: None,
            trigger: RefreshTrigger::Retry,
            idempotency_key: "runtime:poll".to_owned(),
        })
        .await
        .expect("poll refresh should queue");
    wait_for_terminal(&database, &polled.id, Duration::from_secs(2)).await;

    handle.shutdown();
    task.await
        .expect("runtime task should join")
        .expect("ready runtime should stop cleanly");
    assert_eq!(calls.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn two_lanes_run_different_feeds_concurrently() {
    let (data, database) = ready_runtime_database("different-feeds-concurrent").await;
    seed_second_subscribed_feed(&database).await;
    insert_queued_run(
        &database,
        FIRST_RUNTIME_RUN_ID,
        FEED_ID,
        "runtime:first-feed",
    )
    .await;
    insert_queued_run(
        &database,
        SECOND_RUNTIME_RUN_ID,
        SECOND_FEED_ID,
        "runtime:second-feed",
    )
    .await;
    let transport = BlockedNotModifiedTransport {
        calls: Arc::new(AtomicUsize::new(0)),
        entered: Arc::new(Semaphore::new(0)),
        release: Arc::new(Semaphore::new(0)),
    };
    let setup = SetupService::ready(data.path(), None, database.clone());
    let observed = transport.clone();
    let (runtime, handle) = FeedRuntime::new(setup, move |database| {
        Ok(Arc::new(FeedExecutor::with_jitter(
            FeedRepository::new(database),
            FeedUrlPolicy::new(false),
            observed.clone(),
            ZeroJitter,
        )))
    });
    let task = tokio::spawn(runtime.run());
    handle.notify();

    wait_for_entries(&transport.entered, 2, Duration::from_millis(500)).await;
    assert_eq!(transport.calls.load(Ordering::SeqCst), 2);
    transport.release.add_permits(2);
    wait_for_terminal(&database, FIRST_RUNTIME_RUN_ID, Duration::from_secs(1)).await;
    wait_for_terminal(&database, SECOND_RUNTIME_RUN_ID, Duration::from_secs(1)).await;

    handle.shutdown();
    task.await
        .expect("runtime task should join")
        .expect("concurrent runtime should stop cleanly");
}

#[tokio::test]
async fn two_lanes_never_run_same_feed_concurrently() {
    let (data, database) = ready_runtime_database("same-feed-serial").await;
    insert_queued_run(
        &database,
        FIRST_RUNTIME_RUN_ID,
        FEED_ID,
        "runtime:same-feed-one",
    )
    .await;
    insert_queued_run(
        &database,
        SECOND_RUNTIME_RUN_ID,
        FEED_ID,
        "runtime:same-feed-two",
    )
    .await;
    let transport = BlockedNotModifiedTransport {
        calls: Arc::new(AtomicUsize::new(0)),
        entered: Arc::new(Semaphore::new(0)),
        release: Arc::new(Semaphore::new(0)),
    };
    let setup = SetupService::ready(data.path(), None, database.clone());
    let observed = transport.clone();
    let (runtime, handle) = FeedRuntime::new(setup, move |database| {
        Ok(Arc::new(FeedExecutor::with_jitter(
            FeedRepository::new(database),
            FeedUrlPolicy::new(false),
            observed.clone(),
            ZeroJitter,
        )))
    });
    let task = tokio::spawn(runtime.run());
    handle.notify();

    wait_for_entries(&transport.entered, 1, Duration::from_millis(500)).await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(transport.calls.load(Ordering::SeqCst), 1);
    transport.release.add_permits(1);
    wait_for_entries(&transport.entered, 1, Duration::from_millis(500)).await;
    assert_eq!(transport.calls.load(Ordering::SeqCst), 2);
    transport.release.add_permits(1);
    wait_for_terminal(&database, FIRST_RUNTIME_RUN_ID, Duration::from_secs(1)).await;
    wait_for_terminal(&database, SECOND_RUNTIME_RUN_ID, Duration::from_secs(1)).await;

    handle.shutdown();
    task.await
        .expect("runtime task should join")
        .expect("serial runtime should stop cleanly");
}

#[tokio::test]
async fn heartbeat_extends_lease_before_deadline() {
    let (data, database) = ready_runtime_database("heartbeat-extends").await;
    insert_queued_run(
        &database,
        FIRST_RUNTIME_RUN_ID,
        FEED_ID,
        "runtime:heartbeat",
    )
    .await;
    let transport = BlockedNotModifiedTransport {
        calls: Arc::new(AtomicUsize::new(0)),
        entered: Arc::new(Semaphore::new(0)),
        release: Arc::new(Semaphore::new(0)),
    };
    let setup = SetupService::ready(data.path(), None, database.clone());
    let observed = transport.clone();
    let (runtime, handle) = FeedRuntime::new(setup, move |database| {
        Ok(Arc::new(FeedExecutor::with_jitter(
            FeedRepository::new(database),
            FeedUrlPolicy::new(false),
            observed.clone(),
            ZeroJitter,
        )))
    });
    let task = tokio::spawn(runtime.run());

    wait_for_entries(&transport.entered, 1, Duration::from_secs(1)).await;
    let initial_deadline = feed::Entity::find_by_id(FEED_ID)
        .one(&database)
        .await
        .expect("claimed feed should query")
        .expect("claimed feed should exist")
        .lease_until
        .expect("claimed feed should have a lease deadline");
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(20)).await;
    tokio::task::yield_now().await;
    tokio::time::resume();
    wait_for_lease_after(&database, initial_deadline).await;

    transport.release.add_permits(1);
    wait_for_terminal(&database, FIRST_RUNTIME_RUN_ID, Duration::from_secs(1)).await;
    handle.shutdown();
    task.await
        .expect("runtime task should join")
        .expect("heartbeat runtime should stop cleanly");
}

#[tokio::test]
async fn terminal_completion_stops_heartbeat_without_false_lease_lost() {
    let (data, database) = ready_runtime_database("terminal-stops-heartbeat").await;
    install_lease_extension_audit(&database).await;
    insert_queued_run(
        &database,
        FIRST_RUNTIME_RUN_ID,
        FEED_ID,
        "runtime:terminal-first",
    )
    .await;
    let calls = Arc::new(AtomicUsize::new(0));
    let terminal_ready = Arc::new(Notify::new());
    let terminal_release = Arc::new(Notify::new());
    let heartbeat_attempts = Arc::new(AtomicUsize::new(0));
    let setup = SetupService::ready(data.path(), None, database.clone());
    let observed = calls.clone();
    let (runtime, handle) = FeedRuntime::new(setup, move |database| {
        Ok(Arc::new(FeedExecutor::with_jitter(
            FeedRepository::new(database),
            FeedUrlPolicy::new(false),
            NotModifiedTransport {
                calls: observed.clone(),
            },
            ZeroJitter,
        )))
    });
    let runtime = runtime.with_terminal_ready_hook(
        terminal_ready.clone(),
        terminal_release.clone(),
        heartbeat_attempts.clone(),
    );
    let task = tokio::spawn(runtime.run());

    terminal_ready.notified().await;
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(20)).await;
    terminal_release.notify_one();
    tokio::time::resume();
    handle.shutdown();
    task.await
        .expect("runtime task should join")
        .expect("terminal runtime should stop cleanly");
    let runs = feed_refresh_run::Entity::find()
        .all(&database)
        .await
        .expect("terminal runs should query");
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].status, RefreshStatus::NotModified.as_str());
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(heartbeat_attempts.load(Ordering::SeqCst), 0);
    assert_eq!(lease_extension_audit_count(&database).await, 0);
}

#[tokio::test]
async fn heartbeat_lease_loss_cancels_attempt_and_queues_retry() {
    let (data, database) = ready_runtime_database("heartbeat-lease-loss").await;
    insert_queued_run(
        &database,
        FIRST_RUNTIME_RUN_ID,
        FEED_ID,
        "runtime:lease-loss",
    )
    .await;
    let transport = CancellationTransport {
        calls: Arc::new(AtomicUsize::new(0)),
        entered: Arc::new(Semaphore::new(0)),
        cancelled: Arc::new(Semaphore::new(0)),
    };
    let setup = SetupService::ready(data.path(), None, database.clone());
    let observed = transport.clone();
    let (runtime, handle) = FeedRuntime::new(setup, move |database| {
        Ok(Arc::new(FeedExecutor::with_jitter(
            FeedRepository::new(database),
            FeedUrlPolicy::new(false),
            observed.clone(),
            ZeroJitter,
        )))
    });
    let task = tokio::spawn(runtime.run());
    wait_for_entries(&transport.entered, 1, Duration::from_secs(1)).await;

    steal_feed_lease(&database).await;
    tokio::time::pause();
    handle.shutdown();
    tokio::time::advance(Duration::from_secs(20)).await;
    tokio::task::yield_now().await;
    tokio::time::resume();
    wait_for_entries(&transport.cancelled, 1, Duration::from_secs(1)).await;
    task.await
        .expect("runtime task should join")
        .expect("lease-loss runtime should stop cleanly");

    let old = feed_refresh_run::Entity::find_by_id(FIRST_RUNTIME_RUN_ID)
        .one(&database)
        .await
        .expect("lease-lost run should query")
        .expect("lease-lost run should exist");
    assert_eq!(old.status, RefreshStatus::LeaseLost.as_str());
    let retries = feed_refresh_run::Entity::find()
        .all(&database)
        .await
        .expect("lease-loss retries should query")
        .into_iter()
        .filter(|run| run.idempotency_key == format!("r1:{FIRST_RUNTIME_RUN_ID}"))
        .collect::<Vec<_>>();
    assert_eq!(retries.len(), 1);
    assert_eq!(retries[0].status, RefreshStatus::Queued.as_str());
    assert_eq!(transport.calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn scheduled_enqueue_skips_disabled_orphan_unsubscribed_and_active() {
    let (_data, database) = ready_runtime_database("scheduled-skip-filters").await;
    let repository = FeedRepository::new(database.clone());
    set_feed_schedule_state(&database, true, None).await;
    assert_eq!(repository.enqueue_due_scheduled(100).await.unwrap(), 0);

    set_feed_schedule_state(&database, false, Some(OffsetDateTime::now_utc())).await;
    assert_eq!(repository.enqueue_due_scheduled(100).await.unwrap(), 0);

    set_feed_schedule_state(&database, false, None).await;
    subscription::Entity::delete_by_id(SUBSCRIPTION_A_ID)
        .exec(&database)
        .await
        .expect("subscription fixture should delete");
    assert_eq!(repository.enqueue_due_scheduled(100).await.unwrap(), 0);

    insert_subscription(
        &database,
        SUBSCRIPTION_A_ID,
        USER_A_ID,
        OffsetDateTime::now_utc(),
    )
    .await;
    insert_queued_run(
        &database,
        FIRST_RUNTIME_RUN_ID,
        FEED_ID,
        "runtime:active-schedule-filter",
    )
    .await;
    assert_eq!(repository.enqueue_due_scheduled(100).await.unwrap(), 0);
    assert_eq!(
        feed_refresh_run::Entity::find()
            .all(&database)
            .await
            .expect("active runs should query")
            .len(),
        1
    );
}

#[tokio::test]
async fn scheduled_enqueue_revalidates_snapshot_after_waiting_for_feed_lock() {
    let data = tempfile::tempdir().expect("temporary directory should be created");
    let url = format!(
        "sqlite://{}?mode=rwc",
        data.path().join("scheduled-snapshot-race.db").display()
    );
    let database = connect_for_contract(SecretString::from(url.clone())).await;
    migrate(&database).await.expect("migrations should apply");
    seed_due_subscribed_feed(&database).await;
    let blocker = connect_for_contract(SecretString::from(url)).await;
    let transaction = blocker
        .begin()
        .await
        .expect("Feed blocker transaction should start");
    transaction
        .execute(Statement::from_sql_and_values(
            DatabaseBackend::Sqlite,
            "UPDATE feeds SET lease_token = lease_token WHERE id = ?",
            [FEED_ID.into()],
        ))
        .await
        .expect("Feed blocker should hold the Feed write lock");

    let scanned = Arc::new(Notify::new());
    let repository = FeedRepository::new(database.clone());
    let observed = scanned.clone();
    let enqueue = tokio::spawn(async move {
        repository
            .enqueue_due_scheduled_after_scan(100, observed)
            .await
    });
    scanned.notified().await;
    transaction
        .execute(Statement::from_sql_and_values(
            DatabaseBackend::Sqlite,
            "UPDATE feeds
             SET next_fetch_at = strftime('%Y-%m-%dT%H:%M:%f000Z', 'now', '+1 hour')
             WHERE id = ?",
            [FEED_ID.into()],
        ))
        .await
        .expect("blocker should advance the scanned schedule version");
    transaction
        .commit()
        .await
        .expect("Feed blocker should commit the newer schedule version");

    assert_eq!(
        enqueue
            .await
            .expect("scheduled race task should join")
            .expect("scheduled race should not fail"),
        0
    );
    assert!(
        feed_refresh_run::Entity::find()
            .all(&database)
            .await
            .expect("scheduled race runs should query")
            .is_empty()
    );
}

#[tokio::test]
async fn scheduled_idempotency_key_matches_frozen_frame_golden() {
    const NEXT_FETCH_AT: OffsetDateTime = datetime!(2026-07-16 12:00:00 UTC);
    const EXPECTED_KEY: &str = "s1:4C65jR4OytA8YRsC3qD2yjWK_vizX9HFvSYzKVJM0Qo";

    let data = tempfile::tempdir().expect("temporary directory should be created");
    let database_url = format!(
        "sqlite://{}?mode=rwc",
        data.path().join("scheduled-golden.db").display()
    );
    let database = connect_for_contract(SecretString::from(database_url)).await;
    migrate(&database).await.expect("migrations should apply");
    seed_due_subscribed_feed(&database).await;
    let model = feed::Entity::find_by_id(FEED_ID)
        .one(&database)
        .await
        .expect("golden Feed should query")
        .expect("golden Feed should exist");
    let mut active: feed::ActiveModel = model.into();
    active.next_fetch_at = Set(NEXT_FETCH_AT);
    active
        .update(&database)
        .await
        .expect("golden schedule version should update");

    assert_eq!(
        FeedRepository::new(database.clone())
            .enqueue_due_scheduled(100)
            .await
            .expect("golden schedule should enqueue"),
        1
    );
    let run = feed_refresh_run::Entity::find()
        .one(&database)
        .await
        .expect("golden scheduled run should query")
        .expect("golden scheduled run should exist");
    assert_eq!(run.idempotency_key, EXPECTED_KEY);
}

#[tokio::test]
async fn multi_instance_recovery_and_scheduled_enqueue_are_idempotent() {
    let data = tempfile::tempdir().expect("temporary directory should be created");
    let url = format!(
        "sqlite://{}?mode=rwc",
        data.path().join("multi-instance-maintenance.db").display()
    );
    maintenance_idempotency_contract(url).await;
}

#[tokio::test]
async fn postgres_recovery_and_scheduler_contract() {
    let Ok(url) = std::env::var("RAINDROP_TEST_POSTGRES_URL") else {
        eprintln!("postgres runtime contract skipped: test database URL is not configured");
        return;
    };
    maintenance_idempotency_contract(url).await;
}

#[tokio::test]
async fn mysql_recovery_and_scheduler_contract() {
    let Ok(url) = std::env::var("RAINDROP_TEST_MYSQL_URL") else {
        eprintln!("mysql runtime contract skipped: test database URL is not configured");
        return;
    };
    maintenance_idempotency_contract(url).await;
}

#[tokio::test]
async fn graceful_shutdown_stops_new_claims() {
    let (data, database) = ready_runtime_database("graceful-shutdown").await;
    seed_second_subscribed_feed(&database).await;
    insert_queued_run(
        &database,
        FIRST_RUNTIME_RUN_ID,
        FEED_ID,
        "runtime:shutdown-running",
    )
    .await;
    let transport = BlockedNotModifiedTransport {
        calls: Arc::new(AtomicUsize::new(0)),
        entered: Arc::new(Semaphore::new(0)),
        release: Arc::new(Semaphore::new(0)),
    };
    let setup = SetupService::ready(data.path(), None, database.clone());
    let observed = transport.clone();
    let (runtime, handle) = FeedRuntime::new(setup, move |database| {
        Ok(Arc::new(FeedExecutor::with_jitter(
            FeedRepository::new(database),
            FeedUrlPolicy::new(false),
            observed.clone(),
            ZeroJitter,
        )))
    });
    let task = tokio::spawn(runtime.run());
    wait_for_entries(&transport.entered, 1, Duration::from_secs(1)).await;

    handle.shutdown();
    insert_queued_run(
        &database,
        SECOND_RUNTIME_RUN_ID,
        SECOND_FEED_ID,
        "runtime:shutdown-queued",
    )
    .await;
    transport.release.add_permits(1);
    task.await
        .expect("runtime task should join")
        .expect("graceful runtime should stop cleanly");

    let queued = feed_refresh_run::Entity::find_by_id(SECOND_RUNTIME_RUN_ID)
        .one(&database)
        .await
        .expect("post-shutdown run should query")
        .expect("post-shutdown run should exist");
    assert_eq!(queued.status, RefreshStatus::Queued.as_str());
    assert_eq!(transport.calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn shutdown_bound_aborts_blocked_attempt_after_thirty_seconds() {
    let (data, database) = ready_runtime_database("shutdown-bound").await;
    seed_second_subscribed_feed(&database).await;
    insert_queued_run(
        &database,
        FIRST_RUNTIME_RUN_ID,
        FEED_ID,
        "runtime:shutdown-bound-running",
    )
    .await;
    let transport = CancellationTransport {
        calls: Arc::new(AtomicUsize::new(0)),
        entered: Arc::new(Semaphore::new(0)),
        cancelled: Arc::new(Semaphore::new(0)),
    };
    let setup = SetupService::ready(data.path(), None, database.clone());
    let observed = transport.clone();
    let (runtime, handle) = FeedRuntime::new(setup, move |database| {
        Ok(Arc::new(FeedExecutor::with_jitter(
            FeedRepository::new(database),
            FeedUrlPolicy::new(false),
            observed.clone(),
            ZeroJitter,
        )))
    });
    let task = tokio::spawn(runtime.run());
    wait_for_entries(&transport.entered, 1, Duration::from_secs(1)).await;
    tokio::time::sleep(Duration::from_millis(25)).await;

    handle.shutdown();
    for _ in 0..8 {
        tokio::task::yield_now().await;
    }
    insert_queued_run(
        &database,
        SECOND_RUNTIME_RUN_ID,
        SECOND_FEED_ID,
        "runtime:shutdown-bound-queued",
    )
    .await;
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(29)).await;
    tokio::task::yield_now().await;
    assert!(!task.is_finished());
    tokio::time::advance(Duration::from_secs(1)).await;
    tokio::task::yield_now().await;
    tokio::time::resume();

    wait_for_entries(&transport.cancelled, 1, Duration::from_secs(1)).await;
    task.await
        .expect("bounded runtime task should join")
        .expect("bounded runtime should stop cleanly");
    let queued = feed_refresh_run::Entity::find_by_id(SECOND_RUNTIME_RUN_ID)
        .one(&database)
        .await
        .expect("bounded shutdown queued run should query")
        .expect("bounded shutdown queued run should exist");
    assert_eq!(queued.status, RefreshStatus::Queued.as_str());
    assert_eq!(transport.calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn due_scheduled_feed_enqueues_once() {
    let data = tempfile::tempdir().expect("temporary directory should be created");
    let database_url = format!(
        "sqlite://{}?mode=rwc",
        data.path().join("scheduled-enqueue.db").display()
    );
    let database = connect_for_contract(SecretString::from(database_url)).await;
    migrate(&database).await.expect("migrations should apply");
    seed_due_subscribed_feed(&database).await;
    let repository = FeedRepository::new(database.clone());

    assert_eq!(
        repository
            .enqueue_due_scheduled(100)
            .await
            .expect("due scheduling should succeed"),
        1
    );
    assert_eq!(
        repository
            .enqueue_due_scheduled(100)
            .await
            .expect("repeated scheduling should stay idempotent"),
        0
    );

    let runs = feed_refresh_run::Entity::find()
        .all(&database)
        .await
        .expect("scheduled runs should query");
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].feed_id, FEED_ID);
    assert_eq!(runs[0].requested_by_user_id, None);
    assert_eq!(runs[0].trigger_kind, RefreshTrigger::Scheduled.as_str());
    assert_eq!(runs[0].status, RefreshStatus::Queued.as_str());
    assert!(runs[0].idempotency_key.starts_with("s1:"));
    assert_eq!(runs[0].idempotency_key.len(), 46);
}

#[tokio::test]
async fn expired_running_run_recovers_to_one_retry() {
    let data = tempfile::tempdir().expect("temporary directory should be created");
    let database_url = format!(
        "sqlite://{}?mode=rwc",
        data.path().join("expired-recovery.db").display()
    );
    let database = connect_for_contract(SecretString::from(database_url)).await;
    migrate(&database).await.expect("migrations should apply");
    seed_expired_running_run(&database).await;
    let repository = FeedRepository::new(database.clone());

    let first = repository
        .recover_expired_runs(100)
        .await
        .expect("expired recovery should succeed");
    let second = repository
        .recover_expired_runs(100)
        .await
        .expect("repeated recovery should stay idempotent");

    assert_eq!(first.len(), 1);
    assert!(second.is_empty());
    let old = feed_refresh_run::Entity::find_by_id(EXPIRED_RUN_ID)
        .one(&database)
        .await
        .expect("expired run should query")
        .expect("expired run should exist");
    assert_eq!(old.status, RefreshStatus::LeaseLost.as_str());
    assert!(old.completed_at.is_some());
    let retries = feed_refresh_run::Entity::find()
        .all(&database)
        .await
        .expect("retry runs should query")
        .into_iter()
        .filter(|run| run.idempotency_key == format!("r1:{EXPIRED_RUN_ID}"))
        .collect::<Vec<_>>();
    assert_eq!(retries.len(), 1);
    assert_eq!(retries[0].id, first[0]);
    assert_eq!(retries[0].feed_id, FEED_ID);
    assert_eq!(retries[0].requested_by_user_id, None);
    assert_eq!(retries[0].trigger_kind, RefreshTrigger::Retry.as_str());
    assert_eq!(retries[0].status, RefreshStatus::Queued.as_str());
}

#[tokio::test]
async fn recovery_retry_inherits_requester_and_exact_identity() {
    let data = tempfile::tempdir().expect("temporary directory should be created");
    let database_url = format!(
        "sqlite://{}?mode=rwc",
        data.path().join("requester-recovery.db").display()
    );
    let database = connect_for_contract(SecretString::from(database_url)).await;
    migrate(&database).await.expect("migrations should apply");
    seed_expired_running_run_with_requester(&database, Some(USER_A_ID)).await;
    let repository = FeedRepository::new(database.clone());

    let queued = repository
        .recover_expired_runs(100)
        .await
        .expect("requester recovery should succeed");

    assert_eq!(queued.len(), 1);
    let runs = feed_refresh_run::Entity::find()
        .all(&database)
        .await
        .expect("requester recovery runs should query");
    assert_eq!(runs.len(), 2);
    let retry = runs
        .iter()
        .find(|run| run.idempotency_key == format!("r1:{EXPIRED_RUN_ID}"))
        .expect("requester recovery should persist its exact retry key");
    assert_eq!(retry.id, queued[0]);
    assert_eq!(retry.feed_id, FEED_ID);
    assert_eq!(retry.requested_by_user_id.as_deref(), Some(USER_A_ID));
    assert_eq!(retry.trigger_kind, RefreshTrigger::Retry.as_str());
    assert_eq!(retry.status, RefreshStatus::Queued.as_str());
    assert_eq!(
        runs.iter()
            .filter(|run| run.idempotency_key == format!("r1:{EXPIRED_RUN_ID}"))
            .count(),
        1
    );
}

#[tokio::test]
async fn recovery_with_another_queued_run_terminalizes_without_retry() {
    let data = tempfile::tempdir().expect("temporary directory should be created");
    let database_url = format!(
        "sqlite://{}?mode=rwc",
        data.path().join("queued-suppresses-recovery.db").display()
    );
    let database = connect_for_contract(SecretString::from(database_url)).await;
    migrate(&database).await.expect("migrations should apply");
    seed_expired_running_run_with_requester(&database, Some(USER_A_ID)).await;
    insert_queued_run(
        &database,
        SECOND_RUNTIME_RUN_ID,
        FEED_ID,
        "runtime:already-queued",
    )
    .await;
    let repository = FeedRepository::new(database.clone());

    assert!(
        repository
            .recover_expired_runs(100)
            .await
            .expect("queued suppression recovery should succeed")
            .is_empty()
    );
    assert_recovery_suppressed(&database, SECOND_RUNTIME_RUN_ID, RefreshStatus::Queued).await;
}

#[tokio::test]
async fn recovery_with_newer_running_run_terminalizes_without_retry() {
    let data = tempfile::tempdir().expect("temporary directory should be created");
    let database_url = format!(
        "sqlite://{}?mode=rwc",
        data.path().join("running-suppresses-recovery.db").display()
    );
    let database = connect_for_contract(SecretString::from(database_url)).await;
    migrate(&database).await.expect("migrations should apply");
    seed_expired_running_run_with_requester(&database, Some(USER_A_ID)).await;
    install_newer_running_run(&database).await;
    let repository = FeedRepository::new(database.clone());

    assert!(
        repository
            .recover_expired_runs(100)
            .await
            .expect("running suppression recovery should succeed")
            .is_empty()
    );
    assert_recovery_suppressed(&database, SECOND_RUNTIME_RUN_ID, RefreshStatus::Running).await;
}

async fn seed_expired_running_run(database: &sea_orm::DatabaseConnection) {
    seed_expired_running_run_with_requester(database, None).await;
}

async fn seed_expired_running_run_with_requester(
    database: &sea_orm::DatabaseConnection,
    requested_by_user_id: Option<&str>,
) {
    let now = OffsetDateTime::now_utc();
    insert_user(database, USER_A_ID, "runtime-reader").await;
    insert_feed(database, now - time::Duration::hours(1)).await;
    insert_subscription(database, SUBSCRIPTION_A_ID, USER_A_ID, now).await;

    let model = feed::Entity::find_by_id(FEED_ID)
        .one(database)
        .await
        .expect("feed should query")
        .expect("feed should exist");
    let mut active: feed::ActiveModel = model.into();
    active.orphaned_at = Set(None);
    active.lease_owner = Set(Some("crashed-worker".to_owned()));
    active.lease_token = Set(2);
    active.lease_until = Set(Some(now - time::Duration::seconds(1)));
    active
        .update(database)
        .await
        .expect("feed lease should expire");

    feed_refresh_run::ActiveModel {
        id: Set(EXPIRED_RUN_ID.to_owned()),
        feed_id: Set(FEED_ID.to_owned()),
        requested_by_user_id: Set(requested_by_user_id.map(str::to_owned)),
        trigger_kind: Set(RefreshTrigger::Scheduled.as_str().to_owned()),
        status: Set(RefreshStatus::Running.as_str().to_owned()),
        idempotency_key: Set("s1:expired-fixture".to_owned()),
        lease_token: Set(Some(2)),
        commit_generation: Set(None),
        queued_at: Set(now - time::Duration::minutes(2)),
        started_at: Set(Some(now - time::Duration::minutes(1))),
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
    .expect("expired running run should insert");
}

async fn install_newer_running_run(database: &sea_orm::DatabaseConnection) {
    let now = OffsetDateTime::now_utc();
    let model = feed::Entity::find_by_id(FEED_ID)
        .one(database)
        .await
        .expect("feed should query")
        .expect("feed should exist");
    let mut active: feed::ActiveModel = model.into();
    active.lease_owner = Set(Some("newer-worker".to_owned()));
    active.lease_token = Set(3);
    active.lease_until = Set(Some(now + time::Duration::minutes(1)));
    active
        .update(database)
        .await
        .expect("newer feed lease should update");

    feed_refresh_run::ActiveModel {
        id: Set(SECOND_RUNTIME_RUN_ID.to_owned()),
        feed_id: Set(FEED_ID.to_owned()),
        requested_by_user_id: Set(Some(USER_A_ID.to_owned())),
        trigger_kind: Set(RefreshTrigger::Retry.as_str().to_owned()),
        status: Set(RefreshStatus::Running.as_str().to_owned()),
        idempotency_key: Set("runtime:newer-running".to_owned()),
        lease_token: Set(Some(3)),
        commit_generation: Set(None),
        queued_at: Set(now - time::Duration::seconds(2)),
        started_at: Set(Some(now - time::Duration::seconds(1))),
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
    .expect("newer running run should insert");
}

async fn assert_recovery_suppressed(
    database: &sea_orm::DatabaseConnection,
    active_run_id: &str,
    active_status: RefreshStatus,
) {
    let runs = feed_refresh_run::Entity::find()
        .all(database)
        .await
        .expect("suppressed recovery runs should query");
    assert_eq!(runs.len(), 2);
    let old = runs
        .iter()
        .find(|run| run.id == EXPIRED_RUN_ID)
        .expect("expired old run should remain persisted");
    assert_eq!(old.status, RefreshStatus::LeaseLost.as_str());
    assert!(old.completed_at.is_some());
    let active = runs
        .iter()
        .find(|run| run.id == active_run_id)
        .expect("other active run should remain persisted");
    assert_eq!(active.status, active_status.as_str());
    assert_eq!(
        runs.iter()
            .filter(|run| run.idempotency_key == format!("r1:{EXPIRED_RUN_ID}"))
            .count(),
        0
    );
}

async fn seed_due_subscribed_feed(database: &sea_orm::DatabaseConnection) {
    let now = OffsetDateTime::now_utc();
    insert_user(database, USER_A_ID, "scheduled-reader").await;
    insert_feed(database, now - time::Duration::hours(1)).await;
    insert_subscription(database, SUBSCRIPTION_A_ID, USER_A_ID, now).await;
    let model = feed::Entity::find_by_id(FEED_ID)
        .one(database)
        .await
        .expect("feed should query")
        .expect("feed should exist");
    let mut active: feed::ActiveModel = model.into();
    active.orphaned_at = Set(None);
    active.lease_owner = Set(None);
    active.lease_until = Set(None);
    active.next_fetch_at = Set(now - time::Duration::minutes(1));
    active
        .update(database)
        .await
        .expect("feed should become due");
}

async fn seed_future_subscribed_feed(database: &sea_orm::DatabaseConnection) {
    let now = OffsetDateTime::now_utc();
    insert_user(database, USER_A_ID, "runtime-reader").await;
    insert_feed(database, now - time::Duration::hours(1)).await;
    let model = feed::Entity::find_by_id(FEED_ID)
        .one(database)
        .await
        .expect("future feed should query")
        .expect("future feed should exist");
    let mut active: feed::ActiveModel = model.into();
    active.orphaned_at = Set(None);
    active.lease_owner = Set(None);
    active.lease_until = Set(None);
    active.next_fetch_at = Set(now + time::Duration::hours(1));
    active.retry_after_at = Set(None);
    active.validator_url = Set(None);
    active.etag = Set(None);
    active.last_modified = Set(None);
    active
        .update(database)
        .await
        .expect("future feed schedule should update");
    insert_subscription(database, SUBSCRIPTION_A_ID, USER_A_ID, now).await;
}

async fn ready_runtime_database(name: &str) -> (tempfile::TempDir, sea_orm::DatabaseConnection) {
    let data = tempfile::tempdir().expect("temporary directory should be created");
    let database_url = format!(
        "sqlite://{}?mode=rwc",
        data.path().join(format!("{name}.db")).display()
    );
    let database = connect_for_contract(SecretString::from(database_url)).await;
    migrate(&database).await.expect("migrations should apply");
    seed_due_subscribed_feed(&database).await;
    let model = feed::Entity::find_by_id(FEED_ID)
        .one(&database)
        .await
        .expect("feed should query")
        .expect("feed should exist");
    let mut active: feed::ActiveModel = model.into();
    active.next_fetch_at = Set(OffsetDateTime::now_utc() + time::Duration::hours(1));
    active.validator_url = Set(None);
    active.etag = Set(None);
    active.last_modified = Set(None);
    active
        .update(&database)
        .await
        .expect("runtime feed should not be scheduled immediately");
    (data, database)
}

async fn seed_second_subscribed_feed(database: &sea_orm::DatabaseConnection) {
    let model = feed::Entity::find_by_id(FEED_ID)
        .one(database)
        .await
        .expect("first feed should query")
        .expect("first feed should exist");
    let mut active: feed::ActiveModel = model.into();
    active.id = Set(SECOND_FEED_ID.to_owned());
    active.source_url = Set("https://example.com/second.xml".to_owned());
    active.normalized_url = Set("https://example.com/second.xml".to_owned());
    active.normalized_url_hash =
        Set("eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee".to_owned());
    active.fetch_url = Set("https://example.com/second.xml".to_owned());
    active.validator_url = Set(None);
    active.etag = Set(None);
    active.last_modified = Set(None);
    active.response_content_hash = Set(None);
    active.lease_owner = Set(None);
    active.lease_token = Set(0);
    active.lease_until = Set(None);
    active
        .insert(database)
        .await
        .expect("second feed should insert");

    let mut subscription =
        subscription_model(SECOND_SUBSCRIPTION_ID, USER_A_ID, OffsetDateTime::now_utc());
    subscription.feed_id = Set(SECOND_FEED_ID.to_owned());
    subscription
        .insert(database)
        .await
        .expect("second subscription should insert");
}

async fn set_feed_schedule_state(
    database: &sea_orm::DatabaseConnection,
    is_disabled: bool,
    orphaned_at: Option<OffsetDateTime>,
) {
    let model = feed::Entity::find_by_id(FEED_ID)
        .one(database)
        .await
        .expect("scheduled feed should query")
        .expect("scheduled feed should exist");
    let mut active: feed::ActiveModel = model.into();
    active.is_disabled = Set(is_disabled);
    active.orphaned_at = Set(orphaned_at);
    active.next_fetch_at = Set(OffsetDateTime::now_utc() - time::Duration::minutes(1));
    active
        .update(database)
        .await
        .expect("scheduled feed state should update");
}

async fn install_lease_extension_audit(database: &sea_orm::DatabaseConnection) {
    database
        .execute(Statement::from_string(
            DatabaseBackend::Sqlite,
            "CREATE TABLE lease_extension_audit (observed INTEGER NOT NULL)".to_owned(),
        ))
        .await
        .expect("lease extension audit table should create");
    database
        .execute(Statement::from_string(
            DatabaseBackend::Sqlite,
            "CREATE TRIGGER observe_lease_extension
             AFTER UPDATE OF lease_until ON feeds
             WHEN OLD.lease_until IS NOT NULL AND NEW.lease_until IS NOT NULL
             BEGIN
                 INSERT INTO lease_extension_audit (observed) VALUES (1);
             END"
            .to_owned(),
        ))
        .await
        .expect("lease extension audit trigger should create");
}

async fn lease_extension_audit_count(database: &sea_orm::DatabaseConnection) -> i64 {
    database
        .query_one(Statement::from_string(
            DatabaseBackend::Sqlite,
            "SELECT COUNT(*) AS audit_count FROM lease_extension_audit".to_owned(),
        ))
        .await
        .expect("lease extension audit should query")
        .expect("lease extension audit count should exist")
        .try_get("", "audit_count")
        .expect("lease extension audit count should decode")
}

async fn maintenance_idempotency_contract(url: String) {
    let first_database = connect_for_contract(SecretString::from(url.clone())).await;
    rollback(&first_database)
        .await
        .expect("runtime contract database should reset");
    migrate(&first_database)
        .await
        .expect("runtime contract migrations should apply");
    seed_expired_running_run(&first_database).await;
    seed_second_subscribed_feed(&first_database).await;
    let second_database = connect_for_contract(SecretString::from(url)).await;
    let first_repository = FeedRepository::new(first_database.clone());
    let second_repository = FeedRepository::new(second_database.clone());

    let (first_recovery, second_recovery) = tokio::join!(
        first_repository.recover_expired_runs(100),
        second_repository.recover_expired_runs(100),
    );
    let recovered = first_recovery.expect("first recovery should succeed").len()
        + second_recovery
            .expect("second recovery should succeed")
            .len();
    assert_eq!(recovered, 1);

    let (first_schedule, second_schedule) = tokio::join!(
        first_repository.enqueue_due_scheduled(100),
        second_repository.enqueue_due_scheduled(100),
    );
    assert_eq!(
        first_schedule.expect("first scheduler should succeed")
            + second_schedule.expect("second scheduler should succeed"),
        1
    );

    let runs = feed_refresh_run::Entity::find()
        .all(&first_database)
        .await
        .expect("maintenance runs should query");
    assert_eq!(
        runs.iter()
            .filter(|run| run.idempotency_key == format!("r1:{EXPIRED_RUN_ID}"))
            .count(),
        1
    );
    assert_eq!(
        runs.iter()
            .filter(|run| {
                run.feed_id == SECOND_FEED_ID && run.idempotency_key.starts_with("s1:")
            })
            .count(),
        1
    );

    first_database
        .close()
        .await
        .expect("first runtime database should close");
    second_database
        .close()
        .await
        .expect("second runtime database should close");
}

async fn insert_queued_run(
    database: &sea_orm::DatabaseConnection,
    run_id: &str,
    feed_id: &str,
    idempotency_key: &str,
) {
    feed_refresh_run::ActiveModel {
        id: Set(run_id.to_owned()),
        feed_id: Set(feed_id.to_owned()),
        requested_by_user_id: Set(None),
        trigger_kind: Set(RefreshTrigger::Retry.as_str().to_owned()),
        status: Set(RefreshStatus::Queued.as_str().to_owned()),
        idempotency_key: Set(idempotency_key.to_owned()),
        lease_token: Set(None),
        commit_generation: Set(None),
        queued_at: Set(OffsetDateTime::now_utc()),
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
    .expect("queued runtime run should insert");
}

async fn wait_for_entries(entered: &Arc<Semaphore>, count: u32, timeout: Duration) {
    tokio::time::timeout(timeout, entered.clone().acquire_many_owned(count))
        .await
        .expect("transport calls should enter before timeout")
        .expect("transport entry semaphore should remain open")
        .forget();
}

async fn wait_for_lease_after(database: &sea_orm::DatabaseConnection, previous: OffsetDateTime) {
    tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            let deadline = feed::Entity::find_by_id(FEED_ID)
                .one(database)
                .await
                .expect("heartbeat feed should query")
                .expect("heartbeat feed should exist")
                .lease_until
                .expect("heartbeat feed should retain a deadline");
            if deadline > previous {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("heartbeat should extend the lease before timeout");
}

async fn steal_feed_lease(database: &sea_orm::DatabaseConnection) {
    let model = feed::Entity::find_by_id(FEED_ID)
        .one(database)
        .await
        .expect("claimed feed should query")
        .expect("claimed feed should exist");
    let mut active: feed::ActiveModel = model.into();
    active.lease_owner = Set(Some("replacement-worker".to_owned()));
    active.lease_token = Set(active.lease_token.as_ref() + 1);
    active.lease_until = Set(Some(OffsetDateTime::now_utc() + time::Duration::minutes(1)));
    active
        .update(database)
        .await
        .expect("replacement lease should update");
}

async fn wait_for_terminal(
    database: &sea_orm::DatabaseConnection,
    run_id: &str,
    timeout: Duration,
) {
    let completed = tokio::time::timeout(timeout, async {
        loop {
            let run = feed_refresh_run::Entity::find_by_id(run_id)
                .one(database)
                .await
                .expect("refresh run should query")
                .expect("refresh run should exist");
            if !matches!(run.status.as_str(), "QUEUED" | "RUNNING") {
                assert_eq!(run.status, RefreshStatus::NotModified.as_str());
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await;
    if completed.is_err() {
        let run = feed_refresh_run::Entity::find_by_id(run_id)
            .one(database)
            .await
            .expect("timed-out refresh should query")
            .expect("timed-out refresh should exist");
        panic!(
            "runtime should terminalize queued work before timeout; status was {}",
            run.status
        );
    }
}
