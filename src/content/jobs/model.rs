use std::{error::Error, fmt, time::Duration};

use serde_json::Value;
use time::OffsetDateTime;
use uuid::Uuid;

use crate::content::provider::ProviderKind;

use super::hash;

const MAX_IDEMPOTENCY_KEY_BYTES: usize = 255;
const MAX_CALL_CHAIN_BYTES: usize = 64;
const MAX_PLUGIN_KEY_BYTES: usize = 128;
const MAX_VERSION_BYTES: usize = 64;
const MAX_MODEL_BYTES: usize = 200;
const MAX_SCHEMA_ID_BYTES: usize = 255;
const MAX_PROVIDER_LABEL_BYTES: usize = 200;
const MAX_ERROR_CODE_BYTES: usize = 64;
const MAX_REMAINING_DEPTH: u8 = 4;
const MAX_PROVIDER_REQUESTS: u8 = 3;
const MAX_MCP_CALLS: u8 = 4;
pub(super) const MAX_ARTIFACT_BYTES: usize = 512 * 1024;
pub(super) const MAX_METADATA_BYTES: usize = 32 * 1024;
const MAX_ATTEMPTS: u8 = 3;
const MANUAL_TIMEOUT_SECONDS: u16 = 180;
const AUTOMATIC_TIMEOUT_SECONDS: u16 = 120;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ContentJobOperation {
    Summarize,
    Translate,
}

impl ContentJobOperation {
    #[must_use]
    pub const fn as_storage(self) -> &'static str {
        match self {
            Self::Summarize => "SUMMARIZE",
            Self::Translate => "TRANSLATE",
        }
    }

    pub fn from_storage(value: &str) -> Result<Self, ContentRepositoryError> {
        match value {
            "SUMMARIZE" => Ok(Self::Summarize),
            "TRANSLATE" => Ok(Self::Translate),
            _ => Err(ContentRepositoryError::new(
                ContentRepositoryErrorKind::CorruptData,
            )),
        }
    }

    #[must_use]
    pub const fn artifact_kind(self) -> ArtifactKind {
        match self {
            Self::Summarize => ArtifactKind::AiSummary,
            Self::Translate => ArtifactKind::AiTranslation,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ArtifactKind {
    AiSummary,
    AiTranslation,
}

impl ArtifactKind {
    #[must_use]
    pub const fn as_storage(self) -> &'static str {
        match self {
            Self::AiSummary => "AI_SUMMARY",
            Self::AiTranslation => "AI_TRANSLATION",
        }
    }

    pub fn from_storage(value: &str) -> Result<Self, ContentRepositoryError> {
        match value {
            "AI_SUMMARY" => Ok(Self::AiSummary),
            "AI_TRANSLATION" => Ok(Self::AiTranslation),
            _ => Err(ContentRepositoryError::new(
                ContentRepositoryErrorKind::CorruptData,
            )),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ContentJobTrigger {
    ManualApi,
    ReaderSidecar,
    FeedRefreshPersisted,
    McpServer,
}

impl ContentJobTrigger {
    #[must_use]
    pub const fn as_storage(self) -> &'static str {
        match self {
            Self::ManualApi => "MANUAL_API",
            Self::ReaderSidecar => "READER_SIDECAR",
            Self::FeedRefreshPersisted => "FEED_REFRESH_PERSISTED",
            Self::McpServer => "MCP_SERVER",
        }
    }

    pub fn from_storage(value: &str) -> Result<Self, ContentRepositoryError> {
        match value {
            "MANUAL_API" => Ok(Self::ManualApi),
            "READER_SIDECAR" => Ok(Self::ReaderSidecar),
            "FEED_REFRESH_PERSISTED" => Ok(Self::FeedRefreshPersisted),
            "MCP_SERVER" => Ok(Self::McpServer),
            _ => Err(ContentRepositoryError::new(
                ContentRepositoryErrorKind::CorruptData,
            )),
        }
    }

    #[must_use]
    pub const fn timeout_seconds(self) -> u16 {
        match self {
            Self::FeedRefreshPersisted => AUTOMATIC_TIMEOUT_SECONDS,
            Self::ManualApi | Self::ReaderSidecar | Self::McpServer => MANUAL_TIMEOUT_SECONDS,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum JobStatus {
    Queued,
    Running,
    RetryWait,
    Succeeded,
    Failed,
}

impl JobStatus {
    #[must_use]
    pub const fn as_storage(self) -> &'static str {
        match self {
            Self::Queued => "QUEUED",
            Self::Running => "RUNNING",
            Self::RetryWait => "RETRY_WAIT",
            Self::Succeeded => "SUCCEEDED",
            Self::Failed => "FAILED",
        }
    }

    pub fn from_storage(value: &str) -> Result<Self, ContentRepositoryError> {
        match value {
            "QUEUED" => Ok(Self::Queued),
            "RUNNING" => Ok(Self::Running),
            "RETRY_WAIT" => Ok(Self::RetryWait),
            "SUCCEEDED" => Ok(Self::Succeeded),
            "FAILED" => Ok(Self::Failed),
            _ => Err(ContentRepositoryError::new(
                ContentRepositoryErrorKind::CorruptData,
            )),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AttemptStatus {
    Running,
    Succeeded,
    Failed,
    Abandoned,
}

impl AttemptStatus {
    #[must_use]
    pub const fn as_storage(self) -> &'static str {
        match self {
            Self::Running => "RUNNING",
            Self::Succeeded => "SUCCEEDED",
            Self::Failed => "FAILED",
            Self::Abandoned => "ABANDONED",
        }
    }

    pub fn from_storage(value: &str) -> Result<Self, ContentRepositoryError> {
        match value {
            "RUNNING" => Ok(Self::Running),
            "SUCCEEDED" => Ok(Self::Succeeded),
            "FAILED" => Ok(Self::Failed),
            "ABANDONED" => Ok(Self::Abandoned),
            _ => Err(ContentRepositoryError::new(
                ContentRepositoryErrorKind::CorruptData,
            )),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ArtifactIdentityInput {
    pub user_id: String,
    pub entry_id: String,
    pub kind: ArtifactKind,
    pub target_locale: Option<String>,
    pub entry_content_hash: String,
    pub input_hash: String,
    pub config_hash: String,
    pub plugin_key: String,
    pub plugin_version: String,
    pub component_digest: String,
    pub provider_binding_id: String,
    pub provider_kind: ProviderKind,
    pub provider_model: String,
    pub provider_revision: u64,
    pub prompt_version: String,
    pub schema_id: String,
    pub mcp_provenance_hash: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ArtifactIdentity {
    user_id: String,
    entry_id: String,
    kind: ArtifactKind,
    target_locale: Option<String>,
    entry_content_hash: String,
    input_hash: String,
    config_hash: String,
    plugin_key: String,
    plugin_version: String,
    component_digest: String,
    provider_binding_id: String,
    provider_kind: ProviderKind,
    provider_model: String,
    provider_revision: u64,
    prompt_version: String,
    schema_id: String,
    mcp_provenance_hash: String,
    hash: String,
}

impl ArtifactIdentity {
    pub fn new(mut input: ArtifactIdentityInput) -> Result<Self, ContentRepositoryError> {
        validate_uuid(&input.user_id)?;
        validate_uuid(&input.entry_id)?;
        validate_hash(&input.entry_content_hash)?;
        validate_hash(&input.input_hash)?;
        validate_hash(&input.config_hash)?;
        validate_text(&input.plugin_key, MAX_PLUGIN_KEY_BYTES, true)?;
        validate_text(&input.plugin_version, MAX_VERSION_BYTES, true)?;
        validate_hash(&input.component_digest)?;
        validate_uuid(&input.provider_binding_id)?;
        validate_text(&input.provider_model, MAX_MODEL_BYTES, false)?;
        validate_text(&input.prompt_version, MAX_VERSION_BYTES, true)?;
        validate_text(&input.schema_id, MAX_SCHEMA_ID_BYTES, true)?;
        validate_hash(&input.mcp_provenance_hash)?;
        input.target_locale = normalize_optional_locale(input.kind, input.target_locale)?;
        let hash = hash::artifact_identity(&input, input.target_locale.as_deref());
        Ok(Self {
            user_id: input.user_id,
            entry_id: input.entry_id,
            kind: input.kind,
            target_locale: input.target_locale,
            entry_content_hash: input.entry_content_hash,
            input_hash: input.input_hash,
            config_hash: input.config_hash,
            plugin_key: input.plugin_key,
            plugin_version: input.plugin_version,
            component_digest: input.component_digest,
            provider_binding_id: input.provider_binding_id,
            provider_kind: input.provider_kind,
            provider_model: input.provider_model,
            provider_revision: input.provider_revision,
            prompt_version: input.prompt_version,
            schema_id: input.schema_id,
            mcp_provenance_hash: input.mcp_provenance_hash,
            hash,
        })
    }

    #[must_use]
    pub fn user_id(&self) -> &str {
        &self.user_id
    }

    #[must_use]
    pub fn entry_id(&self) -> &str {
        &self.entry_id
    }

    #[must_use]
    pub const fn kind(&self) -> ArtifactKind {
        self.kind
    }

    #[must_use]
    pub fn target_locale(&self) -> Option<&str> {
        self.target_locale.as_deref()
    }

    #[must_use]
    pub fn entry_content_hash(&self) -> &str {
        &self.entry_content_hash
    }

    #[must_use]
    pub fn input_hash(&self) -> &str {
        &self.input_hash
    }

    #[must_use]
    pub fn config_hash(&self) -> &str {
        &self.config_hash
    }

    #[must_use]
    pub fn plugin_key(&self) -> &str {
        &self.plugin_key
    }

    #[must_use]
    pub fn plugin_version(&self) -> &str {
        &self.plugin_version
    }

    #[must_use]
    pub fn component_digest(&self) -> &str {
        &self.component_digest
    }

    #[must_use]
    pub fn provider_binding_id(&self) -> &str {
        &self.provider_binding_id
    }

    #[must_use]
    pub const fn provider_kind(&self) -> ProviderKind {
        self.provider_kind
    }

    #[must_use]
    pub fn provider_model(&self) -> &str {
        &self.provider_model
    }

    #[must_use]
    pub const fn provider_revision(&self) -> u64 {
        self.provider_revision
    }

    #[must_use]
    pub fn prompt_version(&self) -> &str {
        &self.prompt_version
    }

    #[must_use]
    pub fn schema_id(&self) -> &str {
        &self.schema_id
    }

    #[must_use]
    pub fn mcp_provenance_hash(&self) -> &str {
        &self.mcp_provenance_hash
    }

    #[must_use]
    pub fn hash(&self) -> &str {
        &self.hash
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EnqueueContentJobInput {
    pub operation: ContentJobOperation,
    pub trigger: ContentJobTrigger,
    pub identity: ArtifactIdentity,
    pub idempotency_key: String,
    pub call_chain_id: String,
    pub remaining_depth: u8,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EnqueueContentJob {
    operation: ContentJobOperation,
    trigger: ContentJobTrigger,
    identity: ArtifactIdentity,
    idempotency_key: String,
    idempotency_key_hash: String,
    request_hash: String,
    call_chain_id: String,
    remaining_depth: u8,
    max_attempts: u8,
    timeout_seconds: u16,
}

impl EnqueueContentJob {
    pub fn new(input: EnqueueContentJobInput) -> Result<Self, ContentRepositoryError> {
        if input.operation.artifact_kind() != input.identity.kind()
            || input.remaining_depth > MAX_REMAINING_DEPTH
        {
            return Err(ContentRepositoryError::new(
                ContentRepositoryErrorKind::InvalidInput,
            ));
        }
        validate_visible_ascii(&input.idempotency_key, MAX_IDEMPOTENCY_KEY_BYTES)?;
        validate_visible_ascii(&input.call_chain_id, MAX_CALL_CHAIN_BYTES)?;
        let max_attempts = MAX_ATTEMPTS;
        let timeout_seconds = input.trigger.timeout_seconds();
        let idempotency_key_hash = hash::idempotency_key(&input.idempotency_key);
        let request_hash = hash::request(
            &input.identity,
            input.trigger,
            &input.call_chain_id,
            input.remaining_depth,
            max_attempts,
            timeout_seconds,
        );
        Ok(Self {
            operation: input.operation,
            trigger: input.trigger,
            identity: input.identity,
            idempotency_key: input.idempotency_key,
            idempotency_key_hash,
            request_hash,
            call_chain_id: input.call_chain_id,
            remaining_depth: input.remaining_depth,
            max_attempts,
            timeout_seconds,
        })
    }

    #[must_use]
    pub const fn operation(&self) -> ContentJobOperation {
        self.operation
    }

    #[must_use]
    pub const fn trigger(&self) -> ContentJobTrigger {
        self.trigger
    }

    #[must_use]
    pub const fn identity(&self) -> &ArtifactIdentity {
        &self.identity
    }

    #[must_use]
    pub fn idempotency_key(&self) -> &str {
        &self.idempotency_key
    }

    #[must_use]
    pub fn idempotency_key_hash(&self) -> &str {
        &self.idempotency_key_hash
    }

    #[must_use]
    pub fn request_hash(&self) -> &str {
        &self.request_hash
    }

    #[must_use]
    pub fn call_chain_id(&self) -> &str {
        &self.call_chain_id
    }

    #[must_use]
    pub const fn remaining_depth(&self) -> u8 {
        self.remaining_depth
    }

    #[must_use]
    pub const fn max_attempts(&self) -> u8 {
        self.max_attempts
    }

    #[must_use]
    pub const fn timeout_seconds(&self) -> u16 {
        self.timeout_seconds
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ArtifactCandidate {
    identity: ArtifactIdentity,
    provider_label: String,
    payload_json: String,
    provenance_json: String,
}

impl ArtifactCandidate {
    pub fn new(
        identity: ArtifactIdentity,
        provider_label: String,
        payload: Value,
        provenance: Value,
    ) -> Result<Self, ContentRepositoryError> {
        validate_text(&provider_label, MAX_PROVIDER_LABEL_BYTES, false)?;
        let payload_json = hash::canonical_json(
            payload,
            MAX_ARTIFACT_BYTES,
            ContentRepositoryErrorKind::ArtifactTooLarge,
        )?;
        let provenance_json = hash::canonical_json(
            provenance,
            MAX_METADATA_BYTES,
            ContentRepositoryErrorKind::InvalidInput,
        )?;
        Ok(Self {
            identity,
            provider_label,
            payload_json,
            provenance_json,
        })
    }

    #[must_use]
    pub const fn identity(&self) -> &ArtifactIdentity {
        &self.identity
    }

    #[must_use]
    pub fn provider_label(&self) -> &str {
        &self.provider_label
    }

    #[must_use]
    pub fn payload_json(&self) -> &str {
        &self.payload_json
    }

    #[must_use]
    pub fn provenance_json(&self) -> &str {
        &self.provenance_json
    }

    #[must_use]
    pub fn payload_size_bytes(&self) -> usize {
        self.payload_json.len()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AttemptUsage {
    provider_request_count: u8,
    mcp_call_count: u8,
    input_tokens: u64,
    output_tokens: u64,
    estimated_cost_micros: u64,
    execution_metadata_json: String,
}

impl AttemptUsage {
    pub fn new(
        provider_request_count: u8,
        mcp_call_count: u8,
        input_tokens: u64,
        output_tokens: u64,
        estimated_cost_micros: u64,
        execution_metadata: Value,
    ) -> Result<Self, ContentRepositoryError> {
        if provider_request_count > MAX_PROVIDER_REQUESTS
            || mcp_call_count > MAX_MCP_CALLS
            || [input_tokens, output_tokens, estimated_cost_micros]
                .into_iter()
                .any(|value| value > i64::MAX as u64)
        {
            return Err(ContentRepositoryError::new(
                ContentRepositoryErrorKind::InvalidInput,
            ));
        }
        let execution_metadata_json = hash::canonical_json(
            execution_metadata,
            MAX_METADATA_BYTES,
            ContentRepositoryErrorKind::InvalidInput,
        )?;
        Ok(Self {
            provider_request_count,
            mcp_call_count,
            input_tokens,
            output_tokens,
            estimated_cost_micros,
            execution_metadata_json,
        })
    }

    #[must_use]
    pub fn empty() -> Self {
        Self {
            provider_request_count: 0,
            mcp_call_count: 0,
            input_tokens: 0,
            output_tokens: 0,
            estimated_cost_micros: 0,
            execution_metadata_json: "{}".to_owned(),
        }
    }

    #[must_use]
    pub const fn provider_request_count(&self) -> u8 {
        self.provider_request_count
    }

    #[must_use]
    pub const fn mcp_call_count(&self) -> u8 {
        self.mcp_call_count
    }

    #[must_use]
    pub const fn input_tokens(&self) -> u64 {
        self.input_tokens
    }

    #[must_use]
    pub const fn output_tokens(&self) -> u64 {
        self.output_tokens
    }

    #[must_use]
    pub const fn estimated_cost_micros(&self) -> u64 {
        self.estimated_cost_micros
    }

    #[must_use]
    pub fn execution_metadata_json(&self) -> &str {
        &self.execution_metadata_json
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AttemptFailure {
    error_code: String,
    retryable: bool,
    outcome_unknown: bool,
    retry_after: Option<Duration>,
    usage: AttemptUsage,
}

impl AttemptFailure {
    pub fn new(
        error_code: String,
        retryable: bool,
        outcome_unknown: bool,
        retry_after: Option<Duration>,
        usage: AttemptUsage,
    ) -> Result<Self, ContentRepositoryError> {
        if error_code.is_empty()
            || error_code.len() > MAX_ERROR_CODE_BYTES
            || !error_code
                .bytes()
                .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_')
        {
            return Err(ContentRepositoryError::new(
                ContentRepositoryErrorKind::InvalidInput,
            ));
        }
        Ok(Self {
            error_code,
            retryable,
            outcome_unknown,
            retry_after,
            usage,
        })
    }

    #[must_use]
    pub fn error_code(&self) -> &str {
        &self.error_code
    }

    #[must_use]
    pub const fn retryable(&self) -> bool {
        self.retryable
    }

    #[must_use]
    pub const fn outcome_unknown(&self) -> bool {
        self.outcome_unknown
    }

    #[must_use]
    pub const fn retry_after(&self) -> Option<Duration> {
        self.retry_after
    }

    #[must_use]
    pub const fn usage(&self) -> &AttemptUsage {
        &self.usage
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ClaimContentJob {
    owner: String,
}

impl ClaimContentJob {
    pub fn new(owner: String) -> Result<Self, ContentRepositoryError> {
        validate_visible_ascii(&owner, 64)?;
        Ok(Self { owner })
    }

    #[must_use]
    pub fn owner(&self) -> &str {
        &self.owner
    }
}

#[derive(Clone, Eq, PartialEq)]
pub struct ContentJobClaim {
    pub(super) job_id: String,
    pub(super) user_id: String,
    pub(super) entry_id: String,
    pub(super) operation: ContentJobOperation,
    pub(super) trigger: ContentJobTrigger,
    pub(super) idempotency_key: String,
    pub(super) call_chain_id: String,
    pub(super) remaining_depth: u8,
    pub(super) attempt: u8,
    pub(super) lease_owner: String,
    pub(super) lease_token: i64,
    pub(super) lease_until: OffsetDateTime,
    pub(super) attempt_deadline_at: OffsetDateTime,
    pub(super) identity: ArtifactIdentity,
}

impl ContentJobClaim {
    #[must_use]
    pub fn job_id(&self) -> &str {
        &self.job_id
    }

    #[must_use]
    pub fn user_id(&self) -> &str {
        &self.user_id
    }

    #[must_use]
    pub fn entry_id(&self) -> &str {
        &self.entry_id
    }

    #[must_use]
    pub const fn operation(&self) -> ContentJobOperation {
        self.operation
    }

    #[must_use]
    pub const fn trigger(&self) -> ContentJobTrigger {
        self.trigger
    }

    #[must_use]
    pub fn idempotency_key(&self) -> &str {
        &self.idempotency_key
    }

    #[must_use]
    pub fn call_chain_id(&self) -> &str {
        &self.call_chain_id
    }

    #[must_use]
    pub const fn remaining_depth(&self) -> u8 {
        self.remaining_depth
    }

    #[must_use]
    pub const fn attempt(&self) -> u8 {
        self.attempt
    }

    #[must_use]
    pub fn lease_owner(&self) -> &str {
        &self.lease_owner
    }

    #[must_use]
    pub const fn lease_token(&self) -> i64 {
        self.lease_token
    }

    #[must_use]
    pub const fn lease_until(&self) -> OffsetDateTime {
        self.lease_until
    }

    #[must_use]
    pub const fn attempt_deadline_at(&self) -> OffsetDateTime {
        self.attempt_deadline_at
    }

    #[must_use]
    pub const fn identity(&self) -> &ArtifactIdentity {
        &self.identity
    }
}

impl fmt::Debug for ContentJobClaim {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ContentJobClaim")
            .field("operation", &self.operation)
            .field("trigger", &self.trigger)
            .field("attempt", &self.attempt)
            .field("lease_token", &self.lease_token)
            .field("remaining_depth", &self.remaining_depth)
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Eq, PartialEq)]
pub struct ContentExecutionEntry {
    pub(super) entry_id: String,
    pub(super) feed_id: String,
    pub(super) content_hash: String,
    pub(super) title: Option<String>,
    pub(super) text: String,
    pub(super) canonical_url: Option<String>,
}

impl ContentExecutionEntry {
    #[must_use]
    pub fn entry_id(&self) -> &str {
        &self.entry_id
    }

    #[must_use]
    pub fn feed_id(&self) -> &str {
        &self.feed_id
    }

    #[must_use]
    pub fn content_hash(&self) -> &str {
        &self.content_hash
    }

    #[must_use]
    pub fn title(&self) -> Option<&str> {
        self.title.as_deref()
    }

    #[must_use]
    pub fn text(&self) -> &str {
        &self.text
    }

    #[must_use]
    pub fn canonical_url(&self) -> Option<&str> {
        self.canonical_url.as_deref()
    }
}

impl fmt::Debug for ContentExecutionEntry {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ContentExecutionEntry")
            .field("title_bytes", &self.title.as_ref().map(String::len))
            .field("text_bytes", &self.text.len())
            .field("canonical_url_present", &self.canonical_url.is_some())
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LeaseDeadline {
    pub(super) lease_until: OffsetDateTime,
    pub(super) attempt_deadline_at: OffsetDateTime,
    pub(super) remaining_attempt: Duration,
}

impl LeaseDeadline {
    #[must_use]
    pub const fn lease_until(self) -> OffsetDateTime {
        self.lease_until
    }

    #[must_use]
    pub const fn attempt_deadline_at(self) -> OffsetDateTime {
        self.attempt_deadline_at
    }

    #[must_use]
    pub const fn remaining_attempt(self) -> Duration {
        self.remaining_attempt
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct JobSnapshot {
    pub(super) id: String,
    pub(super) operation: ContentJobOperation,
    pub(super) trigger: ContentJobTrigger,
    pub(super) identity: ArtifactIdentity,
    pub(super) status: JobStatus,
    pub(super) attempts: u8,
    pub(super) max_attempts: u8,
    pub(super) next_attempt_at: OffsetDateTime,
    pub(super) last_error_code: Option<String>,
    pub(super) created_at: OffsetDateTime,
    pub(super) started_at: Option<OffsetDateTime>,
    pub(super) completed_at: Option<OffsetDateTime>,
}

impl JobSnapshot {
    #[must_use]
    pub fn id(&self) -> &str {
        &self.id
    }

    #[must_use]
    pub const fn operation(&self) -> ContentJobOperation {
        self.operation
    }

    #[must_use]
    pub const fn trigger(&self) -> ContentJobTrigger {
        self.trigger
    }

    #[must_use]
    pub const fn identity(&self) -> &ArtifactIdentity {
        &self.identity
    }

    #[must_use]
    pub const fn status(&self) -> JobStatus {
        self.status
    }

    #[must_use]
    pub const fn attempts(&self) -> u8 {
        self.attempts
    }

    #[must_use]
    pub const fn max_attempts(&self) -> u8 {
        self.max_attempts
    }

    #[must_use]
    pub const fn next_attempt_at(&self) -> OffsetDateTime {
        self.next_attempt_at
    }

    #[must_use]
    pub fn last_error_code(&self) -> Option<&str> {
        self.last_error_code.as_deref()
    }

    #[must_use]
    pub const fn created_at(&self) -> OffsetDateTime {
        self.created_at
    }

    #[must_use]
    pub const fn started_at(&self) -> Option<OffsetDateTime> {
        self.started_at
    }

    #[must_use]
    pub const fn completed_at(&self) -> Option<OffsetDateTime> {
        self.completed_at
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AttemptSnapshot {
    pub(super) attempt: u8,
    pub(super) lease_token: i64,
    pub(super) status: AttemptStatus,
    pub(super) started_at: OffsetDateTime,
    pub(super) deadline_at: OffsetDateTime,
    pub(super) completed_at: Option<OffsetDateTime>,
    pub(super) error_code: Option<String>,
    pub(super) retryable: Option<bool>,
    pub(super) outcome_unknown: bool,
    pub(super) usage: AttemptUsage,
}

impl AttemptSnapshot {
    #[must_use]
    pub const fn attempt(&self) -> u8 {
        self.attempt
    }

    #[must_use]
    pub const fn status(&self) -> AttemptStatus {
        self.status
    }

    #[must_use]
    pub const fn lease_token(&self) -> i64 {
        self.lease_token
    }

    #[must_use]
    pub const fn started_at(&self) -> OffsetDateTime {
        self.started_at
    }

    #[must_use]
    pub const fn deadline_at(&self) -> OffsetDateTime {
        self.deadline_at
    }

    #[must_use]
    pub const fn completed_at(&self) -> Option<OffsetDateTime> {
        self.completed_at
    }

    #[must_use]
    pub fn error_code(&self) -> Option<&str> {
        self.error_code.as_deref()
    }

    #[must_use]
    pub const fn retryable(&self) -> Option<bool> {
        self.retryable
    }

    #[must_use]
    pub const fn outcome_unknown(&self) -> bool {
        self.outcome_unknown
    }

    #[must_use]
    pub const fn usage(&self) -> &AttemptUsage {
        &self.usage
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ArtifactSnapshot {
    pub(super) id: String,
    pub(super) producer_job_id: String,
    pub(super) identity: ArtifactIdentity,
    pub(super) provider_label: String,
    pub(super) payload_json: String,
    pub(super) provenance_json: String,
    pub(super) created_at: OffsetDateTime,
}

impl ArtifactSnapshot {
    #[must_use]
    pub fn id(&self) -> &str {
        &self.id
    }

    #[must_use]
    pub fn producer_job_id(&self) -> &str {
        &self.producer_job_id
    }

    #[must_use]
    pub const fn identity(&self) -> &ArtifactIdentity {
        &self.identity
    }

    #[must_use]
    pub fn provider_label(&self) -> &str {
        &self.provider_label
    }

    #[must_use]
    pub fn payload_json(&self) -> &str {
        &self.payload_json
    }

    #[must_use]
    pub fn provenance_json(&self) -> &str {
        &self.provenance_json
    }

    #[must_use]
    pub const fn created_at(&self) -> OffsetDateTime {
        self.created_at
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StoredArtifactResult {
    pub(super) artifact: ArtifactSnapshot,
    pub(super) was_reused: bool,
}

impl StoredArtifactResult {
    #[must_use]
    pub const fn artifact(&self) -> &ArtifactSnapshot {
        &self.artifact
    }

    #[must_use]
    pub const fn was_reused(&self) -> bool {
        self.was_reused
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum EnqueueResult {
    Queued(JobSnapshot),
    Reused {
        job: JobSnapshot,
        artifact: Box<ArtifactSnapshot>,
    },
    Existing(JobSnapshot),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ClaimOutcome {
    Claimed(ContentJobClaim),
    RecoveredTerminal(JobSnapshot),
    NoWork,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ContentRepositoryErrorKind {
    InvalidInput,
    NotFound,
    EntryChanged,
    IdempotencyConflict,
    HashCollision,
    NoWork,
    UserConcurrencyLimited,
    LeaseLost,
    AlreadyCompleted,
    AttemptsExhausted,
    ArtifactTooLarge,
    ExecutionInputTooLarge,
    NonCanonicalJson,
    CorruptData,
    Database,
}

pub struct ContentRepositoryError {
    kind: ContentRepositoryErrorKind,
}

impl ContentRepositoryError {
    pub(super) const fn new(kind: ContentRepositoryErrorKind) -> Self {
        Self { kind }
    }

    #[must_use]
    pub const fn kind(&self) -> ContentRepositoryErrorKind {
        self.kind
    }
}

impl fmt::Debug for ContentRepositoryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ContentRepositoryError")
            .field("kind", &self.kind)
            .finish()
    }
}

impl fmt::Display for ContentRepositoryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self.kind {
            ContentRepositoryErrorKind::InvalidInput => "content job input is invalid",
            ContentRepositoryErrorKind::NotFound => "content job resource is unavailable",
            ContentRepositoryErrorKind::EntryChanged => "content entry changed before enqueue",
            ContentRepositoryErrorKind::IdempotencyConflict => {
                "content job idempotency key conflicts"
            }
            ContentRepositoryErrorKind::HashCollision => "content job hash collision detected",
            ContentRepositoryErrorKind::NoWork => "no content job is due",
            ContentRepositoryErrorKind::UserConcurrencyLimited => {
                "content job user concurrency is limited"
            }
            ContentRepositoryErrorKind::LeaseLost => "content job lease is unavailable",
            ContentRepositoryErrorKind::AlreadyCompleted => "content job is already complete",
            ContentRepositoryErrorKind::AttemptsExhausted => "content job attempts are exhausted",
            ContentRepositoryErrorKind::ArtifactTooLarge => "content artifact is too large",
            ContentRepositoryErrorKind::ExecutionInputTooLarge => {
                "content job execution input is too large"
            }
            ContentRepositoryErrorKind::NonCanonicalJson => "content JSON is not canonical",
            ContentRepositoryErrorKind::CorruptData => "content job data is corrupt",
            ContentRepositoryErrorKind::Database => "content job database operation failed",
        })
    }
}

impl Error for ContentRepositoryError {}

fn validate_uuid(value: &str) -> Result<(), ContentRepositoryError> {
    let parsed = Uuid::parse_str(value)
        .map_err(|_| ContentRepositoryError::new(ContentRepositoryErrorKind::InvalidInput))?;
    if parsed.to_string() == value {
        Ok(())
    } else {
        Err(ContentRepositoryError::new(
            ContentRepositoryErrorKind::InvalidInput,
        ))
    }
}

fn validate_hash(value: &str) -> Result<(), ContentRepositoryError> {
    if value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        Ok(())
    } else {
        Err(ContentRepositoryError::new(
            ContentRepositoryErrorKind::InvalidInput,
        ))
    }
}

fn validate_visible_ascii(value: &str, max: usize) -> Result<(), ContentRepositoryError> {
    if !value.is_empty()
        && value.len() <= max
        && value.bytes().all(|byte| (0x21..=0x7e).contains(&byte))
    {
        Ok(())
    } else {
        Err(ContentRepositoryError::new(
            ContentRepositoryErrorKind::InvalidInput,
        ))
    }
}

fn validate_text(value: &str, max: usize, ascii: bool) -> Result<(), ContentRepositoryError> {
    if value.is_empty()
        || value.len() > max
        || value.chars().any(char::is_control)
        || (ascii && !value.is_ascii())
    {
        Err(ContentRepositoryError::new(
            ContentRepositoryErrorKind::InvalidInput,
        ))
    } else {
        Ok(())
    }
}

fn normalize_optional_locale(
    kind: ArtifactKind,
    locale: Option<String>,
) -> Result<Option<String>, ContentRepositoryError> {
    match (kind, locale) {
        (ArtifactKind::AiSummary, None) => Ok(None),
        (ArtifactKind::AiTranslation, Some(locale)) => normalize_locale(&locale).map(Some),
        _ => Err(ContentRepositoryError::new(
            ContentRepositoryErrorKind::InvalidInput,
        )),
    }
}

fn normalize_locale(value: &str) -> Result<String, ContentRepositoryError> {
    if !(2..=35).contains(&value.len()) || !value.is_ascii() {
        return Err(ContentRepositoryError::new(
            ContentRepositoryErrorKind::InvalidInput,
        ));
    }
    let parts = value.split('-').collect::<Vec<_>>();
    if parts.is_empty()
        || !(2..=8).contains(&parts[0].len())
        || !parts[0].bytes().all(|byte| byte.is_ascii_alphabetic())
        || parts.iter().any(|part| {
            part.is_empty()
                || part.len() > 8
                || !part.bytes().all(|byte| byte.is_ascii_alphanumeric())
        })
    {
        return Err(ContentRepositoryError::new(
            ContentRepositoryErrorKind::InvalidInput,
        ));
    }

    let mut normalized = Vec::with_capacity(parts.len());
    normalized.push(parts[0].to_ascii_lowercase());
    for part in &parts[1..] {
        let segment = if part.len() == 4 && part.bytes().all(|byte| byte.is_ascii_alphabetic()) {
            let mut chars = part.to_ascii_lowercase().chars().collect::<Vec<_>>();
            chars[0] = chars[0].to_ascii_uppercase();
            chars.into_iter().collect()
        } else if (part.len() == 2 && part.bytes().all(|byte| byte.is_ascii_alphabetic()))
            || (part.len() == 3 && part.bytes().all(|byte| byte.is_ascii_digit()))
        {
            part.to_ascii_uppercase()
        } else {
            part.to_ascii_lowercase()
        };
        normalized.push(segment);
    }
    Ok(normalized.join("-"))
}
