#[allow(dead_code)]
mod support;

use std::sync::Arc;

use raindrop::{
    content::{
        jobs::{
            ArtifactIdentity, ArtifactIdentityInput, ArtifactKind, ClaimContentJob, ClaimOutcome,
            ContentJobOperation, ContentJobTrigger, ContentRepository, ContentRepositoryErrorKind,
            EnqueueContentJob, EnqueueContentJobInput, EnqueueResult, JobStatus,
        },
        provider::ProviderKind,
    },
    db::{
        entities::{content_job, content_job_attempt},
        migrate,
    },
};
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, EntityTrait, QueryFilter, QueryOrder,
};
use secrecy::SecretString;
use support::database::{
    ENTRY_A_ID, HASH_A, HASH_B, HASH_C, HASH_D, SUBSCRIPTION_A_ID, SUBSCRIPTION_B_ID, USER_A_ID,
    USER_B_ID, connect_for_contract, insert_entry, insert_feed, insert_subscription, insert_user,
};
use time::{OffsetDateTime, macros::datetime};

const PROVIDER_ID: &str = "00000000-0000-4000-8000-000000000901";

#[tokio::test]
async fn claim_allocates_attempt_token_lease_and_hard_deadline() {
    let fixture = fixture("claim").await;
    let job_id = enqueue(&fixture, USER_A_ID, "claim-key").await;

    let claim = expect_claimed(
        fixture
            .repository
            .claim_next(ClaimContentJob::new("worker-a".to_owned()).unwrap())
            .await
            .unwrap(),
    );
    assert_eq!(claim.job_id(), job_id);
    assert_eq!(claim.operation(), ContentJobOperation::Summarize);
    assert_eq!(claim.trigger(), ContentJobTrigger::ManualApi);
    assert_eq!(claim.idempotency_key(), "claim-key");
    assert_eq!(claim.call_chain_id(), "chain-claim-key");
    assert_eq!(claim.remaining_depth(), 4);
    assert_eq!(claim.attempt(), 1);
    assert_eq!(claim.lease_token(), 1);
    assert!(claim.lease_until() <= claim.attempt_deadline_at());
    assert_eq!(
        claim.attempt_deadline_at() - claim.lease_until(),
        time::Duration::seconds(150)
    );
    let entry = fixture
        .repository
        .load_execution_entry(&claim)
        .await
        .unwrap();
    assert_eq!(entry.entry_id(), ENTRY_A_ID);
    assert_eq!(entry.feed_id(), support::database::FEED_ID);
    assert_eq!(entry.content_hash(), HASH_D);
    assert_eq!(entry.title(), Some("Entry 1"));
    assert_eq!(entry.text(), "Safe content");
    assert_eq!(
        entry.canonical_url(),
        Some("https://example.com/articles/1")
    );

    let stored = content_job::Entity::find_by_id(&job_id)
        .one(&fixture.database)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.status, "RUNNING");
    assert_eq!(stored.attempts, 1);
    assert_eq!(stored.lease_owner.as_deref(), Some("worker-a"));

    let attempt = content_job_attempt::Entity::find()
        .filter(content_job_attempt::Column::JobId.eq(&job_id))
        .one(&fixture.database)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(attempt.attempt, 1);
    assert_eq!(attempt.status, "RUNNING");
    assert_eq!(attempt.deadline_at, claim.attempt_deadline_at());
}

#[tokio::test]
async fn claim_orders_due_jobs_and_ignores_future_retry() {
    let fixture = fixture("due-order").await;
    let first = enqueue(&fixture, USER_A_ID, "first").await;
    let second = enqueue(&fixture, USER_A_ID, "second").await;
    let now = OffsetDateTime::now_utc();
    set_retry_wait(&fixture, &first, now + time::Duration::minutes(10)).await;
    set_retry_wait(&fixture, &second, now - time::Duration::minutes(1)).await;

    let claim = expect_claimed(
        fixture
            .repository
            .claim_next(ClaimContentJob::new("due-worker".to_owned()).unwrap())
            .await
            .unwrap(),
    );
    assert_eq!(claim.job_id(), second);

    set_retry_wait(&fixture, &second, now + time::Duration::minutes(10)).await;
    assert!(matches!(
        fixture
            .repository
            .claim_next(ClaimContentJob::new("idle-worker".to_owned()).unwrap())
            .await
            .unwrap(),
        ClaimOutcome::NoWork
    ));
}

#[tokio::test]
async fn two_workers_competing_for_one_job_get_one_claim() {
    let fixture = fixture("race").await;
    enqueue(&fixture, USER_A_ID, "race-key").await;
    let repository = Arc::new(fixture.repository.clone());
    let left = {
        let repository = Arc::clone(&repository);
        tokio::spawn(async move {
            repository
                .claim_next(ClaimContentJob::new("worker-left".to_owned()).unwrap())
                .await
                .unwrap()
        })
    };
    let right = {
        let repository = Arc::clone(&repository);
        tokio::spawn(async move {
            repository
                .claim_next(ClaimContentJob::new("worker-right".to_owned()).unwrap())
                .await
                .unwrap()
        })
    };
    let outcomes = [left.await.unwrap(), right.await.unwrap()];
    assert_eq!(
        outcomes
            .iter()
            .filter(|outcome| matches!(outcome, ClaimOutcome::Claimed(_)))
            .count(),
        1
    );
    assert_eq!(
        outcomes
            .iter()
            .filter(|outcome| matches!(outcome, ClaimOutcome::NoWork))
            .count(),
        1
    );
}

#[tokio::test]
async fn per_user_third_running_job_is_skipped_for_another_user() {
    let fixture = fixture("concurrency").await;
    for key in ["a-one", "a-two", "a-three"] {
        enqueue(&fixture, USER_A_ID, key).await;
    }

    let first = expect_claimed(claim(&fixture, "worker-1").await);
    let second = expect_claimed(claim(&fixture, "worker-2").await);
    assert_eq!(first.user_id(), USER_A_ID);
    assert_eq!(second.user_id(), USER_A_ID);

    enqueue(&fixture, USER_B_ID, "b-one").await;
    let third = expect_claimed(claim(&fixture, "worker-3").await);
    assert_eq!(third.user_id(), USER_B_ID);

    assert!(matches!(
        claim(&fixture, "worker-4").await,
        ClaimOutcome::NoWork
    ));
}

#[tokio::test]
async fn heartbeat_extends_only_to_attempt_deadline() {
    let fixture = fixture("heartbeat").await;
    let job_id = enqueue(&fixture, USER_A_ID, "heartbeat-key").await;
    let claim = expect_claimed(claim(&fixture, "heartbeat-worker").await);
    let now = OffsetDateTime::now_utc();
    let deadline = now + time::Duration::seconds(10);
    rewrite_lease(
        &fixture,
        &job_id,
        claim.lease_token(),
        now + time::Duration::seconds(5),
        deadline,
    )
    .await;

    let extended = fixture.repository.heartbeat(&claim).await.unwrap();
    assert_eq!(extended.attempt_deadline_at(), deadline);
    assert_eq!(extended.lease_until(), deadline);
    assert!(extended.remaining_attempt() > std::time::Duration::ZERO);
    assert!(extended.remaining_attempt() <= std::time::Duration::from_secs(10));
}

#[tokio::test]
async fn heartbeat_rejects_stale_owner_token_and_expired_lease() {
    let fixture = fixture("heartbeat-stale").await;
    let job_id = enqueue(&fixture, USER_A_ID, "stale-key").await;
    let claim = expect_claimed(claim(&fixture, "stale-worker").await);

    rewrite_owner(&fixture, &job_id, "different-worker").await;
    assert_kind(
        fixture.repository.heartbeat(&claim).await.unwrap_err(),
        ContentRepositoryErrorKind::LeaseLost,
    );

    rewrite_owner(&fixture, &job_id, claim.lease_owner()).await;
    rewrite_token(&fixture, &job_id, claim.lease_token() + 1).await;
    assert_kind(
        fixture.repository.heartbeat(&claim).await.unwrap_err(),
        ContentRepositoryErrorKind::LeaseLost,
    );

    rewrite_token(&fixture, &job_id, claim.lease_token()).await;
    let now = OffsetDateTime::now_utc();
    rewrite_lease(
        &fixture,
        &job_id,
        claim.lease_token(),
        now - time::Duration::seconds(2),
        now + time::Duration::seconds(20),
    )
    .await;
    assert_kind(
        fixture.repository.heartbeat(&claim).await.unwrap_err(),
        ContentRepositoryErrorKind::LeaseLost,
    );
}

#[tokio::test]
async fn expired_attempt_is_abandoned_and_reclaimed_with_new_fence() {
    let fixture = fixture("recovery").await;
    let job_id = enqueue(&fixture, USER_A_ID, "recovery-key").await;
    let first = expect_claimed(claim(&fixture, "worker-old").await);
    expire(&fixture, &job_id).await;

    let second = expect_claimed(claim(&fixture, "worker-new").await);
    assert_eq!(second.job_id(), job_id);
    assert_eq!(second.attempt(), 2);
    assert_eq!(second.lease_token(), first.lease_token() + 1);

    let attempts = attempts(&fixture, &job_id).await;
    assert_eq!(attempts.len(), 2);
    assert_eq!(attempts[0].status, "ABANDONED");
    assert_eq!(attempts[0].error_code.as_deref(), Some("JOB_LEASE_EXPIRED"));
    assert_eq!(attempts[0].retryable, Some(true));
    assert_eq!(attempts[1].status, "RUNNING");
}

#[tokio::test]
async fn expired_third_attempt_terminalizes_without_attempt_four() {
    let fixture = fixture("exhausted").await;
    let job_id = enqueue(&fixture, USER_A_ID, "exhausted-key").await;

    for expected in 1..=3 {
        let current = expect_claimed(claim(&fixture, &format!("worker-{expected}")).await);
        assert_eq!(current.attempt(), expected);
        expire(&fixture, &job_id).await;
    }

    let terminal = match claim(&fixture, "recovery-terminal").await {
        ClaimOutcome::RecoveredTerminal(job) => job,
        other => panic!("expected recovered terminal, got {other:?}"),
    };
    assert_eq!(terminal.status(), JobStatus::Failed);
    assert_eq!(terminal.attempts(), 3);
    assert_eq!(terminal.last_error_code(), Some("JOB_ATTEMPTS_EXHAUSTED"));
    assert_eq!(attempts(&fixture, &job_id).await.len(), 3);
}

struct Fixture {
    repository: ContentRepository,
    database: sea_orm::DatabaseConnection,
    _data: tempfile::TempDir,
}

async fn fixture(name: &str) -> Fixture {
    let data = tempfile::tempdir().unwrap();
    let url = format!(
        "sqlite://{}?mode=rwc",
        data.path()
            .join(format!("content-claims-{name}.db"))
            .display()
    );
    let database = connect_for_contract(SecretString::from(url)).await;
    migrate(&database).await.unwrap();
    let now = datetime!(2026-07-19 12:00:00 UTC);
    insert_user(&database, USER_A_ID, &format!("claim-a-{name}")).await;
    insert_user(&database, USER_B_ID, &format!("claim-b-{name}")).await;
    insert_feed(&database, now).await;
    insert_subscription(&database, SUBSCRIPTION_A_ID, USER_A_ID, now).await;
    insert_subscription(&database, SUBSCRIPTION_B_ID, USER_B_ID, now).await;
    insert_entry(
        &database,
        ENTRY_A_ID,
        1,
        "claim-entry",
        HASH_A,
        Some(1_752_926_400_000_000),
        now,
    )
    .await;
    let repository = ContentRepository::new(database.clone());
    Fixture {
        repository,
        database,
        _data: data,
    }
}

async fn enqueue(fixture: &Fixture, user_id: &str, key: &str) -> String {
    match fixture
        .repository
        .enqueue(
            EnqueueContentJob::new(EnqueueContentJobInput {
                operation: ContentJobOperation::Summarize,
                trigger: ContentJobTrigger::ManualApi,
                identity: ArtifactIdentity::new(ArtifactIdentityInput {
                    user_id: user_id.to_owned(),
                    entry_id: ENTRY_A_ID.to_owned(),
                    kind: ArtifactKind::AiSummary,
                    target_locale: None,
                    entry_content_hash: HASH_D.to_owned(),
                    input_hash: HASH_B.to_owned(),
                    config_hash: HASH_C.to_owned(),
                    plugin_key: "raindrop.ai-content".to_owned(),
                    plugin_version: "1.0.0".to_owned(),
                    component_digest: HASH_A.to_owned(),
                    provider_binding_id: PROVIDER_ID.to_owned(),
                    provider_kind: ProviderKind::OpenAiResponses,
                    provider_model: "gpt-5-mini".to_owned(),
                    provider_revision: 0,
                    prompt_version: "summary-v1".to_owned(),
                    schema_id: "raindrop://schemas/artifacts/ai-summary/v1".to_owned(),
                    mcp_provenance_hash: HASH_A.to_owned(),
                })
                .unwrap(),
                idempotency_key: key.to_owned(),
                call_chain_id: format!("chain-{key}"),
                remaining_depth: 4,
            })
            .unwrap(),
        )
        .await
        .unwrap()
    {
        EnqueueResult::Queued(job) => job.id().to_owned(),
        other => panic!("expected queued job, got {other:?}"),
    }
}

async fn claim(fixture: &Fixture, owner: &str) -> ClaimOutcome {
    fixture
        .repository
        .claim_next(ClaimContentJob::new(owner.to_owned()).unwrap())
        .await
        .unwrap()
}

fn expect_claimed(outcome: ClaimOutcome) -> raindrop::content::jobs::ContentJobClaim {
    match outcome {
        ClaimOutcome::Claimed(claim) => claim,
        other => panic!("expected claimed outcome, got {other:?}"),
    }
}

async fn set_retry_wait(fixture: &Fixture, job_id: &str, next: OffsetDateTime) {
    let stored = content_job::Entity::find_by_id(job_id)
        .one(&fixture.database)
        .await
        .unwrap()
        .unwrap();
    let mut active: content_job::ActiveModel = stored.into();
    active.status = Set("RETRY_WAIT".to_owned());
    active.next_attempt_at = Set(next);
    active.lease_owner = Set(None);
    active.lease_until = Set(None);
    active.attempt_deadline_at = Set(None);
    active.update(&fixture.database).await.unwrap();
}

async fn rewrite_token(fixture: &Fixture, job_id: &str, token: i64) {
    let stored = content_job::Entity::find_by_id(job_id)
        .one(&fixture.database)
        .await
        .unwrap()
        .unwrap();
    let mut active: content_job::ActiveModel = stored.into();
    active.lease_token = Set(token);
    active.update(&fixture.database).await.unwrap();
}

async fn rewrite_owner(fixture: &Fixture, job_id: &str, owner: &str) {
    let stored = content_job::Entity::find_by_id(job_id)
        .one(&fixture.database)
        .await
        .unwrap()
        .unwrap();
    let mut active: content_job::ActiveModel = stored.into();
    active.lease_owner = Set(Some(owner.to_owned()));
    active.update(&fixture.database).await.unwrap();
}

async fn rewrite_lease(
    fixture: &Fixture,
    job_id: &str,
    token: i64,
    lease_until: OffsetDateTime,
    deadline: OffsetDateTime,
) {
    let stored = content_job::Entity::find_by_id(job_id)
        .one(&fixture.database)
        .await
        .unwrap()
        .unwrap();
    let mut active: content_job::ActiveModel = stored.into();
    active.lease_token = Set(token);
    active.lease_until = Set(Some(lease_until));
    active.attempt_deadline_at = Set(Some(deadline));
    active.update(&fixture.database).await.unwrap();
}

async fn expire(fixture: &Fixture, job_id: &str) {
    let now = OffsetDateTime::now_utc();
    let stored = content_job::Entity::find_by_id(job_id)
        .one(&fixture.database)
        .await
        .unwrap()
        .unwrap();
    let mut active: content_job::ActiveModel = stored.into();
    active.lease_until = Set(Some(now - time::Duration::seconds(2)));
    active.update(&fixture.database).await.unwrap();
}

async fn attempts(fixture: &Fixture, job_id: &str) -> Vec<content_job_attempt::Model> {
    content_job_attempt::Entity::find()
        .filter(content_job_attempt::Column::JobId.eq(job_id))
        .order_by_asc(content_job_attempt::Column::Attempt)
        .all(&fixture.database)
        .await
        .unwrap()
}

fn assert_kind(
    error: raindrop::content::jobs::ContentRepositoryError,
    expected: ContentRepositoryErrorKind,
) {
    assert_eq!(error.kind(), expected);
}
