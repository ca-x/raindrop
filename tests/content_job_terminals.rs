#[allow(dead_code)]
mod support;

use raindrop::{
    content::{
        jobs::{
            ArtifactCandidate, ArtifactIdentity, ArtifactIdentityInput, ArtifactKind,
            AttemptFailure, AttemptStatus, AttemptUsage, ClaimContentJob, ClaimOutcome,
            ContentJobOperation, ContentJobTrigger, ContentRepository, ContentRepositoryErrorKind,
            EnqueueContentJob, EnqueueContentJobInput, EnqueueResult, JobStatus,
        },
        provider::ProviderKind,
    },
    db::{
        entities::{content_artifact, content_job, content_job_attempt, content_job_result, entry},
        migrate,
    },
};
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, ConnectionTrait, DatabaseBackend, EntityTrait,
    PaginatorTrait, QueryFilter, Statement,
};
use secrecy::SecretString;
use serde_json::json;
use std::time::Duration;
use support::database::{
    ENTRY_A_ID, HASH_A, HASH_B, HASH_C, HASH_D, SUBSCRIPTION_A_ID, USER_A_ID, USER_B_ID,
    connect_for_contract, insert_entry, insert_feed, insert_subscription, insert_user,
};
use time::{OffsetDateTime, macros::datetime};

const PROVIDER_ID: &str = "00000000-0000-4000-8000-000000000901";

#[tokio::test]
async fn permanent_failure_terminalizes_attempt_and_job_with_safe_usage() {
    let fixture = fixture("permanent").await;
    let claim = enqueue_and_claim(&fixture, "permanent-key", "permanent-worker").await;
    let job = fixture
        .repository
        .complete_failure(
            &claim,
            AttemptFailure::new(
                "AI_AUTHENTICATION".to_owned(),
                false,
                false,
                None,
                AttemptUsage::new(1, 0, 100, 0, 42, json!({"schemaVersion": 1})).unwrap(),
            )
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(job.status(), JobStatus::Failed);
    assert_eq!(job.last_error_code(), Some("AI_AUTHENTICATION"));
    assert!(job.completed_at().is_some());

    let attempts = fixture
        .repository
        .list_attempts(USER_A_ID, claim.job_id())
        .await
        .unwrap();
    assert_eq!(attempts.len(), 1);
    assert_eq!(attempts[0].status(), AttemptStatus::Failed);
    assert_eq!(attempts[0].error_code(), Some("AI_AUTHENTICATION"));
    assert_eq!(attempts[0].retryable(), Some(false));
    assert_eq!(attempts[0].usage().input_tokens(), 100);
    assert_eq!(attempts[0].usage().estimated_cost_micros(), 42);
    assert_eq!(artifact_count(&fixture).await, 0);
}

#[tokio::test]
async fn retryable_failures_schedule_five_then_thirty_seconds_and_record_unknown_outcome() {
    let fixture = fixture("retry").await;
    let first = enqueue_and_claim(&fixture, "retry-key", "retry-worker-1").await;
    let job = fixture
        .repository
        .complete_failure(
            &first,
            AttemptFailure::new(
                "AI_TIMEOUT".to_owned(),
                true,
                true,
                None,
                AttemptUsage::empty(),
            )
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(job.status(), JobStatus::RetryWait);
    assert_eq!(retry_delta(&fixture, first.job_id(), 1).await, 5);
    make_due(&fixture, first.job_id()).await;

    let second = expect_claimed(claim(&fixture, "retry-worker-2").await);
    assert_eq!(second.attempt(), 2);
    let job = fixture
        .repository
        .complete_failure(
            &second,
            AttemptFailure::new(
                "AI_RATE_LIMITED".to_owned(),
                true,
                false,
                None,
                AttemptUsage::empty(),
            )
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(job.status(), JobStatus::RetryWait);
    assert_eq!(retry_delta(&fixture, second.job_id(), 2).await, 30);

    let attempts = fixture
        .repository
        .list_attempts(USER_A_ID, second.job_id())
        .await
        .unwrap();
    assert!(attempts[0].outcome_unknown());
    assert!(!attempts[1].outcome_unknown());
}

#[tokio::test]
async fn retry_after_wins_and_is_clamped_to_one_hour() {
    let fixture = fixture("retry-after").await;
    let claim = enqueue_and_claim(&fixture, "retry-after-key", "retry-after-worker").await;
    fixture
        .repository
        .complete_failure(
            &claim,
            AttemptFailure::new(
                "AI_RATE_LIMITED".to_owned(),
                true,
                false,
                Some(Duration::from_secs(2 * 60 * 60)),
                AttemptUsage::empty(),
            )
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(retry_delta(&fixture, claim.job_id(), 1).await, 60 * 60);
}

#[tokio::test]
async fn success_atomically_persists_artifact_result_attempt_and_job_without_touching_entry() {
    let fixture = fixture("success").await;
    let before = entry::Entity::find_by_id(ENTRY_A_ID)
        .one(&fixture.database)
        .await
        .unwrap()
        .unwrap();
    let claim = enqueue_and_claim(&fixture, "success-key", "success-worker").await;
    let result = fixture
        .repository
        .complete_success(
            &claim,
            candidate(&claim),
            AttemptUsage::new(1, 0, 120, 30, 55, json!({"schemaVersion": 1})).unwrap(),
        )
        .await
        .unwrap();
    assert!(!result.was_reused());
    assert_eq!(result.artifact().identity(), claim.identity());
    assert_eq!(
        result.artifact().payload_json(),
        "{\"summary\":\"Safe summary\"}"
    );

    let job = fixture
        .repository
        .get_job(USER_A_ID, claim.job_id())
        .await
        .unwrap();
    assert_eq!(job.status(), JobStatus::Succeeded);
    let attempts = fixture
        .repository
        .list_attempts(USER_A_ID, claim.job_id())
        .await
        .unwrap();
    assert_eq!(attempts[0].status(), AttemptStatus::Succeeded);
    assert_eq!(attempts[0].usage().output_tokens(), 30);
    let fetched = fixture
        .repository
        .get_result(USER_A_ID, claim.job_id())
        .await
        .unwrap();
    assert_eq!(fetched, result);

    let after = entry::Entity::find_by_id(ENTRY_A_ID)
        .one(&fixture.database)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(after.title, before.title);
    assert_eq!(after.summary, before.summary);
    assert_eq!(after.sanitized_content, before.sanitized_content);
    assert_eq!(after.content_hash, before.content_hash);
}

#[tokio::test]
async fn two_jobs_with_same_identity_share_one_immutable_artifact() {
    let fixture = fixture("reuse-race").await;
    let first_job = enqueue(&fixture, "reuse-one").await;
    let second_job = enqueue(&fixture, "reuse-two").await;
    let first = expect_claimed(claim(&fixture, "reuse-worker-1").await);
    let second = expect_claimed(claim(&fixture, "reuse-worker-2").await);
    assert!([first.job_id(), second.job_id()].contains(&first_job.as_str()));
    assert!([first.job_id(), second.job_id()].contains(&second_job.as_str()));

    let first_result = fixture
        .repository
        .complete_success(&first, candidate(&first), AttemptUsage::empty())
        .await
        .unwrap();
    let second_result = fixture
        .repository
        .complete_success(&second, candidate(&second), AttemptUsage::empty())
        .await
        .unwrap();
    assert!(!first_result.was_reused());
    assert!(second_result.was_reused());
    assert_eq!(first_result.artifact().id(), second_result.artifact().id());
    assert_eq!(artifact_count(&fixture).await, 1);
    assert_eq!(result_count(&fixture).await, 2);
}

#[tokio::test]
async fn stale_fence_cannot_commit_failure_or_artifact() {
    let fixture = fixture("stale").await;
    let claim = enqueue_and_claim(&fixture, "stale-terminal", "stale-terminal-worker").await;
    rewrite_token(&fixture, claim.job_id(), claim.lease_token() + 1).await;

    assert_kind(
        fixture
            .repository
            .complete_success(&claim, candidate(&claim), AttemptUsage::empty())
            .await
            .unwrap_err(),
        ContentRepositoryErrorKind::LeaseLost,
    );
    assert_kind(
        fixture
            .repository
            .complete_failure(
                &claim,
                AttemptFailure::new(
                    "AI_TIMEOUT".to_owned(),
                    true,
                    true,
                    None,
                    AttemptUsage::empty(),
                )
                .unwrap(),
            )
            .await
            .unwrap_err(),
        ContentRepositoryErrorKind::LeaseLost,
    );
    assert_eq!(artifact_count(&fixture).await, 0);
    assert_eq!(result_count(&fixture).await, 0);
}

#[tokio::test]
async fn job_attempt_and_result_reads_do_not_cross_tenants() {
    let fixture = fixture("tenant-read").await;
    let claim = enqueue_and_claim(&fixture, "tenant-read-key", "tenant-read-worker").await;
    fixture
        .repository
        .complete_success(&claim, candidate(&claim), AttemptUsage::empty())
        .await
        .unwrap();

    for error in [
        fixture
            .repository
            .get_job(USER_B_ID, claim.job_id())
            .await
            .unwrap_err(),
        fixture
            .repository
            .list_attempts(USER_B_ID, claim.job_id())
            .await
            .unwrap_err(),
        fixture
            .repository
            .get_result(USER_B_ID, claim.job_id())
            .await
            .unwrap_err(),
    ] {
        assert_kind(error, ContentRepositoryErrorKind::NotFound);
    }
}

#[tokio::test]
async fn result_insert_failure_rolls_back_artifact_attempt_and_job() {
    rollback_contract("result", "content_job_results", "BEFORE INSERT").await;
}

#[tokio::test]
async fn job_terminal_failure_rolls_back_artifact_result_and_attempt() {
    rollback_contract("job", "content_jobs", "BEFORE UPDATE").await;
}

async fn rollback_contract(name: &str, table: &str, timing: &str) {
    let fixture = fixture(&format!("rollback-{name}")).await;
    let claim = enqueue_and_claim(
        &fixture,
        &format!("rollback-{name}"),
        &format!("rollback-worker-{name}"),
    )
    .await;
    let trigger = if table == "content_jobs" {
        format!(
            "CREATE TRIGGER fail_{name} {timing} ON {table}
             WHEN NEW.status = 'SUCCEEDED'
             BEGIN SELECT RAISE(ABORT, 'induced terminal failure'); END"
        )
    } else {
        format!(
            "CREATE TRIGGER fail_{name} {timing} ON {table}
             BEGIN SELECT RAISE(ABORT, 'induced result failure'); END"
        )
    };
    fixture
        .database
        .execute(Statement::from_string(DatabaseBackend::Sqlite, trigger))
        .await
        .unwrap();

    assert_kind(
        fixture
            .repository
            .complete_success(&claim, candidate(&claim), AttemptUsage::empty())
            .await
            .unwrap_err(),
        ContentRepositoryErrorKind::Database,
    );
    assert_eq!(artifact_count(&fixture).await, 0);
    assert_eq!(result_count(&fixture).await, 0);
    let job = content_job::Entity::find_by_id(claim.job_id())
        .one(&fixture.database)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(job.status, "RUNNING");
    let attempt = content_job_attempt::Entity::find()
        .one(&fixture.database)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(attempt.status, "RUNNING");
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
            .join(format!("content-terminals-{name}.db"))
            .display()
    );
    let database = connect_for_contract(SecretString::from(url)).await;
    migrate(&database).await.unwrap();
    let now = datetime!(2026-07-19 12:00:00 UTC);
    insert_user(&database, USER_A_ID, &format!("terminal-{name}")).await;
    insert_user(&database, USER_B_ID, &format!("terminal-other-{name}")).await;
    insert_feed(&database, now).await;
    insert_subscription(&database, SUBSCRIPTION_A_ID, USER_A_ID, now).await;
    insert_entry(
        &database,
        ENTRY_A_ID,
        1,
        "terminal-entry",
        HASH_A,
        Some(1_752_926_400_000_000),
        now,
    )
    .await;
    Fixture {
        repository: ContentRepository::new(database.clone()),
        database,
        _data: data,
    }
}

async fn enqueue_and_claim(
    fixture: &Fixture,
    key: &str,
    owner: &str,
) -> raindrop::content::jobs::ContentJobClaim {
    let job_id = enqueue(fixture, key).await;
    let claim = expect_claimed(claim(fixture, owner).await);
    assert_eq!(claim.job_id(), job_id);
    claim
}

async fn enqueue(fixture: &Fixture, key: &str) -> String {
    let result = fixture
        .repository
        .enqueue(
            EnqueueContentJob::new(EnqueueContentJobInput {
                operation: ContentJobOperation::Summarize,
                trigger: ContentJobTrigger::ManualApi,
                identity: identity(),
                idempotency_key: key.to_owned(),
                call_chain_id: format!("chain-{key}"),
                remaining_depth: 4,
            })
            .unwrap(),
        )
        .await
        .unwrap();
    match result {
        EnqueueResult::Queued(job) => job.id().to_owned(),
        other => panic!("expected queued result, got {other:?}"),
    }
}

fn identity() -> ArtifactIdentity {
    ArtifactIdentity::new(ArtifactIdentityInput {
        user_id: USER_A_ID.to_owned(),
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
    .unwrap()
}

fn candidate(claim: &raindrop::content::jobs::ContentJobClaim) -> ArtifactCandidate {
    ArtifactCandidate::new(
        claim.identity().clone(),
        "OpenAI gpt-5-mini".to_owned(),
        json!({"summary": "Safe summary"}),
        json!({"schemaVersion": 1, "degraded": false}),
    )
    .unwrap()
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

async fn make_due(fixture: &Fixture, job_id: &str) {
    let stored = content_job::Entity::find_by_id(job_id)
        .one(&fixture.database)
        .await
        .unwrap()
        .unwrap();
    let mut active: content_job::ActiveModel = stored.into();
    active.next_attempt_at = Set(OffsetDateTime::now_utc() - time::Duration::seconds(1));
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

async fn retry_delta(fixture: &Fixture, job_id: &str, attempt: i32) -> i64 {
    let job = content_job::Entity::find_by_id(job_id)
        .one(&fixture.database)
        .await
        .unwrap()
        .unwrap();
    let stored_attempt = content_job_attempt::Entity::find()
        .filter(content_job_attempt::Column::JobId.eq(job_id))
        .filter(content_job_attempt::Column::Attempt.eq(attempt))
        .one(&fixture.database)
        .await
        .unwrap()
        .unwrap();
    (job.next_attempt_at - stored_attempt.completed_at.unwrap()).whole_seconds()
}

async fn artifact_count(fixture: &Fixture) -> u64 {
    content_artifact::Entity::find()
        .count(&fixture.database)
        .await
        .unwrap()
}

async fn result_count(fixture: &Fixture) -> u64 {
    content_job_result::Entity::find()
        .count(&fixture.database)
        .await
        .unwrap()
}

fn assert_kind(
    error: raindrop::content::jobs::ContentRepositoryError,
    expected: ContentRepositoryErrorKind,
) {
    assert_eq!(error.kind(), expected);
}
