use raindrop::content::{
    jobs::{
        ArtifactCandidate, ArtifactIdentity, ArtifactIdentityInput, ArtifactKind, AttemptFailure,
        AttemptStatus, AttemptUsage, ContentJobOperation, ContentJobTrigger,
        ContentRepositoryErrorKind, EnqueueContentJob, EnqueueContentJobInput, JobStatus,
    },
    provider::ProviderKind,
};
use serde_json::json;
use std::time::Duration;

const USER_ID: &str = "00000000-0000-4000-8000-000000000001";
const ENTRY_ID: &str = "00000000-0000-4000-8000-000000000301";
const PROVIDER_ID: &str = "00000000-0000-4000-8000-000000000901";
const HASH_A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const HASH_B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
const HASH_C: &str = "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";

#[test]
fn content_job_enums_have_exact_storage_contracts() {
    for (operation, storage) in [
        (ContentJobOperation::Summarize, "SUMMARIZE"),
        (ContentJobOperation::Translate, "TRANSLATE"),
    ] {
        assert_eq!(operation.as_storage(), storage);
        assert_eq!(
            ContentJobOperation::from_storage(storage).unwrap(),
            operation
        );
    }
    for (kind, storage) in [
        (ArtifactKind::AiSummary, "AI_SUMMARY"),
        (ArtifactKind::AiTranslation, "AI_TRANSLATION"),
    ] {
        assert_eq!(kind.as_storage(), storage);
        assert_eq!(ArtifactKind::from_storage(storage).unwrap(), kind);
    }
    for (trigger, storage) in [
        (ContentJobTrigger::ManualApi, "MANUAL_API"),
        (ContentJobTrigger::ReaderSidecar, "READER_SIDECAR"),
        (
            ContentJobTrigger::FeedRefreshPersisted,
            "FEED_REFRESH_PERSISTED",
        ),
        (ContentJobTrigger::McpServer, "MCP_SERVER"),
    ] {
        assert_eq!(trigger.as_storage(), storage);
        assert_eq!(ContentJobTrigger::from_storage(storage).unwrap(), trigger);
    }
    for (status, storage) in [
        (JobStatus::Queued, "QUEUED"),
        (JobStatus::Running, "RUNNING"),
        (JobStatus::RetryWait, "RETRY_WAIT"),
        (JobStatus::Succeeded, "SUCCEEDED"),
        (JobStatus::Failed, "FAILED"),
    ] {
        assert_eq!(status.as_storage(), storage);
        assert_eq!(JobStatus::from_storage(storage).unwrap(), status);
    }
    for (status, storage) in [
        (AttemptStatus::Running, "RUNNING"),
        (AttemptStatus::Succeeded, "SUCCEEDED"),
        (AttemptStatus::Failed, "FAILED"),
        (AttemptStatus::Abandoned, "ABANDONED"),
    ] {
        assert_eq!(status.as_storage(), storage);
        assert_eq!(AttemptStatus::from_storage(storage).unwrap(), status);
    }

    for invalid in ["", "queued", "UNKNOWN"] {
        assert_kind(
            ContentJobOperation::from_storage(invalid).unwrap_err(),
            ContentRepositoryErrorKind::CorruptData,
        );
    }
}

#[test]
fn artifact_identity_is_canonical_and_complete() {
    let summary = identity(ArtifactKind::AiSummary, None);
    assert_eq!(summary.target_locale(), None);
    assert_eq!(summary.hash().len(), 64);
    assert!(summary.hash().bytes().all(|byte| byte.is_ascii_hexdigit()));

    let translated = identity(ArtifactKind::AiTranslation, Some("zh-cn"));
    assert_eq!(translated.target_locale(), Some("zh-CN"));
    assert_ne!(summary.hash(), translated.hash());

    let mut changed = identity_input(ArtifactKind::AiSummary, None);
    changed.prompt_version = "summary-v2".to_owned();
    assert_ne!(
        summary.hash(),
        ArtifactIdentity::new(changed).unwrap().hash()
    );

    let invalid = ArtifactIdentity::new(identity_input(ArtifactKind::AiTranslation, None));
    assert_kind(
        invalid.unwrap_err(),
        ContentRepositoryErrorKind::InvalidInput,
    );
}

#[test]
fn enqueue_hashes_case_sensitive_keys_and_trigger_limits() {
    let manual = enqueue("Key", ContentJobTrigger::ManualApi);
    let lower = enqueue("key", ContentJobTrigger::ManualApi);
    let automatic = enqueue("Key", ContentJobTrigger::FeedRefreshPersisted);

    assert_ne!(manual.idempotency_key_hash(), lower.idempotency_key_hash());
    assert_ne!(manual.request_hash(), automatic.request_hash());
    assert_eq!(manual.timeout_seconds(), 180);
    assert_eq!(automatic.timeout_seconds(), 120);
    assert_eq!(manual.max_attempts(), 3);
    assert_eq!(manual.remaining_depth(), 4);
}

#[test]
fn artifact_candidate_canonicalizes_json_and_enforces_bounds() {
    let left = ArtifactCandidate::new(
        identity(ArtifactKind::AiSummary, None),
        "OpenAI".to_owned(),
        json!({"b": 2, "a": 1}),
        json!({"schemaVersion": 1, "degraded": false}),
    )
    .unwrap();
    let right = ArtifactCandidate::new(
        identity(ArtifactKind::AiSummary, None),
        "OpenAI".to_owned(),
        json!({"a": 1, "b": 2}),
        json!({"degraded": false, "schemaVersion": 1}),
    )
    .unwrap();
    assert_eq!(left.payload_json(), "{\"a\":1,\"b\":2}");
    assert_eq!(left.payload_json(), right.payload_json());
    assert_eq!(left.provenance_json(), right.provenance_json());
    assert_eq!(left.payload_size_bytes(), left.payload_json().len());

    let oversized = ArtifactCandidate::new(
        identity(ArtifactKind::AiSummary, None),
        "OpenAI".to_owned(),
        json!({"summary": "x".repeat(512 * 1024)}),
        json!({}),
    );
    assert_kind(
        oversized.unwrap_err(),
        ContentRepositoryErrorKind::ArtifactTooLarge,
    );
}

#[test]
fn attempt_metrics_and_failure_codes_are_bounded() {
    let usage = AttemptUsage::new(3, 4, 100, 20, 42, json!({"schemaVersion": 1})).unwrap();
    assert_eq!(usage.provider_request_count(), 3);
    assert_eq!(usage.mcp_call_count(), 4);
    assert_eq!(usage.estimated_cost_micros(), 42);

    assert_kind(
        AttemptUsage::new(4, 0, 0, 0, 0, json!({})).unwrap_err(),
        ContentRepositoryErrorKind::InvalidInput,
    );
    assert_kind(
        AttemptUsage::new(0, 5, 0, 0, 0, json!({})).unwrap_err(),
        ContentRepositoryErrorKind::InvalidInput,
    );

    let failure = AttemptFailure::new(
        "AI_RATE_LIMITED".to_owned(),
        true,
        true,
        Some(Duration::from_secs(70)),
        usage,
    )
    .unwrap();
    assert!(failure.retryable());
    assert!(failure.outcome_unknown());
    assert_eq!(failure.retry_after(), Some(Duration::from_secs(70)));

    assert_kind(
        AttemptFailure::new(
            "unsafe message".to_owned(),
            false,
            false,
            None,
            AttemptUsage::empty(),
        )
        .unwrap_err(),
        ContentRepositoryErrorKind::InvalidInput,
    );
}

#[test]
fn invalid_identifiers_hashes_locales_and_keys_fail_closed() {
    let mut invalid_user = identity_input(ArtifactKind::AiSummary, None);
    invalid_user.user_id = "not-a-uuid".to_owned();
    assert_kind(
        ArtifactIdentity::new(invalid_user).unwrap_err(),
        ContentRepositoryErrorKind::InvalidInput,
    );

    let mut invalid_hash = identity_input(ArtifactKind::AiSummary, None);
    invalid_hash.config_hash = "ABC".to_owned();
    assert_kind(
        ArtifactIdentity::new(invalid_hash).unwrap_err(),
        ContentRepositoryErrorKind::InvalidInput,
    );

    let invalid_locale = ArtifactIdentity::new(identity_input(
        ArtifactKind::AiTranslation,
        Some("not_a_locale"),
    ));
    assert_kind(
        invalid_locale.unwrap_err(),
        ContentRepositoryErrorKind::InvalidInput,
    );

    for key in ["", "line\nbreak"] {
        let mut input = enqueue_input("valid", ContentJobTrigger::ManualApi);
        input.idempotency_key = key.to_owned();
        assert_kind(
            EnqueueContentJob::new(input).unwrap_err(),
            ContentRepositoryErrorKind::InvalidInput,
        );
    }
    let mut oversized = enqueue_input("valid", ContentJobTrigger::ManualApi);
    oversized.idempotency_key = "x".repeat(256);
    assert_kind(
        EnqueueContentJob::new(oversized).unwrap_err(),
        ContentRepositoryErrorKind::InvalidInput,
    );
}

fn enqueue(idempotency_key: &str, trigger: ContentJobTrigger) -> EnqueueContentJob {
    EnqueueContentJob::new(enqueue_input(idempotency_key, trigger)).unwrap()
}

fn enqueue_input(idempotency_key: &str, trigger: ContentJobTrigger) -> EnqueueContentJobInput {
    EnqueueContentJobInput {
        operation: ContentJobOperation::Summarize,
        trigger,
        identity: identity(ArtifactKind::AiSummary, None),
        idempotency_key: idempotency_key.to_owned(),
        call_chain_id: "manual-chain".to_owned(),
        remaining_depth: 4,
    }
}

fn identity(kind: ArtifactKind, locale: Option<&str>) -> ArtifactIdentity {
    ArtifactIdentity::new(identity_input(kind, locale)).unwrap()
}

fn identity_input(kind: ArtifactKind, locale: Option<&str>) -> ArtifactIdentityInput {
    ArtifactIdentityInput {
        user_id: USER_ID.to_owned(),
        entry_id: ENTRY_ID.to_owned(),
        kind,
        target_locale: locale.map(str::to_owned),
        entry_content_hash: HASH_A.to_owned(),
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
    }
}

fn assert_kind(error: impl ErrorKind, expected: ContentRepositoryErrorKind) {
    assert_eq!(error.kind(), expected);
}

trait ErrorKind {
    fn kind(&self) -> ContentRepositoryErrorKind;
}

impl ErrorKind for raindrop::content::jobs::ContentRepositoryError {
    fn kind(&self) -> ContentRepositoryErrorKind {
        self.kind()
    }
}
