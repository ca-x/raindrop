use std::sync::Arc;

use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    middleware,
    routing::{get, post},
};
use secrecy::SecretString;
use serde::{Deserialize, Serialize};
use tokio::sync::OwnedSemaphorePermit;

use crate::{
    app::AppState,
    auth::{CsrfGuard, CurrentUser},
    content::provider::ProviderRepository,
    feeds::FeedRepository,
    translation::{
        AiTranslationProfile, ApiKeyUpdate, DeepLxDraft, ProductionOpenAiTranslationTransport,
        SaveTranslationConfig, TestTranslationInput, TranslationConfig, TranslationDisplayMode,
        TranslationEngine, TranslationError, TranslationErrorKind, TranslationLookupResult,
        TranslationRepository, TranslationResult, TranslationService, TranslationTestResult,
        TranslationTextResult,
    },
};

use super::{ApiError, ApiJson, RateLimitRejection, routes::sensitive_cache_headers};

pub(super) fn router() -> Router<AppState> {
    let plugin = Router::new()
        .route("/", get(get_config).put(put_config))
        .route("/test", post(test_connection))
        .route("/translate", post(translate_text))
        .route("/lookup", post(lookup))
        .route("/entries/{entry_id}/translate", post(translate_entry))
        .fallback(translation_not_found)
        .method_not_allowed_fallback(translation_method_not_allowed);
    Router::new()
        .route(
            "/api/v2/plugins/translation/",
            axum::routing::any(translation_not_found),
        )
        .nest("/api/v2/plugins/translation", plugin)
        .layer(middleware::map_response(sensitive_cache_headers))
}

async fn translation_not_found() -> ApiError {
    ApiError::not_found()
}

async fn translation_method_not_allowed() -> ApiError {
    ApiError::method_not_allowed()
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct TranslationConfigResponse {
    engine: &'static str,
    display_mode: &'static str,
    is_enabled: bool,
    default_target_locale: String,
    open_ai: OpenAiConfigResponse,
    deep_lx: DeepLxConfigResponse,
    revision: Option<u64>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct OpenAiConfigResponse {
    provider_id: Option<String>,
    max_output_tokens: u32,
    profile: &'static str,
    custom_system_prompt: Option<String>,
    custom_prompt: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DeepLxConfigResponse {
    display_name: String,
    description: Option<String>,
    base_url: Option<String>,
    has_api_key: bool,
}

impl From<TranslationConfig> for TranslationConfigResponse {
    fn from(config: TranslationConfig) -> Self {
        Self {
            engine: config.engine.as_storage(),
            display_mode: config.display_mode.as_storage(),
            is_enabled: config.is_enabled,
            default_target_locale: config.default_target_locale,
            open_ai: OpenAiConfigResponse {
                provider_id: config.open_ai.provider_id,
                max_output_tokens: config.open_ai.max_output_tokens,
                profile: config.open_ai.profile.as_storage(),
                custom_system_prompt: config.open_ai.custom_system_prompt,
                custom_prompt: config.open_ai.custom_prompt,
            },
            deep_lx: DeepLxConfigResponse {
                display_name: config.deeplx.display_name,
                description: config.deeplx.description,
                base_url: config.deeplx.base_url,
                has_api_key: config.deeplx.has_api_key,
            },
            revision: config.revision,
        }
    }
}

#[derive(Clone, Copy, Deserialize)]
enum TranslationEngineRequest {
    #[serde(rename = "OPENAI")]
    OpenAi,
    #[serde(rename = "DEEPLX")]
    DeepLx,
}

impl From<TranslationEngineRequest> for TranslationEngine {
    fn from(value: TranslationEngineRequest) -> Self {
        match value {
            TranslationEngineRequest::OpenAi => Self::OpenAi,
            TranslationEngineRequest::DeepLx => Self::DeepLx,
        }
    }
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
enum TranslationDisplayModeRequest {
    TranslationOnly,
    Bilingual,
    Hover,
    SideBySide,
}

impl From<TranslationDisplayModeRequest> for TranslationDisplayMode {
    fn from(value: TranslationDisplayModeRequest) -> Self {
        match value {
            TranslationDisplayModeRequest::TranslationOnly => Self::TranslationOnly,
            TranslationDisplayModeRequest::Bilingual => Self::Bilingual,
            TranslationDisplayModeRequest::Hover => Self::Hover,
            TranslationDisplayModeRequest::SideBySide => Self::SideBySide,
        }
    }
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
enum AiTranslationProfileRequest {
    General,
    Technical,
    Literary,
    Academic,
    Business,
    SocialNews,
    Custom,
}

impl From<AiTranslationProfileRequest> for AiTranslationProfile {
    fn from(value: AiTranslationProfileRequest) -> Self {
        match value {
            AiTranslationProfileRequest::General => Self::General,
            AiTranslationProfileRequest::Technical => Self::Technical,
            AiTranslationProfileRequest::Literary => Self::Literary,
            AiTranslationProfileRequest::Academic => Self::Academic,
            AiTranslationProfileRequest::Business => Self::Business,
            AiTranslationProfileRequest::SocialNews => Self::SocialNews,
            AiTranslationProfileRequest::Custom => Self::Custom,
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PutTranslationConfigRequest {
    expected_revision: Option<u64>,
    engine: TranslationEngineRequest,
    display_mode: TranslationDisplayModeRequest,
    is_enabled: bool,
    default_target_locale: String,
    open_ai: OpenAiConfigRequest,
    deep_lx: DeepLxConfigRequest,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct OpenAiConfigRequest {
    provider_id: Option<String>,
    max_output_tokens: u32,
    profile: AiTranslationProfileRequest,
    custom_system_prompt: Option<String>,
    custom_prompt: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct DeepLxConfigRequest {
    display_name: String,
    description: Option<String>,
    base_url: Option<String>,
    #[serde(default, deserialize_with = "deserialize_present")]
    api_key: Option<Option<String>>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct TestTranslationRequest {
    engine: TranslationEngineRequest,
    target_locale: String,
    open_ai: TestOpenAiRequest,
    deep_lx: TestDeepLxRequest,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct TestOpenAiRequest {
    provider_id: Option<String>,
    max_output_tokens: u32,
    profile: AiTranslationProfileRequest,
    custom_system_prompt: Option<String>,
    custom_prompt: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct TestDeepLxRequest {
    base_url: Option<String>,
    #[serde(default, deserialize_with = "deserialize_present")]
    api_key: Option<Option<String>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LookupRequest {
    text: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TranslateTextRequest {
    text: String,
}

fn deserialize_present<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    T::deserialize(deserializer).map(Some)
}

fn api_key_update(value: Option<Option<String>>) -> ApiKeyUpdate {
    match value {
        None => ApiKeyUpdate::Keep,
        Some(Some(value)) => ApiKeyUpdate::Set(SecretString::from(value)),
        Some(None) => ApiKeyUpdate::Clear,
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct TranslationTestResponse {
    translated_text: String,
    provider_label: String,
    detected_source_locale: Option<String>,
    target_locale: String,
}

impl From<TranslationTestResult> for TranslationTestResponse {
    fn from(result: TranslationTestResult) -> Self {
        Self {
            translated_text: result.translated_text,
            provider_label: result.provider_label,
            detected_source_locale: result.detected_source_locale,
            target_locale: result.target_locale,
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct TranslationTextResponse {
    translated_text: String,
    provider_label: String,
    detected_source_locale: Option<String>,
    target_locale: String,
}

impl From<TranslationTextResult> for TranslationTextResponse {
    fn from(result: TranslationTextResult) -> Self {
        Self {
            translated_text: result.translated_text,
            provider_label: result.provider_label,
            detected_source_locale: result.detected_source_locale,
            target_locale: result.target_locale,
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct TranslationResponse {
    title: String,
    segments: Vec<TranslationSegmentResponse>,
    provider_label: String,
    detected_source_locale: Option<String>,
    target_locale: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct TranslationSegmentResponse {
    index: u32,
    original_text: String,
    translated_text: String,
}

impl From<TranslationResult> for TranslationResponse {
    fn from(result: TranslationResult) -> Self {
        Self {
            title: result.title,
            segments: result
                .segments
                .into_iter()
                .map(|segment| TranslationSegmentResponse {
                    index: segment.index,
                    original_text: segment.original_text,
                    translated_text: segment.translated_text,
                })
                .collect(),
            provider_label: result.provider_label,
            detected_source_locale: result.detected_source_locale,
            target_locale: result.target_locale,
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct LookupResponse {
    query: String,
    translation: String,
    definition: Option<String>,
    examples: Vec<LookupExampleResponse>,
    provider_label: String,
    detected_source_locale: Option<String>,
    target_locale: String,
}

#[derive(Serialize)]
struct LookupExampleResponse {
    source: String,
    target: String,
}

impl From<TranslationLookupResult> for LookupResponse {
    fn from(result: TranslationLookupResult) -> Self {
        Self {
            query: result.query,
            translation: result.translation,
            definition: result.definition,
            examples: result
                .examples
                .into_iter()
                .map(|example| LookupExampleResponse {
                    source: example.source,
                    target: example.target,
                })
                .collect(),
            provider_label: result.provider_label,
            detected_source_locale: result.detected_source_locale,
            target_locale: result.target_locale,
        }
    }
}

async fn get_config(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
) -> Result<Json<TranslationConfigResponse>, ApiError> {
    let config = service(&state)?
        .get_config(&user.id)
        .await
        .map_err(map_error)?;
    Ok(Json(config.into()))
}

async fn put_config(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    _csrf: CsrfGuard,
    ApiJson(request): ApiJson<PutTranslationConfigRequest>,
) -> Result<Json<TranslationConfigResponse>, ApiError> {
    state
        .provider_mutation_limiter
        .check(&user.id)
        .map_err(map_limiter_rejection)?;
    let config = service(&state)?
        .save_config(
            &user.id,
            SaveTranslationConfig {
                expected_revision: request.expected_revision,
                engine: request.engine.into(),
                display_mode: request.display_mode.into(),
                is_enabled: request.is_enabled,
                default_target_locale: request.default_target_locale,
                open_ai_provider_id: request.open_ai.provider_id,
                open_ai_max_output_tokens: request.open_ai.max_output_tokens,
                open_ai_profile: request.open_ai.profile.into(),
                open_ai_custom_system_prompt: request.open_ai.custom_system_prompt,
                open_ai_custom_prompt: request.open_ai.custom_prompt,
                deeplx_display_name: request.deep_lx.display_name,
                deeplx_description: request.deep_lx.description,
                deeplx_base_url: request.deep_lx.base_url,
                deeplx_api_key: api_key_update(request.deep_lx.api_key),
            },
        )
        .await
        .map_err(map_error)?;
    Ok(Json(config.into()))
}

async fn test_connection(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    _csrf: CsrfGuard,
    ApiJson(request): ApiJson<TestTranslationRequest>,
) -> Result<Json<TranslationTestResponse>, ApiError> {
    state
        .provider_mutation_limiter
        .check(&user.id)
        .map_err(map_limiter_rejection)?;
    let _permits = acquire_translation_permits(&state, &user.id)?;
    let result = service(&state)?
        .test_connection(
            &user.id,
            TestTranslationInput {
                engine: request.engine.into(),
                open_ai_provider_id: request.open_ai.provider_id,
                open_ai_max_output_tokens: request.open_ai.max_output_tokens,
                open_ai_profile: request.open_ai.profile.into(),
                open_ai_custom_system_prompt: request.open_ai.custom_system_prompt,
                open_ai_custom_prompt: request.open_ai.custom_prompt,
                deeplx: DeepLxDraft {
                    base_url: request.deep_lx.base_url,
                    api_key: api_key_update(request.deep_lx.api_key),
                },
                target_locale: request.target_locale,
            },
        )
        .await
        .map_err(map_error)?;
    Ok(Json(result.into()))
}

async fn translate_entry(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    _csrf: CsrfGuard,
    Path(entry_id): Path<String>,
) -> Result<Json<TranslationResponse>, ApiError> {
    admit_translation_request(&state, &user.id)?;
    let _permits = acquire_translation_permits(&state, &user.id)?;
    let result = service(&state)?
        .translate_entry(&user.id, &entry_id)
        .await
        .map_err(map_error)?;
    Ok(Json(result.into()))
}

async fn translate_text(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    _csrf: CsrfGuard,
    ApiJson(request): ApiJson<TranslateTextRequest>,
) -> Result<Json<TranslationTextResponse>, ApiError> {
    admit_translation_request(&state, &user.id)?;
    let _permits = acquire_translation_permits(&state, &user.id)?;
    let result = service(&state)?
        .translate_text(&user.id, &request.text)
        .await
        .map_err(map_error)?;
    Ok(Json(result.into()))
}

async fn lookup(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    _csrf: CsrfGuard,
    ApiJson(request): ApiJson<LookupRequest>,
) -> Result<Json<LookupResponse>, ApiError> {
    admit_translation_request(&state, &user.id)?;
    let _permits = acquire_translation_permits(&state, &user.id)?;
    let result = service(&state)?
        .lookup(&user.id, &request.text)
        .await
        .map_err(map_error)?;
    Ok(Json(result.into()))
}

fn admit_translation_request(state: &AppState, user_id: &str) -> Result<(), ApiError> {
    state
        .content_mutation_limiter
        .check(user_id)
        .map_err(map_limiter_rejection)
}

fn acquire_translation_permits(
    state: &AppState,
    user_id: &str,
) -> Result<(OwnedSemaphorePermit, OwnedSemaphorePermit), ApiError> {
    let user_permit = state
        .translation_user_concurrency
        .try_acquire(user_id)
        .ok_or_else(ApiError::rate_limited)?;
    let global_permit = state
        .translation_request_semaphore
        .clone()
        .try_acquire_owned()
        .map_err(|_| ApiError::rate_limited())?;
    Ok((user_permit, global_permit))
}

fn service(state: &AppState) -> Result<TranslationService, ApiError> {
    let database = state.setup.database().map_err(|_| ApiError::internal())?;
    let keyring = state.provider_keyring();
    let providers = ProviderRepository::new(database.clone(), keyring.clone());
    let openai = if let Some(transport) = state.translation_openai_transport() {
        transport
    } else {
        Arc::new(
            ProductionOpenAiTranslationTransport::new(ProviderRepository::new(
                database.clone(),
                keyring.clone(),
            ))
            .map_err(map_error)?,
        )
    };
    Ok(TranslationService::new(
        TranslationRepository::new(database.clone(), keyring),
        FeedRepository::new(database),
        providers,
        state.translation_deeplx_transport(),
        openai,
    ))
}

fn map_error(error: TranslationError) -> ApiError {
    match error.kind() {
        TranslationErrorKind::InvalidInput => ApiError::validation(),
        TranslationErrorKind::NotConfigured => ApiError::new(
            StatusCode::CONFLICT,
            "TRANSLATION_NOT_CONFIGURED",
            "Translation is not configured",
        ),
        TranslationErrorKind::Disabled => ApiError::new(
            StatusCode::CONFLICT,
            "TRANSLATION_DISABLED",
            "Translation is disabled",
        ),
        TranslationErrorKind::ProviderUnavailable => ApiError::new(
            StatusCode::CONFLICT,
            "TRANSLATION_PROVIDER_UNAVAILABLE",
            "The selected translation provider is unavailable",
        ),
        TranslationErrorKind::KeyringUnavailable => ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "TRANSLATION_SECRET_UNAVAILABLE",
            "Translation credentials are unavailable",
        ),
        TranslationErrorKind::RevisionConflict => ApiError::new(
            StatusCode::CONFLICT,
            "REVISION_CONFLICT",
            "The resource changed; reload and try again",
        ),
        TranslationErrorKind::NotFound => ApiError::not_found(),
        TranslationErrorKind::TooLarge => ApiError::new(
            StatusCode::PAYLOAD_TOO_LARGE,
            "TRANSLATION_INPUT_TOO_LARGE",
            "The translation input is too large",
        ),
        TranslationErrorKind::RateLimited => ApiError::new(
            StatusCode::TOO_MANY_REQUESTS,
            "TRANSLATION_RATE_LIMITED",
            "The translation provider is rate limiting requests",
        ),
        TranslationErrorKind::Timeout => ApiError::new(
            StatusCode::GATEWAY_TIMEOUT,
            "TRANSLATION_TIMEOUT",
            "The translation provider did not respond in time",
        ),
        TranslationErrorKind::Upstream => ApiError::new(
            StatusCode::BAD_GATEWAY,
            "TRANSLATION_UPSTREAM_ERROR",
            "The translation provider could not complete the request",
        ),
        TranslationErrorKind::CorruptData | TranslationErrorKind::Database => ApiError::internal(),
    }
}

fn map_limiter_rejection(rejection: RateLimitRejection) -> ApiError {
    ApiError::rate_limited_with_retry(
        rejection
            .retry_at
            .format(&time::format_description::well_known::Rfc3339)
            .unwrap_or_else(|_| rejection.retry_at.unix_timestamp().to_string()),
        rejection.retry_after_seconds,
    )
}
