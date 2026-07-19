#[allow(dead_code)]
mod support;

use std::sync::Arc;

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use raindrop::{
    content::{
        ai::{AiAvailability, AiContentService, AiContentServiceErrorKind, AiOperationState},
        jobs::{
            ContentJobOperation, ContentRepository, ContentRepositoryErrorKind, EnqueueResult,
            JobStatus,
        },
        provider::{
            CreateProvider, ProviderCapabilities, ProviderKind, ProviderPolicy, ProviderRepository,
            ProviderScope, ProviderSecretKeyring, UpdateProvider,
        },
        worker::{ContentRuntimeHandle, official_ai_contract},
    },
    db::{
        entities::{content_artifact, content_job, content_job_result, entry},
        migrate,
    },
    plugins::{PluginRegistryRepository, SummaryArtifact},
};
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, DatabaseConnection, EntityTrait,
    PaginatorTrait, QueryFilter, sea_query::Expr,
};
use secrecy::SecretString;
use serde_json::json;
use support::{
    database::{
        ENTRY_A_ID, FEED_ID, HASH_D, SUBSCRIPTION_A_ID, USER_A_ID, USER_B_ID, connect_for_contract,
        insert_entry, insert_feed, insert_subscription, insert_user,
    },
    plugin::signed_bundle,
};
use time::{OffsetDateTime, macros::datetime};

struct ServiceFixture {
    _data: tempfile::TempDir,
    database: DatabaseConnection,
    keyring: Arc<ProviderSecretKeyring>,
    provider_id: String,
}

impl ServiceFixture {
    async fn new(name: &str) -> Self {
        let data = tempfile::tempdir().expect("temporary AI service directory");
        let url = format!(
            "sqlite://{}?mode=rwc",
            data.path().join(format!("ai-service-{name}.db")).display()
        );
        let database = connect_for_contract(SecretString::from(url)).await;
        migrate(&database)
            .await
            .expect("AI service database should migrate");
        let now = datetime!(2026-07-19 12:00:00 UTC);
        insert_user(&database, USER_A_ID, &format!("ai-service-a-{name}")).await;
        insert_user(&database, USER_B_ID, &format!("ai-service-b-{name}")).await;
        insert_feed(&database, now).await;
        insert_subscription(&database, SUBSCRIPTION_A_ID, USER_A_ID, now).await;
        insert_entry(
            &database,
            ENTRY_A_ID,
            1,
            "ai-service-entry",
            HASH_D,
            Some(1_752_926_400_000_000),
            now,
        )
        .await;

        let registry = PluginRegistryRepository::new(database.clone());
        registry
            .sync_bundled(&signed_bundle("1.0.0", b"AI service component"))
            .await
            .expect("official AI plugin should install");
        let keyring = Arc::new(provider_keyring());
        let provider_repository =
            ProviderRepository::new(database.clone(), Some(Arc::clone(&keyring)));
        let provider = provider_repository
            .create(provider_input())
            .await
            .expect("AI service provider should create");
        registry
            .replace_ai_config(
                "raindrop.ai-content",
                USER_A_ID,
                None,
                true,
                &config_json(provider.id()),
            )
            .await
            .expect("AI service config should create");
        Self {
            _data: data,
            database,
            keyring,
            provider_id: provider.id().to_owned(),
        }
    }

    fn service(&self, with_keyring: bool) -> AiContentService {
        AiContentService::new(
            self.database.clone(),
            with_keyring.then(|| Arc::clone(&self.keyring)),
            ContentRuntimeHandle::inert(),
        )
    }
}

#[test]
fn shared_operation_contract_is_exact_for_summary_and_translation() {
    let summary = official_ai_contract(ContentJobOperation::Summarize);
    assert_eq!(summary.plugin_key, "raindrop.ai-content");
    assert_eq!(summary.prompt_version, "raindrop-summary-v1");
    assert_eq!(
        summary.schema_id,
        "raindrop://schemas/artifacts/ai-summary/v1"
    );
    let translation = official_ai_contract(ContentJobOperation::Translate);
    assert_eq!(translation.prompt_version, "raindrop-translation-v1");
    assert_eq!(
        translation.schema_id,
        "raindrop://schemas/artifacts/ai-translation/v1"
    );
}

#[tokio::test]
async fn execution_projection_is_tenant_scoped_typed_and_bounded() {
    let fixture = ServiceFixture::new("projection").await;
    let repository = ContentRepository::new(fixture.database.clone());
    let entry = repository
        .get_execution_entry_for_user(USER_A_ID, ENTRY_A_ID)
        .await
        .expect("visible execution entry should load");
    assert_eq!(entry.entry_id(), ENTRY_A_ID);
    assert_eq!(entry.feed_id(), FEED_ID);
    assert_eq!(entry.content_hash(), HASH_D);
    assert_eq!(entry.title(), Some("Entry 1"));
    assert_eq!(entry.text(), "Safe content");
    assert_eq!(
        entry.canonical_url(),
        Some("https://example.com/articles/1")
    );
    assert_eq!(
        repository
            .get_execution_entry_for_user(USER_B_ID, ENTRY_A_ID)
            .await
            .expect_err("cross-user execution entry should be hidden")
            .kind(),
        ContentRepositoryErrorKind::NotFound
    );

    set_entry_html(&fixture.database, &"x".repeat(512 * 1024)).await;
    assert_eq!(
        repository
            .get_execution_entry_for_user(USER_A_ID, ENTRY_A_ID)
            .await
            .expect("exact execution text limit should load")
            .text()
            .len(),
        512 * 1024
    );
    set_entry_html(&fixture.database, &"x".repeat(512 * 1024 + 1)).await;
    assert_eq!(
        repository
            .get_execution_entry_for_user(USER_A_ID, ENTRY_A_ID)
            .await
            .expect_err("execution text over the limit should fail")
            .kind(),
        ContentRepositoryErrorKind::ExecutionInputTooLarge
    );

    let reader_openapi = std::fs::read_to_string("docs/openapi/reader-v1.json")
        .expect("Reader OpenAPI should exist");
    assert!(!reader_openapi.contains("\"contentHash\""));
}

#[tokio::test]
async fn latest_identity_job_is_deterministic_and_corruption_fails_closed() {
    let fixture = ServiceFixture::new("latest-job").await;
    let service = fixture.service(true);
    let first = expect_queued(
        service
            .enqueue(
                USER_A_ID,
                ENTRY_A_ID,
                ContentJobOperation::Summarize,
                None,
                "latest-first",
            )
            .await
            .expect("first identity job should enqueue"),
    );
    let second = expect_queued(
        service
            .enqueue(
                USER_A_ID,
                ENTRY_A_ID,
                ContentJobOperation::Summarize,
                None,
                "latest-second",
            )
            .await
            .expect("second identity job should enqueue"),
    );
    set_job_created_at(
        &fixture.database,
        first.id(),
        datetime!(2026-07-19 12:00:01 UTC),
    )
    .await;
    set_job_created_at(
        &fixture.database,
        second.id(),
        datetime!(2026-07-19 12:00:02 UTC),
    )
    .await;
    let repository = ContentRepository::new(fixture.database.clone());
    assert_eq!(
        repository
            .find_latest_job_by_identity(USER_A_ID, second.identity())
            .await
            .expect("latest identity job should load")
            .expect("latest identity job should exist")
            .id(),
        second.id()
    );

    content_job::Entity::update_many()
        .col_expr(
            content_job::Column::ProviderModel,
            Expr::value("corrupt-model"),
        )
        .filter(content_job::Column::Id.eq(second.id()))
        .exec(&fixture.database)
        .await
        .expect("job identity should corrupt");
    assert_eq!(
        repository
            .find_latest_job_by_identity(USER_A_ID, first.identity())
            .await
            .expect_err("identity corruption should fail closed")
            .kind(),
        ContentRepositoryErrorKind::CorruptData
    );
}

#[tokio::test]
async fn overview_and_enqueue_use_the_current_complete_identity() {
    let fixture = ServiceFixture::new("overview-enqueue").await;
    let service = fixture.service(true);
    let initial = service
        .overview(USER_A_ID, ENTRY_A_ID, None)
        .await
        .expect("AI overview should load");
    assert_eq!(initial.availability(), AiAvailability::Ready);
    assert_eq!(initial.summary().state(), AiOperationState::Idle);
    assert_eq!(initial.translation().state(), AiOperationState::Idle);
    assert_eq!(initial.translation().target_locale(), Some("zh-CN"));
    assert_eq!(
        service
            .overview(USER_A_ID, ENTRY_A_ID, Some("not_a_locale"))
            .await
            .expect_err("invalid translation locale should fail")
            .kind(),
        AiContentServiceErrorKind::InvalidInput
    );

    let queued = expect_queued(
        service
            .enqueue(
                USER_A_ID,
                ENTRY_A_ID,
                ContentJobOperation::Summarize,
                None,
                "reader:overview-enqueue",
            )
            .await
            .expect("summary should enqueue"),
    );
    let identity = queued.identity();
    assert_eq!(identity.user_id(), USER_A_ID);
    assert_eq!(identity.entry_id(), ENTRY_A_ID);
    assert_eq!(identity.entry_content_hash(), HASH_D);
    assert_eq!(identity.plugin_key(), "raindrop.ai-content");
    assert_eq!(identity.plugin_version(), "1.0.0");
    assert_eq!(identity.provider_binding_id(), fixture.provider_id);
    assert_eq!(identity.provider_revision(), 0);
    assert_eq!(identity.prompt_version(), "raindrop-summary-v1");
    assert_eq!(
        identity.schema_id(),
        "raindrop://schemas/artifacts/ai-summary/v1"
    );
    assert_eq!(identity.target_locale(), None);

    let existing = service
        .enqueue(
            USER_A_ID,
            ENTRY_A_ID,
            ContentJobOperation::Summarize,
            None,
            "reader:overview-enqueue",
        )
        .await
        .expect("exact enqueue retry should be idempotent");
    assert!(matches!(existing, EnqueueResult::Existing(ref job) if job.id() == queued.id()));
    assert_eq!(job_count(&fixture.database).await, 1);
    assert_eq!(
        service
            .overview(USER_A_ID, ENTRY_A_ID, None)
            .await
            .expect("queued overview should load")
            .summary()
            .state(),
        AiOperationState::Queued
    );
    assert_eq!(
        service
            .overview(USER_B_ID, ENTRY_A_ID, None)
            .await
            .expect_err("cross-user entry should be hidden")
            .kind(),
        AiContentServiceErrorKind::NotFound
    );
}

#[tokio::test]
async fn missing_keyring_blocks_new_work_but_reuses_current_artifact() {
    let fixture = ServiceFixture::new("keyring-reuse").await;
    let without_keyring = fixture.service(false);
    assert_eq!(
        without_keyring
            .enqueue(
                USER_A_ID,
                ENTRY_A_ID,
                ContentJobOperation::Summarize,
                None,
                "reader:no-keyring",
            )
            .await
            .expect_err("missing keyring should block new work")
            .kind(),
        AiContentServiceErrorKind::KeyringUnavailable
    );
    assert_eq!(job_count(&fixture.database).await, 0);

    let with_keyring = fixture.service(true);
    let producer = expect_queued(
        with_keyring
            .enqueue(
                USER_A_ID,
                ENTRY_A_ID,
                ContentJobOperation::Summarize,
                None,
                "reader:artifact-producer",
            )
            .await
            .expect("artifact producer should enqueue"),
    );
    seed_summary_artifact(&fixture.database, &producer).await;

    let reused = without_keyring
        .enqueue(
            USER_A_ID,
            ENTRY_A_ID,
            ContentJobOperation::Summarize,
            None,
            "reader:artifact-reuse",
        )
        .await
        .expect("current artifact should reuse without keyring");
    assert!(matches!(reused, EnqueueResult::Reused { .. }));
}

#[tokio::test]
async fn manual_retry_creates_a_new_job_from_the_current_snapshot() {
    let fixture = ServiceFixture::new("manual-retry").await;
    let service = fixture.service(true);
    let old = expect_queued(
        service
            .enqueue(
                USER_A_ID,
                ENTRY_A_ID,
                ContentJobOperation::Summarize,
                None,
                "reader:failed-old",
            )
            .await
            .expect("old job should enqueue"),
    );
    mark_job_failed(&fixture.database, old.id()).await;
    let providers =
        ProviderRepository::new(fixture.database.clone(), Some(Arc::clone(&fixture.keyring)));
    let updated = providers
        .update(
            &fixture.provider_id,
            &ProviderScope::user(USER_A_ID).unwrap(),
            UpdateProvider {
                expected_revision: 0,
                display_name: Some("Updated provider".to_owned()),
                ..UpdateProvider::default()
            },
        )
        .await
        .expect("provider metadata should update");
    assert_eq!(updated.revision(), 1);

    let retried = expect_queued(
        service
            .retry(USER_A_ID, old.id(), "reader-retry:new-snapshot")
            .await
            .expect("failed job should retry"),
    );
    assert_ne!(retried.id(), old.id());
    assert_eq!(retried.identity().provider_revision(), 1);
    assert_ne!(retried.identity().hash(), old.identity().hash());
    let repository = ContentRepository::new(fixture.database.clone());
    let old_after = repository
        .get_job(USER_A_ID, old.id())
        .await
        .expect("old job should remain");
    assert_eq!(old_after.status(), JobStatus::Failed);
    assert_eq!(old_after.attempts(), old.attempts());
    assert_eq!(
        service
            .retry(USER_A_ID, retried.id(), "reader-retry:not-failed")
            .await
            .expect_err("queued retry job should not retry")
            .kind(),
        AiContentServiceErrorKind::JobNotRetryable
    );
}

fn provider_input() -> CreateProvider {
    CreateProvider {
        scope: ProviderScope::user(USER_A_ID).unwrap(),
        display_name: "AI service provider".to_owned(),
        kind: ProviderKind::OpenAiResponses,
        endpoint: None,
        model: "gpt-test-model".to_owned(),
        credential: SecretString::from("ai-service-provider-secret-sentinel"),
        capabilities: ProviderCapabilities {
            supports_usage: true,
            supports_idempotency: true,
            supports_streaming: false,
        },
        policy: ProviderPolicy {
            max_concurrency: 2,
            requests_per_minute: Some(30),
            max_input_tokens_per_request: 128_000,
            max_output_tokens_per_request: 4_096,
            input_cost_micros_per_million_tokens: None,
            output_cost_micros_per_million_tokens: None,
            max_cost_micros_per_request: Some(250_000),
        },
        is_enabled: true,
    }
}

fn config_json(provider_id: &str) -> Vec<u8> {
    serde_json::to_vec(&json!({
        "schemaVersion": 1,
        "operations": {
            "summarize": {
                "enabled": true,
                "providerId": provider_id,
                "style": "BALANCED",
                "maxOutputTokens": 1024,
                "mcp": {
                    "mode": "DISABLED",
                    "failurePolicy": "FAIL_OPEN",
                    "maxToolCalls": 0,
                    "tools": []
                }
            },
            "translate": {
                "enabled": true,
                "providerId": provider_id,
                "defaultTargetLocale": "zh-CN",
                "maxOutputTokens": 4096,
                "mcp": {
                    "mode": "DISABLED",
                    "failurePolicy": "FAIL_OPEN",
                    "maxToolCalls": 0,
                    "tools": []
                }
            }
        },
        "automatic": {
            "enabled": false,
            "operations": ["SUMMARIZE", "TRANSLATE"],
            "allSubscribedFeeds": false,
            "feedIds": [],
            "categoryIds": []
        }
    }))
    .expect("AI service config should serialize")
}

fn provider_keyring() -> ProviderSecretKeyring {
    let key = SecretString::from(format!("primary:{}", URL_SAFE_NO_PAD.encode([0x41_u8; 32])));
    ProviderSecretKeyring::from_entries(&[key]).expect("AI service keyring should construct")
}

async fn set_entry_html(database: &DatabaseConnection, text: &str) {
    let storage = format!(
        "rdsc:v1:{}",
        serde_json::to_string(&json!({
            "html": format!("<p>{text}</p>"),
            "inertImages": []
        }))
        .expect("entry content should serialize")
    );
    entry::Entity::update_many()
        .col_expr(entry::Column::SanitizedContent, Expr::value(storage))
        .filter(entry::Column::Id.eq(ENTRY_A_ID))
        .exec(database)
        .await
        .expect("entry content should update");
}

async fn set_job_created_at(database: &DatabaseConnection, job_id: &str, at: OffsetDateTime) {
    content_job::Entity::update_many()
        .col_expr(content_job::Column::CreatedAt, Expr::value(at))
        .filter(content_job::Column::Id.eq(job_id))
        .exec(database)
        .await
        .expect("job created_at should update");
}

async fn mark_job_failed(database: &DatabaseConnection, job_id: &str) {
    let now = OffsetDateTime::now_utc();
    content_job::Entity::update_many()
        .col_expr(content_job::Column::Status, Expr::value("FAILED"))
        .col_expr(
            content_job::Column::LastErrorCode,
            Expr::value("PROVIDER_UNAVAILABLE"),
        )
        .col_expr(content_job::Column::CompletedAt, Expr::value(Some(now)))
        .filter(content_job::Column::Id.eq(job_id))
        .exec(database)
        .await
        .expect("job should become failed");
}

async fn seed_summary_artifact(
    database: &DatabaseConnection,
    producer: &raindrop::content::jobs::JobSnapshot,
) {
    mark_job_succeeded(database, producer.id()).await;
    let identity = producer.identity();
    let artifact_id = "00000000-0000-4000-8000-000000008001";
    let payload = SummaryArtifact::parse(
        br#"{"schemaVersion":1,"sourceLanguage":"en","summary":"Safe summary","bullets":[],"conclusion":null}"#,
    )
    .expect("summary artifact should parse")
    .canonical_json()
    .to_owned();
    let now = OffsetDateTime::now_utc();
    content_artifact::ActiveModel {
        id: Set(artifact_id.to_owned()),
        user_id: Set(identity.user_id().to_owned()),
        entry_id: Set(identity.entry_id().to_owned()),
        producer_job_id: Set(producer.id().to_owned()),
        kind: Set(identity.kind().as_storage().to_owned()),
        locale: Set(identity.target_locale().map(str::to_owned)),
        schema_id: Set(identity.schema_id().to_owned()),
        entry_content_hash: Set(identity.entry_content_hash().to_owned()),
        input_hash: Set(identity.input_hash().to_owned()),
        config_hash: Set(identity.config_hash().to_owned()),
        processor_key: Set(identity.plugin_key().to_owned()),
        processor_version: Set(identity.plugin_version().to_owned()),
        component_digest: Set(identity.component_digest().to_owned()),
        provider_binding_id: Set(identity.provider_binding_id().to_owned()),
        provider_kind: Set(identity.provider_kind().as_storage().to_owned()),
        provider_model: Set(identity.provider_model().to_owned()),
        provider_revision: Set(i64::try_from(identity.provider_revision()).unwrap()),
        provider_label: Set("gpt-test-model".to_owned()),
        prompt_version: Set(identity.prompt_version().to_owned()),
        mcp_provenance_hash: Set(identity.mcp_provenance_hash().to_owned()),
        identity_hash: Set(identity.hash().to_owned()),
        payload_size_bytes: Set(i32::try_from(payload.len()).unwrap()),
        payload_json: Set(payload),
        provenance_json: Set("{\"schemaVersion\":1}".to_owned()),
        created_at: Set(now),
    }
    .insert(database)
    .await
    .expect("summary artifact should insert");
    content_job_result::ActiveModel {
        job_id: Set(producer.id().to_owned()),
        artifact_id: Set(artifact_id.to_owned()),
        was_reused: Set(false),
        linked_at: Set(now),
    }
    .insert(database)
    .await
    .expect("summary job result should insert");
}

async fn mark_job_succeeded(database: &DatabaseConnection, job_id: &str) {
    let now = OffsetDateTime::now_utc();
    content_job::Entity::update_many()
        .col_expr(content_job::Column::Status, Expr::value("SUCCEEDED"))
        .col_expr(content_job::Column::CompletedAt, Expr::value(Some(now)))
        .filter(content_job::Column::Id.eq(job_id))
        .exec(database)
        .await
        .expect("job should become succeeded");
}

fn expect_queued(result: EnqueueResult) -> raindrop::content::jobs::JobSnapshot {
    match result {
        EnqueueResult::Queued(job) => job,
        other => panic!("expected queued job, got {other:?}"),
    }
}

async fn job_count(database: &DatabaseConnection) -> u64 {
    content_job::Entity::find()
        .count(database)
        .await
        .expect("content jobs should count")
}
