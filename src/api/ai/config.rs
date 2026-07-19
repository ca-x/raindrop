use std::fmt;

use axum::{Json, Router, extract::State, http::StatusCode, middleware, routing::get};
use serde::{Deserialize, Deserializer, Serialize, de::Visitor};

use crate::{
    app::AppState,
    auth::{CsrfGuard, CurrentUser},
    content::provider::{ProviderCoreError, ProviderCoreErrorKind, ProviderRepository},
    plugins::{
        AiContentConfig, AiSummaryStyle, PluginConfig, PluginRegistryError,
        PluginRegistryErrorKind, PluginRegistryRepository, PluginSystemState,
    },
};

use super::super::{ApiError, ApiJson, RateLimitRejection, routes::sensitive_cache_headers};

const OFFICIAL_AI_PLUGIN_KEY: &str = "raindrop.ai-content";
const MCP_STATE: &str = "CONTRACT_READY_TRANSPORT_UNAVAILABLE";

pub(super) fn router() -> Router<AppState> {
    Router::new()
        .route(CONFIG_PATH, get(get_config).put(put_config))
        .route("/api/v1/ai/config/", axum::routing::any(config_not_found))
        .method_not_allowed_fallback(config_method_not_allowed)
        .layer(middleware::map_response(sensitive_cache_headers))
}

const CONFIG_PATH: &str = "/api/v1/ai/config";

async fn config_not_found() -> ApiError {
    ApiError::not_found()
}

async fn config_method_not_allowed() -> ApiError {
    ApiError::method_not_allowed()
}

#[derive(Clone, Copy)]
enum PublicPluginState {
    Ready,
    Unavailable,
    Disabled,
    Quarantined,
}

impl PublicPluginState {
    const fn as_wire(self) -> &'static str {
        match self {
            Self::Ready => "READY",
            Self::Unavailable => "UNAVAILABLE",
            Self::Disabled => "DISABLED",
            Self::Quarantined => "QUARANTINED",
        }
    }

    const fn is_ready(self) -> bool {
        matches!(self, Self::Ready)
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AiConfigEnvelope {
    plugin_state: &'static str,
    mcp_state: &'static str,
    config: Option<AiConfigResponse>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AiConfigResponse {
    revision: u64,
    is_enabled: bool,
    summary: AiSummaryConfigResponse,
    translation: AiTranslationConfigResponse,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AiSummaryConfigResponse {
    enabled: bool,
    provider_id: String,
    style: AiSummaryStyle,
    max_output_tokens: u32,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AiTranslationConfigResponse {
    enabled: bool,
    provider_id: String,
    default_target_locale: String,
    max_output_tokens: u32,
}

impl AiConfigResponse {
    fn from_stored(config: &PluginConfig) -> Self {
        let document = config.config();
        Self {
            revision: config.revision(),
            is_enabled: config.is_enabled(),
            summary: AiSummaryConfigResponse {
                enabled: document.summarize_enabled(),
                provider_id: document.summarize_provider_id().to_owned(),
                style: document.summarize_style(),
                max_output_tokens: document.summarize_max_output_tokens(),
            },
            translation: AiTranslationConfigResponse {
                enabled: document.translate_enabled(),
                provider_id: document.translate_provider_id().to_owned(),
                default_target_locale: document.default_target_locale().to_owned(),
                max_output_tokens: document.translate_max_output_tokens(),
            },
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PutAiConfigRequest {
    expected_revision: RequiredNullableRevision,
    is_enabled: bool,
    summary: AiSummaryConfigRequest,
    translation: AiTranslationConfigRequest,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct AiSummaryConfigRequest {
    enabled: bool,
    provider_id: String,
    style: AiSummaryStyle,
    max_output_tokens: u32,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct AiTranslationConfigRequest {
    enabled: bool,
    provider_id: String,
    default_target_locale: String,
    max_output_tokens: u32,
}

struct RequiredNullableRevision(Option<u64>);

impl<'de> Deserialize<'de> for RequiredNullableRevision {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_any(RequiredNullableRevisionVisitor)
    }
}

struct RequiredNullableRevisionVisitor;

impl Visitor<'_> for RequiredNullableRevisionVisitor {
    type Value = RequiredNullableRevision;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a non-negative revision integer or null")
    }

    fn visit_unit<E>(self) -> Result<Self::Value, E> {
        Ok(RequiredNullableRevision(None))
    }

    fn visit_u64<E>(self, value: u64) -> Result<Self::Value, E> {
        Ok(RequiredNullableRevision(Some(value)))
    }
}

async fn get_config(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
) -> Result<Json<AiConfigEnvelope>, ApiError> {
    let (registry, _) = repositories(&state)?;
    let plugin_state = load_plugin_state(&registry).await?;
    let config = if matches!(plugin_state, PublicPluginState::Unavailable) {
        None
    } else {
        registry
            .get_ai_config(OFFICIAL_AI_PLUGIN_KEY, &user.id)
            .await
            .map_err(map_registry_error)?
            .as_ref()
            .map(AiConfigResponse::from_stored)
    };
    Ok(Json(AiConfigEnvelope {
        plugin_state: plugin_state.as_wire(),
        mcp_state: MCP_STATE,
        config,
    }))
}

async fn put_config(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    _csrf: CsrfGuard,
    ApiJson(request): ApiJson<PutAiConfigRequest>,
) -> Result<Json<AiConfigEnvelope>, ApiError> {
    state
        .content_mutation_limiter
        .check(&user.id)
        .map_err(map_limiter_rejection)?;
    if request.is_enabled != (request.summary.enabled || request.translation.enabled) {
        return Err(ApiError::validation()
            .with_field("isEnabled", "Value must match the enabled operations"));
    }

    let (registry, providers) = repositories(&state)?;
    let plugin_state = load_plugin_state(&registry).await?;
    if !plugin_state.is_ready() {
        return Err(ai_unavailable());
    }
    if request.summary.enabled {
        require_enabled_provider(&providers, &user.id, &request.summary.provider_id).await?;
    }
    if request.translation.enabled {
        require_enabled_provider(&providers, &user.id, &request.translation.provider_id).await?;
    }

    let config_json = serde_json::to_vec(&serde_json::json!({
        "schemaVersion": 1,
        "operations": {
            "summarize": {
                "enabled": request.summary.enabled,
                "providerId": request.summary.provider_id,
                "style": request.summary.style,
                "maxOutputTokens": request.summary.max_output_tokens,
                "mcp": {
                    "mode": "DISABLED",
                    "failurePolicy": "FAIL_OPEN",
                    "maxToolCalls": 0,
                    "tools": []
                }
            },
            "translate": {
                "enabled": request.translation.enabled,
                "providerId": request.translation.provider_id,
                "defaultTargetLocale": request.translation.default_target_locale,
                "maxOutputTokens": request.translation.max_output_tokens,
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
    .map_err(|_| ApiError::internal())?;
    let parsed = AiContentConfig::parse(&config_json).map_err(map_registry_error)?;
    let stored = registry
        .replace_ai_config(
            OFFICIAL_AI_PLUGIN_KEY,
            &user.id,
            request.expected_revision.0,
            request.is_enabled,
            parsed.canonical_json().as_bytes(),
        )
        .await
        .map_err(map_registry_error)?;

    Ok(Json(AiConfigEnvelope {
        plugin_state: PublicPluginState::Ready.as_wire(),
        mcp_state: MCP_STATE,
        config: Some(AiConfigResponse::from_stored(&stored)),
    }))
}

fn repositories(
    state: &AppState,
) -> Result<(PluginRegistryRepository, ProviderRepository), ApiError> {
    let database = state.setup.database().map_err(|_| ApiError::internal())?;
    Ok((
        PluginRegistryRepository::new(database.clone()),
        ProviderRepository::new(database, state.provider_keyring()),
    ))
}

async fn load_plugin_state(
    repository: &PluginRegistryRepository,
) -> Result<PublicPluginState, ApiError> {
    match repository.get_installation(OFFICIAL_AI_PLUGIN_KEY).await {
        Ok(installation) => Ok(match installation.system_state() {
            PluginSystemState::Enabled => PublicPluginState::Ready,
            PluginSystemState::Disabled => PublicPluginState::Disabled,
            PluginSystemState::Quarantined => PublicPluginState::Quarantined,
        }),
        Err(error) if error.kind() == PluginRegistryErrorKind::NotFound => {
            Ok(PublicPluginState::Unavailable)
        }
        Err(error) => Err(map_registry_error(error)),
    }
}

async fn require_enabled_provider(
    repository: &ProviderRepository,
    user_id: &str,
    provider_id: &str,
) -> Result<(), ApiError> {
    let provider = repository
        .get_visible_for_user(provider_id, user_id)
        .await
        .map_err(map_selected_provider_error)?;
    if provider.is_enabled() {
        Ok(())
    } else {
        Err(ai_unavailable())
    }
}

fn map_selected_provider_error(error: ProviderCoreError) -> ApiError {
    match error.kind() {
        ProviderCoreErrorKind::InvalidProviderId => {
            ApiError::validation().with_field("providerId", "Identifier is invalid")
        }
        ProviderCoreErrorKind::NotFound | ProviderCoreErrorKind::ProviderDisabled => {
            ai_unavailable()
        }
        ProviderCoreErrorKind::InvalidUserId
        | ProviderCoreErrorKind::Database
        | ProviderCoreErrorKind::CorruptData => ApiError::internal(),
        ProviderCoreErrorKind::InvalidDisplayName
        | ProviderCoreErrorKind::InvalidEndpoint
        | ProviderCoreErrorKind::InvalidModel
        | ProviderCoreErrorKind::InvalidCredential
        | ProviderCoreErrorKind::UnsupportedCapability
        | ProviderCoreErrorKind::InvalidPolicy
        | ProviderCoreErrorKind::InvalidPatch
        | ProviderCoreErrorKind::RevisionConflict
        | ProviderCoreErrorKind::SecretUnavailable => ApiError::internal(),
    }
}

fn map_registry_error(error: PluginRegistryError) -> ApiError {
    match error.kind() {
        PluginRegistryErrorKind::InvalidInput
        | PluginRegistryErrorKind::InvalidJson
        | PluginRegistryErrorKind::DuplicateJsonKey
        | PluginRegistryErrorKind::PayloadTooLarge
        | PluginRegistryErrorKind::InvalidConfig => ApiError::validation(),
        PluginRegistryErrorKind::NotFound => ApiError::not_found(),
        PluginRegistryErrorKind::RevisionConflict => ApiError::new(
            StatusCode::CONFLICT,
            "REVISION_CONFLICT",
            "The resource changed; reload and try again",
        ),
        PluginRegistryErrorKind::InvalidManifest
        | PluginRegistryErrorKind::ComponentDigestMismatch
        | PluginRegistryErrorKind::UnknownSigningKey
        | PluginRegistryErrorKind::InvalidSignature
        | PluginRegistryErrorKind::InvalidArtifact
        | PluginRegistryErrorKind::InvalidLifecycleEvent
        | PluginRegistryErrorKind::QuotaExceeded
        | PluginRegistryErrorKind::CorruptData
        | PluginRegistryErrorKind::Database => ApiError::internal(),
    }
}

fn ai_unavailable() -> ApiError {
    ApiError::new(
        StatusCode::CONFLICT,
        "AI_UNAVAILABLE",
        "AI content is temporarily unavailable",
    )
}

fn map_limiter_rejection(rejection: RateLimitRejection) -> ApiError {
    let retry_at = rejection.retry_at.to_offset(time::UtcOffset::UTC).format(
        time::macros::format_description!(
            "[year]-[month]-[day]T[hour]:[minute]:[second].[subsecond digits:6]Z"
        ),
    );
    match retry_at {
        Ok(retry_at) => ApiError::rate_limited_with_retry(retry_at, rejection.retry_after_seconds),
        Err(_) => ApiError::internal(),
    }
}
