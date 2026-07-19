use std::{error::Error, fmt, sync::Arc};

use sea_orm::DatabaseConnection;

use crate::{
    content::{
        jobs::{
            ArtifactIdentity, ArtifactIdentityInput, ArtifactSnapshot, ContentExecutionEntry,
            ContentJobOperation, ContentJobTrigger, ContentRepository, ContentRepositoryError,
            ContentRepositoryErrorKind, EnqueueContentJob, EnqueueContentJobInput, EnqueueResult,
            JobSnapshot, JobStatus,
        },
        provider::{
            ProviderCoreError, ProviderCoreErrorKind, ProviderRepository, ProviderSecretKeyring,
        },
        worker::{
            ContentInvocationInput, ContentRuntimeHandle, OFFICIAL_AI_PLUGIN_KEY,
            disabled_mcp_provenance_hash, official_ai_contract,
        },
    },
    plugins::{
        PluginConfig, PluginInstallation, PluginRegistryError, PluginRegistryErrorKind,
        PluginRegistryRepository, PluginSystemState, json::normalize_locale,
    },
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AiAvailability {
    Ready,
    NotConfigured,
    Disabled,
    ProviderUnavailable,
    PluginUnavailable,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AiOperationState {
    Unavailable,
    Disabled,
    Idle,
    Queued,
    Running,
    RetryWait,
    Succeeded,
    Failed,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AiOperationOverview {
    operation: ContentJobOperation,
    target_locale: Option<String>,
    state: AiOperationState,
    job: Option<JobSnapshot>,
    artifact: Option<ArtifactSnapshot>,
}

impl AiOperationOverview {
    #[must_use]
    pub const fn operation(&self) -> ContentJobOperation {
        self.operation
    }

    #[must_use]
    pub fn target_locale(&self) -> Option<&str> {
        self.target_locale.as_deref()
    }

    #[must_use]
    pub const fn state(&self) -> AiOperationState {
        self.state
    }

    #[must_use]
    pub const fn job(&self) -> Option<&JobSnapshot> {
        self.job.as_ref()
    }

    #[must_use]
    pub const fn artifact(&self) -> Option<&ArtifactSnapshot> {
        self.artifact.as_ref()
    }

    fn unavailable(operation: ContentJobOperation, target_locale: Option<String>) -> Self {
        Self {
            operation,
            target_locale,
            state: AiOperationState::Unavailable,
            job: None,
            artifact: None,
        }
    }

    fn disabled(operation: ContentJobOperation, target_locale: Option<String>) -> Self {
        Self {
            operation,
            target_locale,
            state: AiOperationState::Disabled,
            job: None,
            artifact: None,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AiEntryOverview {
    availability: AiAvailability,
    summary: AiOperationOverview,
    translation: AiOperationOverview,
}

impl AiEntryOverview {
    #[must_use]
    pub const fn availability(&self) -> AiAvailability {
        self.availability
    }

    #[must_use]
    pub const fn summary(&self) -> &AiOperationOverview {
        &self.summary
    }

    #[must_use]
    pub const fn translation(&self) -> &AiOperationOverview {
        &self.translation
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AiContentServiceErrorKind {
    InvalidInput,
    NotFound,
    EntryChanged,
    NotConfigured,
    Disabled,
    ProviderUnavailable,
    PluginUnavailable,
    KeyringUnavailable,
    IdempotencyConflict,
    JobNotRetryable,
    CorruptData,
    Database,
}

pub struct AiContentServiceError {
    kind: AiContentServiceErrorKind,
}

impl AiContentServiceError {
    const fn new(kind: AiContentServiceErrorKind) -> Self {
        Self { kind }
    }

    #[must_use]
    pub const fn kind(&self) -> AiContentServiceErrorKind {
        self.kind
    }
}

impl fmt::Debug for AiContentServiceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AiContentServiceError")
            .field("kind", &self.kind)
            .finish()
    }
}

impl fmt::Display for AiContentServiceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self.kind {
            AiContentServiceErrorKind::InvalidInput => "AI content input is invalid",
            AiContentServiceErrorKind::NotFound => "AI content resource is unavailable",
            AiContentServiceErrorKind::EntryChanged => "AI content entry changed",
            AiContentServiceErrorKind::NotConfigured => "AI content is not configured",
            AiContentServiceErrorKind::Disabled => "AI content is disabled",
            AiContentServiceErrorKind::ProviderUnavailable => "AI provider is unavailable",
            AiContentServiceErrorKind::PluginUnavailable => "AI plugin is unavailable",
            AiContentServiceErrorKind::KeyringUnavailable => "AI provider keyring is unavailable",
            AiContentServiceErrorKind::IdempotencyConflict => {
                "AI content idempotency key conflicts"
            }
            AiContentServiceErrorKind::JobNotRetryable => "AI content job is not retryable",
            AiContentServiceErrorKind::CorruptData => "AI content stored data is corrupt",
            AiContentServiceErrorKind::Database => "AI content database operation failed",
        })
    }
}

impl Error for AiContentServiceError {}

pub struct AiContentService {
    content: ContentRepository,
    plugins: PluginRegistryRepository,
    providers: ProviderRepository,
    keyring_available: bool,
    runtime: ContentRuntimeHandle,
}

impl AiContentService {
    #[must_use]
    pub fn new(
        database: DatabaseConnection,
        provider_keyring: Option<Arc<ProviderSecretKeyring>>,
        runtime: ContentRuntimeHandle,
    ) -> Self {
        let keyring_available = provider_keyring.is_some();
        Self {
            content: ContentRepository::new(database.clone()),
            plugins: PluginRegistryRepository::new(database.clone()),
            providers: ProviderRepository::new(database, provider_keyring),
            keyring_available,
            runtime,
        }
    }

    pub async fn overview(
        &self,
        user_id: &str,
        entry_id: &str,
        translation_locale: Option<&str>,
    ) -> Result<AiEntryOverview, AiContentServiceError> {
        let requested_locale = normalize_requested_locale(translation_locale)?;
        let entry = self
            .content
            .get_execution_entry_for_user(user_id, entry_id)
            .await
            .map_err(map_content_error)?;
        let installation = match self.plugins.get_installation(OFFICIAL_AI_PLUGIN_KEY).await {
            Ok(installation) if installation.system_state() == PluginSystemState::Enabled => {
                installation
            }
            Ok(_) => {
                return Ok(unavailable_overview(
                    AiAvailability::PluginUnavailable,
                    requested_locale,
                ));
            }
            Err(error) if error.kind() == PluginRegistryErrorKind::NotFound => {
                return Ok(unavailable_overview(
                    AiAvailability::PluginUnavailable,
                    requested_locale,
                ));
            }
            Err(error) => return Err(map_plugin_error(error)),
        };
        let Some(config) = self
            .plugins
            .get_ai_config(OFFICIAL_AI_PLUGIN_KEY, user_id)
            .await
            .map_err(map_plugin_error)?
        else {
            return Ok(unavailable_overview(
                AiAvailability::NotConfigured,
                requested_locale,
            ));
        };
        let translation_locale =
            requested_locale.unwrap_or_else(|| config.config().default_target_locale().to_owned());
        if !config.is_enabled() {
            return Ok(AiEntryOverview {
                availability: AiAvailability::Disabled,
                summary: AiOperationOverview::disabled(ContentJobOperation::Summarize, None),
                translation: AiOperationOverview::disabled(
                    ContentJobOperation::Translate,
                    Some(translation_locale),
                ),
            });
        }

        let summary = self
            .overview_operation(
                user_id,
                &entry,
                &installation,
                &config,
                ContentJobOperation::Summarize,
                None,
            )
            .await?;
        let translation = self
            .overview_operation(
                user_id,
                &entry,
                &installation,
                &config,
                ContentJobOperation::Translate,
                Some(translation_locale),
            )
            .await?;
        let availability = if [summary.state(), translation.state()]
            .into_iter()
            .any(|state| {
                !matches!(
                    state,
                    AiOperationState::Unavailable | AiOperationState::Disabled
                )
            }) {
            AiAvailability::Ready
        } else if [summary.state(), translation.state()]
            .into_iter()
            .any(|state| state == AiOperationState::Unavailable)
        {
            AiAvailability::ProviderUnavailable
        } else {
            AiAvailability::Disabled
        };
        Ok(AiEntryOverview {
            availability,
            summary,
            translation,
        })
    }

    pub async fn enqueue(
        &self,
        user_id: &str,
        entry_id: &str,
        operation: ContentJobOperation,
        target_locale: Option<&str>,
        idempotency_key: &str,
    ) -> Result<EnqueueResult, AiContentServiceError> {
        let context = self
            .resolve_operation(user_id, entry_id, operation, target_locale)
            .await?;
        let artifact = self
            .content
            .find_artifact_by_identity(user_id, &context.identity)
            .await
            .map_err(map_content_error)?;
        if artifact.is_none() && !self.keyring_available {
            return Err(AiContentServiceError::new(
                AiContentServiceErrorKind::KeyringUnavailable,
            ));
        }
        let request = EnqueueContentJob::new(EnqueueContentJobInput {
            operation,
            trigger: ContentJobTrigger::ReaderSidecar,
            identity: context.identity,
            idempotency_key: idempotency_key.to_owned(),
            call_chain_id: "reader-sidecar".to_owned(),
            remaining_depth: 0,
        })
        .map_err(map_content_error)?;
        let outcome = self
            .content
            .enqueue(request)
            .await
            .map_err(map_content_error)?;
        self.runtime.notify();
        Ok(outcome)
    }

    pub async fn retry(
        &self,
        user_id: &str,
        job_id: &str,
        idempotency_key: &str,
    ) -> Result<EnqueueResult, AiContentServiceError> {
        let old = self
            .content
            .get_job(user_id, job_id)
            .await
            .map_err(map_content_error)?;
        if old.status() != JobStatus::Failed {
            return Err(AiContentServiceError::new(
                AiContentServiceErrorKind::JobNotRetryable,
            ));
        }
        let outcome = self
            .enqueue(
                user_id,
                old.identity().entry_id(),
                old.operation(),
                old.identity().target_locale(),
                idempotency_key,
            )
            .await?;
        if matches!(&outcome, EnqueueResult::Existing(job) if job.id() == old.id()) {
            return Err(AiContentServiceError::new(
                AiContentServiceErrorKind::IdempotencyConflict,
            ));
        }
        Ok(outcome)
    }

    async fn overview_operation(
        &self,
        user_id: &str,
        entry: &ContentExecutionEntry,
        installation: &PluginInstallation,
        config: &PluginConfig,
        operation: ContentJobOperation,
        target_locale: Option<String>,
    ) -> Result<AiOperationOverview, AiContentServiceError> {
        if !operation_enabled(config, operation) {
            return Ok(AiOperationOverview::disabled(operation, target_locale));
        }
        let identity = match self
            .build_identity(
                user_id,
                entry,
                installation,
                config,
                operation,
                target_locale.as_deref(),
            )
            .await
        {
            Ok(identity) => identity,
            Err(error) if error.kind() == AiContentServiceErrorKind::ProviderUnavailable => {
                return Ok(AiOperationOverview::unavailable(operation, target_locale));
            }
            Err(error) => return Err(error),
        };
        let artifact = self
            .content
            .find_artifact_by_identity(user_id, &identity)
            .await
            .map_err(map_content_error)?;
        let job = self
            .content
            .find_latest_job_by_identity(user_id, &identity)
            .await
            .map_err(map_content_error)?;
        match (&artifact, &job) {
            (Some(_), Some(job)) if job.status() == JobStatus::Succeeded => {}
            (Some(_), _) => {
                return Err(AiContentServiceError::new(
                    AiContentServiceErrorKind::CorruptData,
                ));
            }
            (None, Some(job)) if job.status() == JobStatus::Succeeded => {
                return Err(AiContentServiceError::new(
                    AiContentServiceErrorKind::CorruptData,
                ));
            }
            (None, _) => {}
        }
        let state = if artifact.is_some() {
            AiOperationState::Succeeded
        } else {
            job.as_ref()
                .map_or(AiOperationState::Idle, |job| map_job_state(job.status()))
        };
        Ok(AiOperationOverview {
            operation,
            target_locale,
            state,
            job,
            artifact,
        })
    }

    async fn resolve_operation(
        &self,
        user_id: &str,
        entry_id: &str,
        operation: ContentJobOperation,
        target_locale: Option<&str>,
    ) -> Result<ResolvedOperation, AiContentServiceError> {
        let entry = self
            .content
            .get_execution_entry_for_user(user_id, entry_id)
            .await
            .map_err(map_content_error)?;
        let installation = self
            .plugins
            .get_installation(OFFICIAL_AI_PLUGIN_KEY)
            .await
            .map_err(map_plugin_unavailable)?;
        if installation.system_state() != PluginSystemState::Enabled {
            return Err(AiContentServiceError::new(
                AiContentServiceErrorKind::PluginUnavailable,
            ));
        }
        let config = self
            .plugins
            .get_ai_config(OFFICIAL_AI_PLUGIN_KEY, user_id)
            .await
            .map_err(map_plugin_error)?
            .ok_or_else(|| AiContentServiceError::new(AiContentServiceErrorKind::NotConfigured))?;
        if !config.is_enabled() || !operation_enabled(&config, operation) {
            return Err(AiContentServiceError::new(
                AiContentServiceErrorKind::Disabled,
            ));
        }
        let target_locale = match operation {
            ContentJobOperation::Summarize if target_locale.is_none() => None,
            ContentJobOperation::Translate => Some(
                normalize_requested_locale(target_locale)?
                    .unwrap_or_else(|| config.config().default_target_locale().to_owned()),
            ),
            ContentJobOperation::Summarize => {
                return Err(AiContentServiceError::new(
                    AiContentServiceErrorKind::InvalidInput,
                ));
            }
        };
        let identity = self
            .build_identity(
                user_id,
                &entry,
                &installation,
                &config,
                operation,
                target_locale.as_deref(),
            )
            .await?;
        Ok(ResolvedOperation { identity })
    }

    async fn build_identity(
        &self,
        user_id: &str,
        entry: &ContentExecutionEntry,
        installation: &PluginInstallation,
        config: &PluginConfig,
        operation: ContentJobOperation,
        target_locale: Option<&str>,
    ) -> Result<ArtifactIdentity, AiContentServiceError> {
        let provider_id = operation_provider_id(config, operation);
        let provider = self
            .providers
            .get_visible_for_user(provider_id, user_id)
            .await
            .map_err(map_provider_error)?;
        if !provider.is_enabled() {
            return Err(AiContentServiceError::new(
                AiContentServiceErrorKind::ProviderUnavailable,
            ));
        }
        let invocation = ContentInvocationInput::new(entry, operation, target_locale)
            .map_err(|_| AiContentServiceError::new(AiContentServiceErrorKind::InvalidInput))?;
        let contract = official_ai_contract(operation);
        ArtifactIdentity::new(ArtifactIdentityInput {
            user_id: user_id.to_owned(),
            entry_id: entry.entry_id().to_owned(),
            kind: operation.artifact_kind(),
            target_locale: invocation.target_locale().map(str::to_owned),
            entry_content_hash: entry.content_hash().to_owned(),
            input_hash: invocation.hash().to_owned(),
            config_hash: config.config_hash().to_owned(),
            plugin_key: contract.plugin_key.to_owned(),
            plugin_version: installation.version().to_owned(),
            component_digest: installation.component_digest().to_owned(),
            provider_binding_id: provider.id().to_owned(),
            provider_kind: provider.kind(),
            provider_model: provider.model().to_owned(),
            provider_revision: provider.revision(),
            prompt_version: contract.prompt_version.to_owned(),
            schema_id: contract.schema_id.to_owned(),
            mcp_provenance_hash: disabled_mcp_provenance_hash(),
        })
        .map_err(map_content_error)
    }
}

struct ResolvedOperation {
    identity: ArtifactIdentity,
}

fn unavailable_overview(
    availability: AiAvailability,
    translation_locale: Option<String>,
) -> AiEntryOverview {
    AiEntryOverview {
        availability,
        summary: AiOperationOverview::unavailable(ContentJobOperation::Summarize, None),
        translation: AiOperationOverview::unavailable(
            ContentJobOperation::Translate,
            translation_locale,
        ),
    }
}

fn operation_enabled(config: &PluginConfig, operation: ContentJobOperation) -> bool {
    match operation {
        ContentJobOperation::Summarize => config.config().summarize_enabled(),
        ContentJobOperation::Translate => config.config().translate_enabled(),
    }
}

fn operation_provider_id(config: &PluginConfig, operation: ContentJobOperation) -> &str {
    match operation {
        ContentJobOperation::Summarize => config.config().summarize_provider_id(),
        ContentJobOperation::Translate => config.config().translate_provider_id(),
    }
}

fn normalize_requested_locale(
    locale: Option<&str>,
) -> Result<Option<String>, AiContentServiceError> {
    locale
        .map(|locale| normalize_locale(locale, PluginRegistryErrorKind::InvalidInput))
        .transpose()
        .map_err(|_| AiContentServiceError::new(AiContentServiceErrorKind::InvalidInput))
}

const fn map_job_state(status: JobStatus) -> AiOperationState {
    match status {
        JobStatus::Queued => AiOperationState::Queued,
        JobStatus::Running => AiOperationState::Running,
        JobStatus::RetryWait => AiOperationState::RetryWait,
        JobStatus::Succeeded => AiOperationState::Succeeded,
        JobStatus::Failed => AiOperationState::Failed,
    }
}

fn map_content_error(error: ContentRepositoryError) -> AiContentServiceError {
    let kind = match error.kind() {
        ContentRepositoryErrorKind::InvalidInput
        | ContentRepositoryErrorKind::ExecutionInputTooLarge => {
            AiContentServiceErrorKind::InvalidInput
        }
        ContentRepositoryErrorKind::NotFound => AiContentServiceErrorKind::NotFound,
        ContentRepositoryErrorKind::EntryChanged => AiContentServiceErrorKind::EntryChanged,
        ContentRepositoryErrorKind::IdempotencyConflict => {
            AiContentServiceErrorKind::IdempotencyConflict
        }
        ContentRepositoryErrorKind::HashCollision
        | ContentRepositoryErrorKind::NonCanonicalJson
        | ContentRepositoryErrorKind::CorruptData => AiContentServiceErrorKind::CorruptData,
        ContentRepositoryErrorKind::Database => AiContentServiceErrorKind::Database,
        ContentRepositoryErrorKind::NoWork
        | ContentRepositoryErrorKind::UserConcurrencyLimited
        | ContentRepositoryErrorKind::LeaseLost
        | ContentRepositoryErrorKind::AlreadyCompleted
        | ContentRepositoryErrorKind::AttemptsExhausted
        | ContentRepositoryErrorKind::ArtifactTooLarge => AiContentServiceErrorKind::CorruptData,
    };
    AiContentServiceError::new(kind)
}

fn map_plugin_error(error: PluginRegistryError) -> AiContentServiceError {
    let kind = match error.kind() {
        PluginRegistryErrorKind::NotFound => AiContentServiceErrorKind::NotFound,
        PluginRegistryErrorKind::CorruptData => AiContentServiceErrorKind::CorruptData,
        PluginRegistryErrorKind::Database => AiContentServiceErrorKind::Database,
        PluginRegistryErrorKind::InvalidInput
        | PluginRegistryErrorKind::InvalidJson
        | PluginRegistryErrorKind::DuplicateJsonKey
        | PluginRegistryErrorKind::PayloadTooLarge
        | PluginRegistryErrorKind::InvalidManifest
        | PluginRegistryErrorKind::ComponentDigestMismatch
        | PluginRegistryErrorKind::UnknownSigningKey
        | PluginRegistryErrorKind::InvalidSignature
        | PluginRegistryErrorKind::InvalidConfig
        | PluginRegistryErrorKind::InvalidArtifact
        | PluginRegistryErrorKind::InvalidLifecycleEvent
        | PluginRegistryErrorKind::RevisionConflict
        | PluginRegistryErrorKind::QuotaExceeded => AiContentServiceErrorKind::CorruptData,
    };
    AiContentServiceError::new(kind)
}

fn map_plugin_unavailable(error: PluginRegistryError) -> AiContentServiceError {
    if error.kind() == PluginRegistryErrorKind::NotFound {
        AiContentServiceError::new(AiContentServiceErrorKind::PluginUnavailable)
    } else {
        map_plugin_error(error)
    }
}

fn map_provider_error(error: ProviderCoreError) -> AiContentServiceError {
    let kind = match error.kind() {
        ProviderCoreErrorKind::NotFound | ProviderCoreErrorKind::ProviderDisabled => {
            AiContentServiceErrorKind::ProviderUnavailable
        }
        ProviderCoreErrorKind::Database => AiContentServiceErrorKind::Database,
        ProviderCoreErrorKind::CorruptData => AiContentServiceErrorKind::CorruptData,
        ProviderCoreErrorKind::InvalidProviderId
        | ProviderCoreErrorKind::InvalidUserId
        | ProviderCoreErrorKind::InvalidDisplayName
        | ProviderCoreErrorKind::InvalidEndpoint
        | ProviderCoreErrorKind::InvalidModel
        | ProviderCoreErrorKind::InvalidCredential
        | ProviderCoreErrorKind::UnsupportedCapability
        | ProviderCoreErrorKind::InvalidPolicy
        | ProviderCoreErrorKind::InvalidPatch
        | ProviderCoreErrorKind::RevisionConflict
        | ProviderCoreErrorKind::SecretUnavailable => AiContentServiceErrorKind::CorruptData,
    };
    AiContentServiceError::new(kind)
}
