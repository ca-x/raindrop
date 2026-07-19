#[allow(dead_code)]
mod support;

use raindrop::{
    content::{
        jobs::{
            ArtifactCandidate, ArtifactIdentity, ArtifactIdentityInput, ArtifactKind, AttemptUsage,
            ClaimContentJob, ClaimOutcome, ContentJobOperation, ContentJobTrigger,
            ContentRepository, EnqueueContentJob, EnqueueContentJobInput, EnqueueResult, JobStatus,
        },
        provider::ProviderKind,
    },
    db::{migrate, rollback},
};
use secrecy::SecretString;
use serde_json::json;
use support::database::{
    ENTRY_A_ID, HASH_A, HASH_B, HASH_C, HASH_D, SUBSCRIPTION_A_ID, USER_A_ID, connect_for_contract,
    insert_entry, insert_feed, insert_subscription, insert_user,
};

const PROVIDER_ID: &str = "00000000-0000-4000-8000-000000000901";

#[tokio::test]
async fn sqlite_content_job_backend_contract() {
    let data = tempfile::tempdir().unwrap();
    let url = format!(
        "sqlite://{}?mode=rwc",
        data.path().join("content-backend.db").display()
    );
    backend_content_job_contract(&url).await;
}

#[tokio::test]
async fn postgres_content_job_backend_contract() {
    let Ok(url) = std::env::var("RAINDROP_TEST_POSTGRES_URL") else {
        eprintln!("postgres content job contract skipped: test database URL is not configured");
        return;
    };
    backend_content_job_contract(&url).await;
}

#[tokio::test]
async fn mysql_content_job_backend_contract() {
    let Ok(url) = std::env::var("RAINDROP_TEST_MYSQL_URL") else {
        eprintln!("mysql content job contract skipped: test database URL is not configured");
        return;
    };
    backend_content_job_contract(&url).await;
}

async fn backend_content_job_contract(url: &str) {
    let database = connect_for_contract(SecretString::from(url.to_owned())).await;
    rollback(&database)
        .await
        .expect("dedicated content job contract database should reset");
    migrate(&database)
        .await
        .expect("content job migrations should apply");
    let now = time::OffsetDateTime::now_utc();
    insert_user(&database, USER_A_ID, "content-backend-user").await;
    insert_feed(&database, now).await;
    insert_subscription(&database, SUBSCRIPTION_A_ID, USER_A_ID, now).await;
    insert_entry(
        &database,
        ENTRY_A_ID,
        1,
        "content-backend-entry",
        HASH_A,
        None,
        now,
    )
    .await;

    let repository = ContentRepository::new(database.clone());
    let queued = repository
        .enqueue(request("backend-generate"))
        .await
        .expect("backend job should enqueue");
    assert!(matches!(queued, EnqueueResult::Queued(_)));
    let claim = match repository
        .claim_next(ClaimContentJob::new("backend-worker".to_owned()).unwrap())
        .await
        .expect("backend job should claim")
    {
        ClaimOutcome::Claimed(claim) => claim,
        other => panic!("expected backend claim, got {other:?}"),
    };
    let execution_entry = repository
        .load_execution_entry(&claim)
        .await
        .expect("backend execution entry should remain visible and decodable");
    assert_eq!(execution_entry.entry_id(), ENTRY_A_ID);
    assert_eq!(execution_entry.feed_id(), support::database::FEED_ID);
    assert_eq!(execution_entry.content_hash(), HASH_D);
    assert_eq!(execution_entry.title(), Some("Entry 1"));
    assert_eq!(execution_entry.text(), "Safe content");
    assert_eq!(
        execution_entry.canonical_url(),
        Some("https://example.com/articles/1")
    );
    let completed = repository
        .complete_success(
            &claim,
            ArtifactCandidate::new(
                claim.identity().clone(),
                "Backend model".to_owned(),
                json!({"summary": "Backend safe"}),
                json!({"schemaVersion": 1}),
            )
            .unwrap(),
            AttemptUsage::empty(),
        )
        .await
        .expect("backend artifact should commit");
    assert!(!completed.was_reused());
    assert_eq!(
        repository
            .get_job(USER_A_ID, claim.job_id())
            .await
            .unwrap()
            .status(),
        JobStatus::Succeeded
    );

    let reused = repository
        .enqueue(request("backend-reuse"))
        .await
        .expect("backend artifact should reuse");
    match reused {
        EnqueueResult::Reused { job, artifact } => {
            assert_eq!(job.status(), JobStatus::Succeeded);
            assert_eq!(artifact.id(), completed.artifact().id());
        }
        other => panic!("expected backend reuse, got {other:?}"),
    }

    database.close().await.expect("database should close");
}

fn request(key: &str) -> EnqueueContentJob {
    EnqueueContentJob::new(EnqueueContentJobInput {
        operation: ContentJobOperation::Summarize,
        trigger: ContentJobTrigger::ManualApi,
        identity: ArtifactIdentity::new(ArtifactIdentityInput {
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
        .unwrap(),
        idempotency_key: key.to_owned(),
        call_chain_id: format!("chain-{key}"),
        remaining_depth: 4,
    })
    .unwrap()
}
