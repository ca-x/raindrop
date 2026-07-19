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
    content::{
        jobs::{
            ArtifactCandidate, ArtifactIdentity, ArtifactIdentityInput, ArtifactKind,
            AttemptFailure, AttemptUsage, ClaimContentJob, ClaimOutcome, ContentJobClaim,
            ContentJobOperation, ContentJobTrigger, ContentRepository, EnqueueContentJob,
            EnqueueContentJobInput, EnqueueResult, JobStatus,
        },
        provider::ProviderKind,
        worker::{
            ContentProcessFailure, ContentProcessSuccess, ContentProcessor, ContentRuntime,
            ContentWorker,
        },
    },
    db::{
        entities::{content_job, content_job_attempt},
        migrate,
    },
};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, sea_query::Expr};
use secrecy::SecretString;
use serde_json::json;
use support::database::{
    ENTRY_A_ID, HASH_A, HASH_C, HASH_D, connect_for_contract, insert_entry, insert_feed,
    insert_subscription, insert_user,
};
use time::{OffsetDateTime, macros::datetime};
use tokio::sync::{Notify, Semaphore};

const PROVIDER_ID: &str = "00000000-0000-4000-8000-000000000901";

#[tokio::test]
async fn worker_terminalizes_success_and_retryable_failure() {
    let fixture = RuntimeFixture::new("worker-terminal", 1).await;
    let success_job = fixture.enqueue(0, "success").await;
    let success_claim = fixture.claim("worker-success").await;
    ContentWorker::new(
        Arc::clone(&fixture.repository),
        Arc::new(ImmediateProcessor::Success),
    )
    .run_claim(success_claim)
    .await
    .expect("success claim should terminalize");
    assert_eq!(
        fixture.job(&success_job).await.status(),
        JobStatus::Succeeded
    );

    let retry_job = fixture.enqueue(0, "retry").await;
    let retry_claim = fixture.claim("worker-retry").await;
    ContentWorker::new(
        Arc::clone(&fixture.repository),
        Arc::new(ImmediateProcessor::RetryableFailure),
    )
    .run_claim(retry_claim)
    .await
    .expect("retryable failure should terminalize its attempt");
    let retry = fixture.job(&retry_job).await;
    assert_eq!(retry.status(), JobStatus::RetryWait);
    assert_eq!(retry.last_error_code(), Some("FAKE_RETRY"));
    let attempts = fixture
        .repository
        .list_attempts(&fixture.users[0], &retry_job)
        .await
        .expect("retry attempts");
    assert_eq!(attempts.len(), 1);
    assert_eq!(attempts[0].error_code(), Some("FAKE_RETRY"));
    assert_eq!(attempts[0].retryable(), Some(true));
}

#[tokio::test]
async fn worker_heartbeat_extends_lease_and_lease_loss_cancels_without_terminal_write() {
    let extension_fixture = RuntimeFixture::new("heartbeat-extension", 1).await;
    let extension_job = extension_fixture.enqueue(0, "extension").await;
    let extension_claim = extension_fixture.claim("heartbeat-extension").await;
    let extension_processor = Arc::new(BlockingProcessor::new());
    let extension_worker = ContentWorker::new(
        Arc::clone(&extension_fixture.repository),
        extension_processor.clone(),
    );
    let extension_task =
        tokio::spawn(async move { extension_worker.run_claim(extension_claim).await });
    wait_for_permits(&extension_processor.entered, 1, Duration::from_secs(2)).await;
    let shortened = OffsetDateTime::now_utc() + time::Duration::seconds(5);
    content_job::Entity::update_many()
        .col_expr(
            content_job::Column::LeaseUntil,
            Expr::value(Some(shortened)),
        )
        .filter(content_job::Column::Id.eq(&extension_job))
        .exec(&extension_fixture.database)
        .await
        .expect("shorten content lease");
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(11)).await;
    tokio::task::yield_now().await;
    tokio::time::resume();
    wait_for_lease_after(
        &extension_fixture.database,
        &extension_job,
        shortened,
        Duration::from_secs(2),
    )
    .await;
    extension_processor.release.add_permits(1);
    extension_task
        .await
        .expect("extension worker join")
        .expect("extension worker result");

    let loss_fixture = RuntimeFixture::new("heartbeat-loss", 1).await;
    let loss_job = loss_fixture.enqueue(0, "loss").await;
    let loss_claim = loss_fixture.claim("heartbeat-loss").await;
    let loss_processor = Arc::new(BlockingProcessor::new());
    let loss_worker =
        ContentWorker::new(Arc::clone(&loss_fixture.repository), loss_processor.clone());
    let loss_task = tokio::spawn(async move { loss_worker.run_claim(loss_claim).await });
    wait_for_permits(&loss_processor.entered, 1, Duration::from_secs(2)).await;
    content_job::Entity::update_many()
        .col_expr(
            content_job::Column::LeaseOwner,
            Expr::value(Some("stolen-worker")),
        )
        .filter(content_job::Column::Id.eq(&loss_job))
        .exec(&loss_fixture.database)
        .await
        .expect("steal content lease");
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(10)).await;
    tokio::task::yield_now().await;
    tokio::time::resume();
    wait_for_permits(&loss_processor.cancelled, 1, Duration::from_secs(2)).await;
    loss_task
        .await
        .expect("lease-loss worker join")
        .expect("lease loss should converge without a second terminal effect");
    let stored = content_job::Entity::find_by_id(&loss_job)
        .one(&loss_fixture.database)
        .await
        .expect("lease-loss job query")
        .expect("lease-loss job exists");
    assert_eq!(stored.status, "RUNNING");
    let attempt = content_job_attempt::Entity::find()
        .filter(content_job_attempt::Column::JobId.eq(&loss_job))
        .one(&loss_fixture.database)
        .await
        .expect("lease-loss attempt query")
        .expect("lease-loss attempt exists");
    assert_eq!(attempt.status, "RUNNING");
    assert!(attempt.completed_at.is_none());
}

#[tokio::test]
async fn worker_stops_heartbeat_before_terminal_commit() {
    let fixture = RuntimeFixture::new("terminal-order", 1).await;
    let job_id = fixture.enqueue(0, "terminal-order").await;
    let claim = fixture.claim("terminal-order").await;
    let ready = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());
    let heartbeat_attempts = Arc::new(AtomicUsize::new(0));
    let worker = ContentWorker::new(
        Arc::clone(&fixture.repository),
        Arc::new(ImmediateProcessor::Success),
    )
    .with_terminal_ready_hook(
        Arc::clone(&ready),
        Arc::clone(&release),
        Arc::clone(&heartbeat_attempts),
    );
    let task = tokio::spawn(async move { worker.run_claim(claim).await });
    ready.notified().await;
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(20)).await;
    tokio::task::yield_now().await;
    assert_eq!(heartbeat_attempts.load(Ordering::SeqCst), 0);
    release.notify_one();
    tokio::time::resume();
    task.await
        .expect("terminal ordering worker join")
        .expect("terminal ordering worker result");
    assert_eq!(fixture.job(&job_id).await.status(), JobStatus::Succeeded);
}

#[tokio::test]
async fn runtime_notify_and_poll_both_wake_work() {
    let fixture = RuntimeFixture::new("notify-poll", 1).await;
    let calls = Arc::new(AtomicUsize::new(0));
    let processor = Arc::new(CountingProcessor {
        calls: Arc::clone(&calls),
    });
    let (runtime, handle) = ContentRuntime::new(Arc::clone(&fixture.repository), processor);
    let task = tokio::spawn(runtime.run());

    let notified = fixture.enqueue(0, "notified").await;
    handle.notify();
    fixture
        .wait_for_status(&notified, JobStatus::Succeeded, Duration::from_millis(500))
        .await;

    let polled = fixture.enqueue(0, "polled").await;
    fixture
        .wait_for_status(&polled, JobStatus::Succeeded, Duration::from_secs(2))
        .await;

    handle.shutdown();
    task.await
        .expect("notify/poll runtime join")
        .expect("notify/poll runtime result");
    assert_eq!(calls.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn runtime_runs_exactly_eight_lanes_and_never_exceeds_the_ceiling() {
    let fixture = RuntimeFixture::new("eight-lanes", 10).await;
    for index in 0..10 {
        fixture.enqueue(index, &format!("lane-{index}")).await;
    }
    let processor = Arc::new(BlockingProcessor::new());
    let (runtime, handle) = ContentRuntime::new(Arc::clone(&fixture.repository), processor.clone());
    let task = tokio::spawn(runtime.run());
    handle.notify();

    wait_for_permits(&processor.entered, 8, Duration::from_secs(2)).await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(processor.active.load(Ordering::SeqCst), 8);
    assert_eq!(processor.maximum.load(Ordering::SeqCst), 8);
    assert_eq!(processor.entered.available_permits(), 0);

    processor.release.add_permits(8);
    wait_for_permits(&processor.entered, 2, Duration::from_secs(2)).await;
    assert!(processor.maximum.load(Ordering::SeqCst) <= 8);
    processor.release.add_permits(2);
    for index in 0..10 {
        let job_id = fixture.job_ids[index]
            .lock()
            .expect("job id lock")
            .clone()
            .expect("lane job id");
        fixture
            .wait_for_status(&job_id, JobStatus::Succeeded, Duration::from_secs(2))
            .await;
    }

    handle.shutdown();
    task.await
        .expect("eight-lane runtime join")
        .expect("eight-lane runtime result");
}

#[tokio::test]
async fn runtime_shutdown_is_bounded_and_cancels_remaining_lane_futures() {
    let fixture = RuntimeFixture::new("bounded-shutdown", 1).await;
    fixture.enqueue(0, "bounded").await;
    let processor = Arc::new(BlockingProcessor::new());
    let (runtime, handle) = ContentRuntime::new(Arc::clone(&fixture.repository), processor.clone());
    let task = tokio::spawn(runtime.run());
    handle.notify();
    wait_for_permits(&processor.entered, 1, Duration::from_secs(2)).await;

    tokio::time::pause();
    handle.shutdown();
    tokio::task::yield_now().await;
    tokio::time::advance(Duration::from_secs(30)).await;
    tokio::task::yield_now().await;
    tokio::time::resume();
    task.await
        .expect("bounded shutdown runtime join")
        .expect("bounded shutdown runtime result");
    wait_for_permits(&processor.cancelled, 1, Duration::from_secs(2)).await;
}

enum ImmediateProcessor {
    Success,
    RetryableFailure,
}

#[async_trait]
impl ContentProcessor for ImmediateProcessor {
    async fn process(
        &self,
        claim: &ContentJobClaim,
        remaining_attempt: Duration,
    ) -> Result<ContentProcessSuccess, ContentProcessFailure> {
        assert!(!remaining_attempt.is_zero());
        match self {
            Self::Success => Ok(fake_success(claim)),
            Self::RetryableFailure => Err(ContentProcessFailure::from_attempt_failure(
                AttemptFailure::new(
                    "FAKE_RETRY".to_owned(),
                    true,
                    false,
                    Some(Duration::from_secs(70)),
                    AttemptUsage::empty(),
                )
                .expect("fake retry failure"),
            )),
        }
    }
}

struct CountingProcessor {
    calls: Arc<AtomicUsize>,
}

#[async_trait]
impl ContentProcessor for CountingProcessor {
    async fn process(
        &self,
        claim: &ContentJobClaim,
        _remaining_attempt: Duration,
    ) -> Result<ContentProcessSuccess, ContentProcessFailure> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(fake_success(claim))
    }
}

struct BlockingProcessor {
    entered: Arc<Semaphore>,
    release: Arc<Semaphore>,
    cancelled: Arc<Semaphore>,
    active: Arc<AtomicUsize>,
    maximum: Arc<AtomicUsize>,
}

impl BlockingProcessor {
    fn new() -> Self {
        Self {
            entered: Arc::new(Semaphore::new(0)),
            release: Arc::new(Semaphore::new(0)),
            cancelled: Arc::new(Semaphore::new(0)),
            active: Arc::new(AtomicUsize::new(0)),
            maximum: Arc::new(AtomicUsize::new(0)),
        }
    }
}

#[async_trait]
impl ContentProcessor for BlockingProcessor {
    async fn process(
        &self,
        claim: &ContentJobClaim,
        _remaining_attempt: Duration,
    ) -> Result<ContentProcessSuccess, ContentProcessFailure> {
        let active = self.active.fetch_add(1, Ordering::SeqCst) + 1;
        self.maximum.fetch_max(active, Ordering::SeqCst);
        self.entered.add_permits(1);
        let mut guard = ActiveGuard {
            active: Arc::clone(&self.active),
            cancelled: Arc::clone(&self.cancelled),
            completed: false,
        };
        self.release
            .acquire()
            .await
            .expect("blocking processor release")
            .forget();
        guard.completed = true;
        Ok(fake_success(claim))
    }
}

struct ActiveGuard {
    active: Arc<AtomicUsize>,
    cancelled: Arc<Semaphore>,
    completed: bool,
}

impl Drop for ActiveGuard {
    fn drop(&mut self) {
        self.active.fetch_sub(1, Ordering::SeqCst);
        if !self.completed {
            self.cancelled.add_permits(1);
        }
    }
}

fn fake_success(claim: &ContentJobClaim) -> ContentProcessSuccess {
    ContentProcessSuccess::new(
        ArtifactCandidate::new(
            claim.identity().clone(),
            "fake-model".to_owned(),
            json!({"schemaVersion": 1}),
            json!({"schemaVersion": 1}),
        )
        .expect("fake artifact candidate"),
        AttemptUsage::empty(),
    )
}

struct RuntimeFixture {
    _data: tempfile::TempDir,
    database: sea_orm::DatabaseConnection,
    repository: Arc<ContentRepository>,
    users: Vec<String>,
    job_ids: Vec<std::sync::Mutex<Option<String>>>,
}

impl RuntimeFixture {
    async fn new(name: &str, user_count: usize) -> Self {
        let data = tempfile::tempdir().expect("temporary content runtime directory");
        let url = format!(
            "sqlite://{}?mode=rwc",
            data.path()
                .join(format!("content-runtime-{name}.db"))
                .display()
        );
        let database = connect_for_contract(SecretString::from(url)).await;
        migrate(&database).await.expect("content runtime migration");
        let now = datetime!(2026-07-19 12:00:00 UTC);
        insert_feed(&database, now).await;
        insert_entry(
            &database,
            ENTRY_A_ID,
            1,
            "content-runtime-entry",
            HASH_A,
            Some(1_752_926_400_000_000),
            now,
        )
        .await;
        let mut users = Vec::with_capacity(user_count);
        for index in 0..user_count {
            let user_id = format!("00000000-0000-4000-8000-{:012}", index + 1);
            let subscription_id = format!("00000000-0000-4000-9000-{:012}", index + 1);
            insert_user(&database, &user_id, &format!("runtime-user-{name}-{index}")).await;
            insert_subscription(&database, &subscription_id, &user_id, now).await;
            users.push(user_id);
        }
        Self {
            _data: data,
            database: database.clone(),
            repository: Arc::new(ContentRepository::new(database)),
            users,
            job_ids: (0..user_count)
                .map(|_| std::sync::Mutex::new(None))
                .collect(),
        }
    }

    async fn enqueue(&self, user_index: usize, key: &str) -> String {
        let user_id = &self.users[user_index];
        let identity = ArtifactIdentity::new(ArtifactIdentityInput {
            user_id: user_id.clone(),
            entry_id: ENTRY_A_ID.to_owned(),
            kind: ArtifactKind::AiSummary,
            target_locale: None,
            entry_content_hash: HASH_D.to_owned(),
            input_hash: test_hash("raindrop.content-runtime-input.v1", key.as_bytes()),
            config_hash: HASH_C.to_owned(),
            plugin_key: "raindrop.ai-content".to_owned(),
            plugin_version: "1.0.0".to_owned(),
            component_digest: HASH_A.to_owned(),
            provider_binding_id: PROVIDER_ID.to_owned(),
            provider_kind: ProviderKind::OpenAiResponses,
            provider_model: "fake-model".to_owned(),
            provider_revision: 0,
            prompt_version: "raindrop-summary-v1".to_owned(),
            schema_id: "raindrop://schemas/artifacts/ai-summary/v1".to_owned(),
            mcp_provenance_hash: HASH_A.to_owned(),
        })
        .expect("runtime artifact identity");
        let request = EnqueueContentJob::new(EnqueueContentJobInput {
            operation: ContentJobOperation::Summarize,
            trigger: ContentJobTrigger::ManualApi,
            identity,
            idempotency_key: format!("runtime-{user_index}-{key}"),
            call_chain_id: format!("chain-{user_index}-{key}"),
            remaining_depth: 2,
        })
        .expect("runtime enqueue request");
        let job_id = match self
            .repository
            .enqueue(request)
            .await
            .expect("runtime enqueue")
        {
            EnqueueResult::Queued(job) => job.id().to_owned(),
            other => panic!("expected queued runtime job, got {other:?}"),
        };
        *self.job_ids[user_index].lock().expect("job id lock") = Some(job_id.clone());
        job_id
    }

    async fn claim(&self, owner: &str) -> ContentJobClaim {
        match self
            .repository
            .claim_next(ClaimContentJob::new(owner.to_owned()).expect("runtime owner"))
            .await
            .expect("runtime claim")
        {
            ClaimOutcome::Claimed(claim) => claim,
            other => panic!("expected runtime claim, got {other:?}"),
        }
    }

    async fn job(&self, job_id: &str) -> raindrop::content::jobs::JobSnapshot {
        let stored = content_job::Entity::find_by_id(job_id)
            .one(&self.database)
            .await
            .expect("runtime job query")
            .expect("runtime job exists");
        self.repository
            .get_job(&stored.user_id, job_id)
            .await
            .expect("runtime job snapshot")
    }

    async fn wait_for_status(&self, job_id: &str, status: JobStatus, wait: Duration) {
        tokio::time::timeout(wait, async {
            loop {
                if self.job(job_id).await.status() == status {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("runtime job should reach expected status");
    }
}

async fn wait_for_permits(semaphore: &Semaphore, count: u32, wait: Duration) {
    tokio::time::timeout(wait, semaphore.acquire_many(count))
        .await
        .expect("event permits should arrive")
        .expect("event semaphore should remain open")
        .forget();
}

async fn wait_for_lease_after(
    database: &sea_orm::DatabaseConnection,
    job_id: &str,
    expected: OffsetDateTime,
    wait: Duration,
) {
    tokio::time::timeout(wait, async {
        loop {
            let lease = content_job::Entity::find_by_id(job_id)
                .one(database)
                .await
                .expect("content lease query")
                .expect("content lease job")
                .lease_until
                .expect("content lease deadline");
            if lease > expected {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("content heartbeat should extend the lease");
}

fn test_hash(context: &str, value: &[u8]) -> String {
    let mut hasher = blake3::Hasher::new_derive_key(context);
    hasher.update(&(value.len() as u64).to_be_bytes());
    hasher.update(value);
    hasher.finalize().to_hex().to_string()
}
