#[allow(dead_code)]
mod support;

use std::{collections::HashSet, sync::Arc};

use raindrop::{
    content::{
        jobs::{
            ArtifactIdentity, ArtifactIdentityInput, ArtifactKind, ContentJobOperation,
            ContentJobTrigger, ContentRepository, ContentRepositoryErrorKind, EnqueueContentJob,
            EnqueueContentJobInput, EnqueueResult, JobStatus,
        },
        provider::ProviderKind,
    },
    db::{
        entities::{content_artifact, content_job, content_job_result, entry, user},
        migrate,
    },
};
use sea_orm::{ActiveModelTrait, ActiveValue::Set, EntityTrait, PaginatorTrait};
use secrecy::SecretString;
use support::database::{
    ENTRY_A_ID, FEED_ID, HASH_A, HASH_B, HASH_C, HASH_D, SUBSCRIPTION_A_ID, USER_A_ID, USER_B_ID,
    connect_for_contract, insert_entry, insert_feed, insert_subscription, insert_user,
};
use time::{OffsetDateTime, macros::datetime};

const PROVIDER_ID: &str = "00000000-0000-4000-8000-000000000901";

#[tokio::test]
async fn enqueue_visible_entry_is_queued_and_exact_retry_is_existing() {
    let fixture = fixture("visible").await;
    let request = request("visible-key", ContentJobTrigger::ManualApi, HASH_D);

    let queued = fixture.repository.enqueue(request.clone()).await.unwrap();
    let job = expect_queued(queued);
    assert_eq!(job.status(), JobStatus::Queued);
    assert_eq!(job.attempts(), 0);
    assert_eq!(job.identity().entry_content_hash(), HASH_D);

    let existing = fixture.repository.enqueue(request).await.unwrap();
    assert_eq!(expect_existing(existing).id(), job.id());
    assert_eq!(job_count(&fixture.database).await, 1);
    assert_eq!(result_count(&fixture.database).await, 0);
}

#[tokio::test]
async fn enqueue_hides_missing_invisible_and_disabled_users() {
    let fixture = fixture("tenant").await;

    assert_error(
        fixture
            .repository
            .enqueue(request_for_user(
                "invisible",
                ContentJobTrigger::ManualApi,
                HASH_D,
                USER_B_ID,
                ENTRY_A_ID,
            ))
            .await
            .unwrap_err(),
        ContentRepositoryErrorKind::NotFound,
    );
    assert_error(
        fixture
            .repository
            .enqueue(request_for_user(
                "missing-entry",
                ContentJobTrigger::ManualApi,
                HASH_D,
                USER_A_ID,
                "00000000-0000-4000-8000-000000000399",
            ))
            .await
            .unwrap_err(),
        ContentRepositoryErrorKind::NotFound,
    );

    let mut disabled: user::ActiveModel = user::Entity::find_by_id(USER_A_ID)
        .one(&fixture.database)
        .await
        .unwrap()
        .unwrap()
        .into();
    disabled.is_disabled = Set(true);
    disabled.update(&fixture.database).await.unwrap();
    assert_error(
        fixture
            .repository
            .enqueue(request("disabled", ContentJobTrigger::ManualApi, HASH_D))
            .await
            .unwrap_err(),
        ContentRepositoryErrorKind::NotFound,
    );
    assert_eq!(job_count(&fixture.database).await, 0);
}

#[tokio::test]
async fn enqueue_rejects_entry_drift_without_writes() {
    let fixture = fixture("drift").await;
    assert_error(
        fixture
            .repository
            .enqueue(request("drift-key", ContentJobTrigger::ManualApi, HASH_A))
            .await
            .unwrap_err(),
        ContentRepositoryErrorKind::EntryChanged,
    );
    assert_eq!(job_count(&fixture.database).await, 0);
}

#[tokio::test]
async fn same_idempotency_key_with_different_request_conflicts() {
    let fixture = fixture("conflict").await;
    fixture
        .repository
        .enqueue(request(
            "conflict-key",
            ContentJobTrigger::ManualApi,
            HASH_D,
        ))
        .await
        .unwrap();

    assert_error(
        fixture
            .repository
            .enqueue(request(
                "conflict-key",
                ContentJobTrigger::ReaderSidecar,
                HASH_D,
            ))
            .await
            .unwrap_err(),
        ContentRepositoryErrorKind::IdempotencyConflict,
    );
    assert_eq!(job_count(&fixture.database).await, 1);
}

#[tokio::test]
async fn concurrent_exact_enqueue_converges_to_one_job() {
    let fixture = fixture("concurrent").await;
    let repository = Arc::new(fixture.repository);
    let requests = (0..8).map(|_| {
        let repository = Arc::clone(&repository);
        tokio::spawn(async move {
            repository
                .enqueue(request(
                    "concurrent-key",
                    ContentJobTrigger::ManualApi,
                    HASH_D,
                ))
                .await
                .map(job_id)
        })
    });
    let ids = futures_util::future::join_all(requests)
        .await
        .into_iter()
        .map(|result| result.unwrap().unwrap())
        .collect::<HashSet<_>>();
    assert_eq!(ids.len(), 1);
    assert_eq!(job_count(&fixture.database).await, 1);
}

#[tokio::test]
async fn enqueue_reuses_exact_artifact_without_creating_an_attempt() {
    let fixture = fixture("reuse").await;
    let identity = request("producer", ContentJobTrigger::ManualApi, HASH_D)
        .identity()
        .clone();
    let producer_job_id = seed_artifact(&fixture, &identity, false).await;

    let reused = fixture
        .repository
        .enqueue(request(
            "reuse-key",
            ContentJobTrigger::ReaderSidecar,
            HASH_D,
        ))
        .await
        .unwrap();
    let (job, artifact) = expect_reused(reused);
    assert_eq!(job.status(), JobStatus::Succeeded);
    assert_eq!(job.attempts(), 0);
    assert_eq!(artifact.producer_job_id(), producer_job_id);
    assert_eq!(artifact.identity(), &identity);

    let stored_result = content_job_result::Entity::find_by_id(job.id())
        .one(&fixture.database)
        .await
        .unwrap()
        .unwrap();
    assert!(stored_result.was_reused);
}

#[tokio::test]
async fn artifact_identity_hash_collision_fails_closed() {
    let fixture = fixture("collision").await;
    let identity = request("producer", ContentJobTrigger::ManualApi, HASH_D)
        .identity()
        .clone();
    seed_artifact(&fixture, &identity, true).await;

    assert_error(
        fixture
            .repository
            .enqueue(request(
                "collision-key",
                ContentJobTrigger::ReaderSidecar,
                HASH_D,
            ))
            .await
            .unwrap_err(),
        ContentRepositoryErrorKind::HashCollision,
    );
}

struct Fixture {
    repository: ContentRepository,
    database: sea_orm::DatabaseConnection,
    _data: tempfile::TempDir,
}

async fn fixture(name: &str) -> Fixture {
    let data = tempfile::tempdir().unwrap();
    let path = data.path().join(format!("content-enqueue-{name}.db"));
    let url = format!("sqlite://{}?mode=rwc", path.display());
    let database = connect_for_contract(SecretString::from(url)).await;
    migrate(&database).await.unwrap();
    let now = datetime!(2026-07-19 12:00:00 UTC);
    insert_user(&database, USER_A_ID, &format!("user-a-{name}")).await;
    insert_user(&database, USER_B_ID, &format!("user-b-{name}")).await;
    insert_feed(&database, now).await;
    insert_subscription(&database, SUBSCRIPTION_A_ID, USER_A_ID, now).await;
    insert_entry(
        &database,
        ENTRY_A_ID,
        1,
        "content-entry",
        HASH_A,
        Some(1_752_926_400_000_000),
        now,
    )
    .await;
    let stored = entry::Entity::find_by_id(ENTRY_A_ID)
        .one(&database)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.feed_id, FEED_ID);
    assert_eq!(stored.content_hash, HASH_D);
    let repository = ContentRepository::new(database.clone());
    Fixture {
        repository,
        database,
        _data: data,
    }
}

fn request(
    idempotency_key: &str,
    trigger: ContentJobTrigger,
    entry_content_hash: &str,
) -> EnqueueContentJob {
    request_for_user(
        idempotency_key,
        trigger,
        entry_content_hash,
        USER_A_ID,
        ENTRY_A_ID,
    )
}

fn request_for_user(
    idempotency_key: &str,
    trigger: ContentJobTrigger,
    entry_content_hash: &str,
    user_id: &str,
    entry_id: &str,
) -> EnqueueContentJob {
    EnqueueContentJob::new(EnqueueContentJobInput {
        operation: ContentJobOperation::Summarize,
        trigger,
        identity: ArtifactIdentity::new(ArtifactIdentityInput {
            user_id: user_id.to_owned(),
            entry_id: entry_id.to_owned(),
            kind: ArtifactKind::AiSummary,
            target_locale: None,
            entry_content_hash: entry_content_hash.to_owned(),
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
        idempotency_key: idempotency_key.to_owned(),
        call_chain_id: "manual-chain".to_owned(),
        remaining_depth: 4,
    })
    .unwrap()
}

async fn seed_artifact(fixture: &Fixture, identity: &ArtifactIdentity, collide: bool) -> String {
    let producer = fixture
        .repository
        .enqueue(request(
            "producer-job",
            ContentJobTrigger::ManualApi,
            HASH_D,
        ))
        .await
        .unwrap();
    let producer_job_id = job_id(producer);
    let now = OffsetDateTime::now_utc();
    let stored_job = content_job::Entity::find_by_id(&producer_job_id)
        .one(&fixture.database)
        .await
        .unwrap()
        .unwrap();
    let mut terminal: content_job::ActiveModel = stored_job.into();
    terminal.status = Set("SUCCEEDED".to_owned());
    terminal.completed_at = Set(Some(now));
    terminal.update(&fixture.database).await.unwrap();

    let artifact_id = "00000000-0000-4000-8000-000000001099";
    content_artifact::ActiveModel {
        id: Set(artifact_id.to_owned()),
        user_id: Set(identity.user_id().to_owned()),
        entry_id: Set(identity.entry_id().to_owned()),
        producer_job_id: Set(producer_job_id.clone()),
        kind: Set(identity.kind().as_storage().to_owned()),
        locale: Set(identity.target_locale().map(str::to_owned)),
        schema_id: Set(identity.schema_id().to_owned()),
        entry_content_hash: Set(identity.entry_content_hash().to_owned()),
        input_hash: Set(identity.input_hash().to_owned()),
        config_hash: Set(if collide {
            HASH_A.to_owned()
        } else {
            identity.config_hash().to_owned()
        }),
        processor_key: Set(identity.plugin_key().to_owned()),
        processor_version: Set(identity.plugin_version().to_owned()),
        component_digest: Set(identity.component_digest().to_owned()),
        provider_binding_id: Set(identity.provider_binding_id().to_owned()),
        provider_kind: Set(identity.provider_kind().as_storage().to_owned()),
        provider_model: Set(identity.provider_model().to_owned()),
        provider_revision: Set(i64::try_from(identity.provider_revision()).unwrap()),
        provider_label: Set("OpenAI".to_owned()),
        prompt_version: Set(identity.prompt_version().to_owned()),
        mcp_provenance_hash: Set(identity.mcp_provenance_hash().to_owned()),
        identity_hash: Set(identity.hash().to_owned()),
        payload_json: Set("{\"summary\":\"Safe\"}".to_owned()),
        provenance_json: Set("{\"schemaVersion\":1}".to_owned()),
        payload_size_bytes: Set(18),
        created_at: Set(now),
    }
    .insert(&fixture.database)
    .await
    .unwrap();
    content_job_result::ActiveModel {
        job_id: Set(producer_job_id.clone()),
        artifact_id: Set(artifact_id.to_owned()),
        was_reused: Set(false),
        linked_at: Set(now),
    }
    .insert(&fixture.database)
    .await
    .unwrap();
    producer_job_id
}

fn expect_queued(result: EnqueueResult) -> raindrop::content::jobs::JobSnapshot {
    match result {
        EnqueueResult::Queued(job) => job,
        other => panic!("expected queued result, got {other:?}"),
    }
}

fn expect_existing(result: EnqueueResult) -> raindrop::content::jobs::JobSnapshot {
    match result {
        EnqueueResult::Existing(job) => job,
        other => panic!("expected existing result, got {other:?}"),
    }
}

fn expect_reused(
    result: EnqueueResult,
) -> (
    raindrop::content::jobs::JobSnapshot,
    raindrop::content::jobs::ArtifactSnapshot,
) {
    match result {
        EnqueueResult::Reused { job, artifact } => (job, *artifact),
        other => panic!("expected reused result, got {other:?}"),
    }
}

fn job_id(result: EnqueueResult) -> String {
    match result {
        EnqueueResult::Queued(job) | EnqueueResult::Existing(job) => job.id().to_owned(),
        EnqueueResult::Reused { job, .. } => job.id().to_owned(),
    }
}

async fn job_count(database: &sea_orm::DatabaseConnection) -> u64 {
    content_job::Entity::find().count(database).await.unwrap()
}

async fn result_count(database: &sea_orm::DatabaseConnection) -> u64 {
    content_job_result::Entity::find()
        .count(database)
        .await
        .unwrap()
}

fn assert_error(
    error: raindrop::content::jobs::ContentRepositoryError,
    expected: ContentRepositoryErrorKind,
) {
    assert_eq!(error.kind(), expected);
}
