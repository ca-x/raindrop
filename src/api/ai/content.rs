use std::fmt;

use axum::{
    Json, Router,
    extract::{FromRequestParts, Path, Query, State},
    http::{HeaderValue, StatusCode, header::LOCATION, request::Parts},
    middleware,
    response::{IntoResponse, Response},
    routing::{get, post},
};
use serde::{Deserialize, Deserializer, Serialize, de::DeserializeOwned, de::Visitor};
use time::{OffsetDateTime, UtcOffset, macros::format_description};
use uuid::Uuid;

use crate::{
    app::AppState,
    auth::{CsrfGuard, CurrentUser},
    content::{
        ai::{
            AiAvailability, AiContentService, AiContentServiceError, AiContentServiceErrorKind,
            AiEntryOverview, AiOperationOverview, AiOperationState,
        },
        jobs::{
            ArtifactKind, ArtifactSnapshot, ContentJobOperation, ContentRepository,
            ContentRepositoryError, ContentRepositoryErrorKind, EnqueueResult, JobSnapshot,
            JobStatus,
        },
    },
    plugins::{SummaryArtifact, TranslationArtifact},
};

use super::super::{ApiError, ApiJson, RateLimitRejection, routes::sensitive_cache_headers};

const MCP_STATE: &str = "CONTRACT_READY_TRANSPORT_UNAVAILABLE";
const PUBLIC_TIME_FORMAT: &[time::format_description::FormatItem<'static>] =
    format_description!("[year]-[month]-[day]T[hour]:[minute]:[second].[subsecond digits:6]Z");

pub(super) fn router() -> Router<AppState> {
    Router::new()
        .route(
            "/api/v1/entries/{entry_id}/ai",
            get(get_entry_ai).fallback(content_method_not_allowed),
        )
        .route(
            "/api/v1/entries/{entry_id}/ai/jobs",
            post(enqueue_entry_ai).fallback(content_method_not_allowed),
        )
        .route(
            "/api/v1/ai/jobs/{job_id}",
            get(get_ai_job).fallback(content_method_not_allowed),
        )
        .route(
            "/api/v1/ai/jobs/{job_id}/result",
            get(get_ai_result).fallback(content_method_not_allowed),
        )
        .route(
            "/api/v1/ai/jobs/{job_id}/retry",
            post(retry_ai_job).fallback(content_method_not_allowed),
        )
        .route(
            "/api/v1/entries/{entry_id}/ai/",
            axum::routing::any(content_not_found),
        )
        .route(
            "/api/v1/entries/{entry_id}/ai/{*rest}",
            axum::routing::any(content_not_found),
        )
        .route(
            "/api/v1/ai/jobs/{job_id}/",
            axum::routing::any(content_not_found),
        )
        .route(
            "/api/v1/ai/jobs/{job_id}/{*rest}",
            axum::routing::any(content_not_found),
        )
        .layer(middleware::map_response(sensitive_cache_headers))
}

async fn content_not_found() -> ApiError {
    ApiError::not_found()
}

async fn content_method_not_allowed() -> ApiError {
    ApiError::method_not_allowed()
}

struct ApiPath<T>(T);

impl<T, S> FromRequestParts<S> for ApiPath<T>
where
    T: DeserializeOwned + Send,
    S: Send + Sync,
{
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        Path::<T>::from_request_parts(parts, state)
            .await
            .map(|Path(value)| Self(value))
            .map_err(|_| ApiError::validation())
    }
}

struct ApiQuery<T>(T);

impl<T, S> FromRequestParts<S> for ApiQuery<T>
where
    T: DeserializeOwned,
    S: Send + Sync,
{
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        Query::<T>::from_request_parts(parts, state)
            .await
            .map(|Query(value)| Self(value))
            .map_err(|_| ApiError::validation())
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct EntryAiQuery {
    translation_locale: Option<String>,
}

#[derive(Clone, Copy, Deserialize)]
enum OperationRequest {
    #[serde(rename = "SUMMARIZE")]
    Summarize,
    #[serde(rename = "TRANSLATE")]
    Translate,
}

impl From<OperationRequest> for ContentJobOperation {
    fn from(value: OperationRequest) -> Self {
        match value {
            OperationRequest::Summarize => Self::Summarize,
            OperationRequest::Translate => Self::Translate,
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct EnqueueAiRequest {
    operation: OperationRequest,
    target_locale: RequiredNullableLocale,
    idempotency_key: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RetryAiRequest {
    idempotency_key: String,
}

struct RequiredNullableLocale(Option<String>);

impl<'de> Deserialize<'de> for RequiredNullableLocale {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_any(RequiredNullableLocaleVisitor)
    }
}

struct RequiredNullableLocaleVisitor;

impl Visitor<'_> for RequiredNullableLocaleVisitor {
    type Value = RequiredNullableLocale;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a target locale string or null")
    }

    fn visit_unit<E>(self) -> Result<Self::Value, E> {
        Ok(RequiredNullableLocale(None))
    }

    fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        Ok(RequiredNullableLocale(Some(value.to_owned())))
    }

    fn visit_string<E>(self, value: String) -> Result<Self::Value, E> {
        Ok(RequiredNullableLocale(Some(value)))
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct EntryAiResponse {
    availability: &'static str,
    mcp_state: &'static str,
    summary: AiOperationResponse,
    translation: AiOperationResponse,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AiOperationResponse {
    operation: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    target_locale: Option<String>,
    state: &'static str,
    job: Option<AiJobResponse>,
    artifact: Option<AiArtifactResponse>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AiJobResponse {
    job_id: String,
    status: &'static str,
    attempts: u8,
    max_attempts: u8,
    next_attempt_at: String,
    last_error_code: Option<String>,
    created_at: String,
    started_at: Option<String>,
    completed_at: Option<String>,
}

impl AiJobResponse {
    fn from_snapshot(job: &JobSnapshot) -> Result<Self, ApiError> {
        if job.last_error_code().is_some_and(|code| {
            code.is_empty()
                || code.len() > 64
                || !code
                    .bytes()
                    .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_')
        }) {
            return Err(ApiError::internal());
        }
        Ok(Self {
            job_id: job.id().to_owned(),
            status: job.status().as_storage(),
            attempts: job.attempts(),
            max_attempts: job.max_attempts(),
            next_attempt_at: format_public_time(job.next_attempt_at())?,
            last_error_code: job.last_error_code().map(str::to_owned),
            created_at: format_public_time(job.created_at())?,
            started_at: job.started_at().map(format_public_time).transpose()?,
            completed_at: job.completed_at().map(format_public_time).transpose()?,
        })
    }
}

#[derive(Serialize)]
#[serde(untagged)]
enum AiArtifactResponse {
    Summary(SummaryArtifactResponse),
    Translation(TranslationArtifactResponse),
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SummaryArtifactResponse {
    artifact_id: String,
    kind: &'static str,
    provider_label: String,
    created_at: String,
    source_language: String,
    summary: String,
    bullets: Vec<String>,
    conclusion: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct TranslationArtifactResponse {
    artifact_id: String,
    kind: &'static str,
    provider_label: String,
    created_at: String,
    detected_source_language: String,
    target_locale: String,
    title: String,
    body_markdown: String,
}

impl AiArtifactResponse {
    fn from_snapshot(artifact: &ArtifactSnapshot) -> Result<Self, ApiError> {
        if artifact.provider_label().is_empty()
            || artifact.provider_label().len() > 200
            || artifact.provider_label().chars().any(char::is_control)
        {
            return Err(ApiError::internal());
        }
        match artifact.identity().kind() {
            ArtifactKind::AiSummary => {
                let parsed = SummaryArtifact::parse(artifact.payload_json().as_bytes())
                    .map_err(|_| ApiError::internal())?;
                Ok(Self::Summary(SummaryArtifactResponse {
                    artifact_id: artifact.id().to_owned(),
                    kind: "AI_SUMMARY",
                    provider_label: artifact.provider_label().to_owned(),
                    created_at: format_public_time(artifact.created_at())?,
                    source_language: parsed.source_language().to_owned(),
                    summary: parsed.summary().to_owned(),
                    bullets: parsed.bullets().to_vec(),
                    conclusion: parsed.conclusion().map(str::to_owned),
                }))
            }
            ArtifactKind::AiTranslation => {
                let parsed = TranslationArtifact::parse(artifact.payload_json().as_bytes())
                    .map_err(|_| ApiError::internal())?;
                if Some(parsed.target_locale()) != artifact.identity().target_locale() {
                    return Err(ApiError::internal());
                }
                Ok(Self::Translation(TranslationArtifactResponse {
                    artifact_id: artifact.id().to_owned(),
                    kind: "AI_TRANSLATION",
                    provider_label: artifact.provider_label().to_owned(),
                    created_at: format_public_time(artifact.created_at())?,
                    detected_source_language: parsed.detected_source_language().to_owned(),
                    target_locale: parsed.target_locale().to_owned(),
                    title: parsed.title().to_owned(),
                    body_markdown: parsed.body_markdown().to_owned(),
                }))
            }
        }
    }
}

impl EntryAiResponse {
    fn from_overview(overview: &AiEntryOverview) -> Result<Self, ApiError> {
        Ok(Self {
            availability: availability_wire(overview.availability()),
            mcp_state: MCP_STATE,
            summary: AiOperationResponse::from_overview(overview.summary())?,
            translation: AiOperationResponse::from_overview(overview.translation())?,
        })
    }
}

impl AiOperationResponse {
    fn from_overview(overview: &AiOperationOverview) -> Result<Self, ApiError> {
        Ok(Self {
            operation: overview.operation().as_storage(),
            target_locale: overview.target_locale().map(str::to_owned),
            state: operation_state_wire(overview.state()),
            job: overview
                .job()
                .map(AiJobResponse::from_snapshot)
                .transpose()?,
            artifact: overview
                .artifact()
                .map(AiArtifactResponse::from_snapshot)
                .transpose()?,
        })
    }
}

async fn get_entry_ai(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    ApiPath(entry_id): ApiPath<String>,
    ApiQuery(query): ApiQuery<EntryAiQuery>,
) -> Result<Json<EntryAiResponse>, ApiError> {
    validate_canonical_uuid(&entry_id, "entryId")?;
    let overview = ai_service(&state)?
        .overview(&user.id, &entry_id, query.translation_locale.as_deref())
        .await
        .map_err(map_service_error)?;
    Ok(Json(EntryAiResponse::from_overview(&overview)?))
}

async fn enqueue_entry_ai(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    _csrf: CsrfGuard,
    ApiPath(entry_id): ApiPath<String>,
    ApiJson(request): ApiJson<EnqueueAiRequest>,
) -> Result<Response, ApiError> {
    validate_canonical_uuid(&entry_id, "entryId")?;
    state
        .content_mutation_limiter
        .check(&user.id)
        .map_err(map_limiter_rejection)?;
    let outcome = ai_service(&state)?
        .enqueue(
            &user.id,
            &entry_id,
            request.operation.into(),
            request.target_locale.0.as_deref(),
            &request.idempotency_key,
        )
        .await
        .map_err(map_service_error)?;
    enqueue_response(outcome)
}

async fn get_ai_job(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    ApiPath(job_id): ApiPath<String>,
) -> Result<Json<AiJobResponse>, ApiError> {
    validate_canonical_uuid(&job_id, "jobId")?;
    let job = content_repository(&state)?
        .get_job(&user.id, &job_id)
        .await
        .map_err(map_repository_error)?;
    Ok(Json(AiJobResponse::from_snapshot(&job)?))
}

async fn get_ai_result(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    ApiPath(job_id): ApiPath<String>,
) -> Result<Json<AiArtifactResponse>, ApiError> {
    validate_canonical_uuid(&job_id, "jobId")?;
    let repository = content_repository(&state)?;
    let job = repository
        .get_job(&user.id, &job_id)
        .await
        .map_err(map_repository_error)?;
    if job.status() != JobStatus::Succeeded {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "AI_RESULT_NOT_READY",
            "AI result is not ready",
        ));
    }
    let result = repository
        .get_result(&user.id, &job_id)
        .await
        .map_err(|error| {
            if error.kind() == ContentRepositoryErrorKind::NotFound {
                ApiError::internal()
            } else {
                map_repository_error(error)
            }
        })?;
    Ok(Json(AiArtifactResponse::from_snapshot(result.artifact())?))
}

async fn retry_ai_job(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    _csrf: CsrfGuard,
    ApiPath(job_id): ApiPath<String>,
    ApiJson(request): ApiJson<RetryAiRequest>,
) -> Result<Response, ApiError> {
    validate_canonical_uuid(&job_id, "jobId")?;
    state
        .content_mutation_limiter
        .check(&user.id)
        .map_err(map_limiter_rejection)?;
    let outcome = ai_service(&state)?
        .retry(&user.id, &job_id, &request.idempotency_key)
        .await
        .map_err(map_service_error)?;
    enqueue_response(outcome)
}

fn enqueue_response(outcome: EnqueueResult) -> Result<Response, ApiError> {
    let (status, job) = match outcome {
        EnqueueResult::Queued(job) | EnqueueResult::Reused { job, .. } => {
            (StatusCode::CREATED, job)
        }
        EnqueueResult::Existing(job) => (StatusCode::OK, job),
    };
    let location = HeaderValue::from_str(&format!("/api/v1/ai/jobs/{}", job.id()))
        .map_err(|_| ApiError::internal())?;
    Ok((
        status,
        [(LOCATION, location)],
        Json(AiJobResponse::from_snapshot(&job)?),
    )
        .into_response())
}

fn ai_service(state: &AppState) -> Result<AiContentService, ApiError> {
    let database = state.setup.database().map_err(|_| ApiError::internal())?;
    Ok(AiContentService::new(
        database,
        state.provider_keyring(),
        state.content_runtime.clone(),
    ))
}

fn content_repository(state: &AppState) -> Result<ContentRepository, ApiError> {
    state
        .setup
        .database()
        .map(ContentRepository::new)
        .map_err(|_| ApiError::internal())
}

fn validate_canonical_uuid(value: &str, field: &'static str) -> Result<(), ApiError> {
    let parsed = Uuid::parse_str(value)
        .map_err(|_| ApiError::validation().with_field(field, "Identifier is invalid"))?;
    if parsed.to_string() == value {
        Ok(())
    } else {
        Err(ApiError::validation().with_field(field, "Identifier is invalid"))
    }
}

fn map_service_error(error: AiContentServiceError) -> ApiError {
    match error.kind() {
        AiContentServiceErrorKind::InvalidInput => ApiError::validation(),
        AiContentServiceErrorKind::NotFound => ApiError::not_found(),
        AiContentServiceErrorKind::EntryChanged
        | AiContentServiceErrorKind::NotConfigured
        | AiContentServiceErrorKind::Disabled
        | AiContentServiceErrorKind::ProviderUnavailable
        | AiContentServiceErrorKind::PluginUnavailable => ApiError::new(
            StatusCode::CONFLICT,
            "AI_UNAVAILABLE",
            "AI content is temporarily unavailable",
        ),
        AiContentServiceErrorKind::KeyringUnavailable => ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "AI_PROVIDER_KEYRING_UNAVAILABLE",
            "AI provider credentials are unavailable",
        ),
        AiContentServiceErrorKind::IdempotencyConflict => ApiError::new(
            StatusCode::CONFLICT,
            "IDEMPOTENCY_CONFLICT",
            "The idempotency key was used for a different request",
        ),
        AiContentServiceErrorKind::JobNotRetryable => ApiError::new(
            StatusCode::CONFLICT,
            "AI_JOB_NOT_RETRYABLE",
            "The AI job cannot be retried",
        ),
        AiContentServiceErrorKind::CorruptData | AiContentServiceErrorKind::Database => {
            ApiError::internal()
        }
    }
}

fn map_repository_error(error: ContentRepositoryError) -> ApiError {
    match error.kind() {
        ContentRepositoryErrorKind::InvalidInput => ApiError::validation(),
        ContentRepositoryErrorKind::NotFound => ApiError::not_found(),
        ContentRepositoryErrorKind::IdempotencyConflict => ApiError::new(
            StatusCode::CONFLICT,
            "IDEMPOTENCY_CONFLICT",
            "The idempotency key was used for a different request",
        ),
        ContentRepositoryErrorKind::EntryChanged
        | ContentRepositoryErrorKind::NoWork
        | ContentRepositoryErrorKind::UserConcurrencyLimited
        | ContentRepositoryErrorKind::LeaseLost
        | ContentRepositoryErrorKind::AlreadyCompleted
        | ContentRepositoryErrorKind::AttemptsExhausted
        | ContentRepositoryErrorKind::ArtifactTooLarge
        | ContentRepositoryErrorKind::ExecutionInputTooLarge
        | ContentRepositoryErrorKind::HashCollision
        | ContentRepositoryErrorKind::NonCanonicalJson
        | ContentRepositoryErrorKind::CorruptData
        | ContentRepositoryErrorKind::Database => ApiError::internal(),
    }
}

const fn availability_wire(availability: AiAvailability) -> &'static str {
    match availability {
        AiAvailability::Ready => "READY",
        AiAvailability::NotConfigured => "NOT_CONFIGURED",
        AiAvailability::Disabled => "DISABLED",
        AiAvailability::ProviderUnavailable => "PROVIDER_UNAVAILABLE",
        AiAvailability::PluginUnavailable => "PLUGIN_UNAVAILABLE",
    }
}

const fn operation_state_wire(state: AiOperationState) -> &'static str {
    match state {
        AiOperationState::Unavailable => "UNAVAILABLE",
        AiOperationState::Disabled => "DISABLED",
        AiOperationState::Idle => "IDLE",
        AiOperationState::Queued => "QUEUED",
        AiOperationState::Running => "RUNNING",
        AiOperationState::RetryWait => "RETRY_WAIT",
        AiOperationState::Succeeded => "SUCCEEDED",
        AiOperationState::Failed => "FAILED",
    }
}

fn map_limiter_rejection(rejection: RateLimitRejection) -> ApiError {
    match format_public_time(rejection.retry_at) {
        Ok(retry_at) => ApiError::rate_limited_with_retry(retry_at, rejection.retry_after_seconds),
        Err(error) => error,
    }
}

fn format_public_time(value: OffsetDateTime) -> Result<String, ApiError> {
    value
        .to_offset(UtcOffset::UTC)
        .format(PUBLIC_TIME_FORMAT)
        .map_err(|_| ApiError::internal())
}
