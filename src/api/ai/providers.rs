use std::fmt;

use axum::{
    Json, Router,
    extract::{FromRequestParts, Path, State},
    http::{HeaderValue, StatusCode, header::LOCATION, request::Parts},
    middleware,
    response::{IntoResponse, Response},
    routing::get,
};
use secrecy::SecretString;
use serde::{Deserialize, Deserializer, Serialize, de::DeserializeOwned, de::Visitor};
use time::{OffsetDateTime, UtcOffset, macros::format_description};
use uuid::Uuid;

use crate::{
    app::AppState,
    auth::{CsrfGuard, CurrentUser},
    content::provider::{
        CreateProvider, ProviderCapabilities, ProviderCoreError, ProviderCoreErrorKind,
        ProviderKind, ProviderMetadata, ProviderPolicy, ProviderRepository, ProviderScope,
        UpdateProvider,
    },
};

use super::super::{ApiError, ApiJson, RateLimitRejection, routes::sensitive_cache_headers};

const USER_PROVIDER_LIMIT: u64 = 32;
const PUBLIC_TIME_FORMAT: &[time::format_description::FormatItem<'static>] =
    format_description!("[year]-[month]-[day]T[hour]:[minute]:[second].[subsecond digits:6]Z");

pub(super) fn router() -> Router<AppState> {
    let providers = Router::new()
        .route("/", get(list_providers).post(create_provider))
        .route("/{provider_id}", get(get_provider).patch(update_provider))
        .fallback(provider_not_found)
        .method_not_allowed_fallback(provider_method_not_allowed);
    Router::new()
        .route(
            "/api/v1/ai/providers/",
            axum::routing::any(provider_not_found),
        )
        .nest("/api/v1/ai/providers", providers)
        .layer(middleware::map_response(sensitive_cache_headers))
}

async fn provider_not_found() -> ApiError {
    ApiError::not_found()
}

async fn provider_method_not_allowed() -> ApiError {
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

#[derive(Clone, Copy, Deserialize)]
enum ProviderKindRequest {
    #[serde(rename = "ANTHROPIC_MESSAGES")]
    AnthropicMessages,
    #[serde(rename = "OPENAI_RESPONSES")]
    OpenAiResponses,
    #[serde(rename = "OPENAI_CHAT_COMPLETIONS")]
    OpenAiChatCompletions,
    #[serde(rename = "GOOGLE_GEMINI")]
    GoogleGemini,
}

impl From<ProviderKindRequest> for ProviderKind {
    fn from(value: ProviderKindRequest) -> Self {
        match value {
            ProviderKindRequest::AnthropicMessages => Self::AnthropicMessages,
            ProviderKindRequest::OpenAiResponses => Self::OpenAiResponses,
            ProviderKindRequest::OpenAiChatCompletions => Self::OpenAiChatCompletions,
            ProviderKindRequest::GoogleGemini => Self::GoogleGemini,
        }
    }
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ProviderCapabilitiesRequest {
    supports_usage: bool,
    supports_idempotency: bool,
}

impl From<ProviderCapabilitiesRequest> for ProviderCapabilities {
    fn from(value: ProviderCapabilitiesRequest) -> Self {
        Self {
            supports_usage: value.supports_usage,
            supports_idempotency: value.supports_idempotency,
            supports_streaming: false,
        }
    }
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ProviderPolicyRequest {
    max_concurrency: u16,
    requests_per_minute: Option<u32>,
    max_input_tokens_per_request: u32,
    max_output_tokens_per_request: u32,
    input_cost_micros_per_million_tokens: Option<u64>,
    output_cost_micros_per_million_tokens: Option<u64>,
    max_cost_micros_per_request: Option<u64>,
}

impl From<ProviderPolicyRequest> for ProviderPolicy {
    fn from(value: ProviderPolicyRequest) -> Self {
        Self {
            max_concurrency: value.max_concurrency,
            requests_per_minute: value.requests_per_minute,
            max_input_tokens_per_request: value.max_input_tokens_per_request,
            max_output_tokens_per_request: value.max_output_tokens_per_request,
            input_cost_micros_per_million_tokens: value.input_cost_micros_per_million_tokens,
            output_cost_micros_per_million_tokens: value.output_cost_micros_per_million_tokens,
            max_cost_micros_per_request: value.max_cost_micros_per_request,
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CreateProviderRequest {
    display_name: String,
    kind: ProviderKindRequest,
    endpoint: NullableEndpointRequest,
    model: String,
    credential: String,
    capabilities: ProviderCapabilitiesRequest,
    policy: ProviderPolicyRequest,
    is_enabled: bool,
}

struct NullableEndpointRequest(Option<String>);

impl<'de> Deserialize<'de> for NullableEndpointRequest {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_any(NullableEndpointVisitor)
    }
}

struct NullableEndpointVisitor;

impl Visitor<'_> for NullableEndpointVisitor {
    type Value = NullableEndpointRequest;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("an HTTPS endpoint string or null")
    }

    fn visit_unit<E>(self) -> Result<Self::Value, E> {
        Ok(NullableEndpointRequest(None))
    }

    fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        Ok(NullableEndpointRequest(Some(value.to_owned())))
    }

    fn visit_string<E>(self, value: String) -> Result<Self::Value, E> {
        Ok(NullableEndpointRequest(Some(value)))
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct UpdateProviderRequest {
    expected_revision: u64,
    #[serde(default, deserialize_with = "deserialize_present")]
    display_name: Option<String>,
    #[serde(default, deserialize_with = "deserialize_present")]
    endpoint: Option<String>,
    #[serde(default, deserialize_with = "deserialize_present")]
    model: Option<String>,
    credential: Option<String>,
    #[serde(default, deserialize_with = "deserialize_present")]
    capabilities: Option<ProviderCapabilitiesRequest>,
    #[serde(default, deserialize_with = "deserialize_present")]
    policy: Option<ProviderPolicyRequest>,
    #[serde(default, deserialize_with = "deserialize_present")]
    is_enabled: Option<bool>,
}

fn deserialize_present<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    T::deserialize(deserializer).map(Some)
}

impl UpdateProviderRequest {
    fn into_domain(self) -> UpdateProvider {
        UpdateProvider {
            expected_revision: self.expected_revision,
            display_name: self.display_name,
            endpoint: self.endpoint,
            model: self.model,
            credential: self.credential.map(SecretString::from),
            capabilities: self.capabilities.map(Into::into),
            policy: self.policy.map(Into::into),
            is_enabled: self.is_enabled,
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ProviderListResponse {
    keyring_status: &'static str,
    items: Vec<ProviderResponse>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ProviderResponse {
    provider_id: String,
    scope: &'static str,
    can_edit: bool,
    display_name: String,
    kind: &'static str,
    endpoint: String,
    model: String,
    capabilities: ProviderCapabilitiesResponse,
    policy: ProviderPolicyResponse,
    is_enabled: bool,
    revision: u64,
    created_at: String,
    updated_at: String,
}

impl ProviderResponse {
    fn from_metadata(provider: ProviderMetadata) -> Result<Self, ApiError> {
        let (scope, can_edit) = match provider.scope() {
            ProviderScope::Instance => ("INSTANCE", false),
            ProviderScope::User(_) => ("USER", true),
        };
        let capabilities = provider.capabilities();
        let policy = provider.policy();
        Ok(Self {
            provider_id: provider.id().to_owned(),
            scope,
            can_edit,
            display_name: provider.display_name().to_owned(),
            kind: provider.kind().as_storage(),
            endpoint: provider.endpoint().as_str().to_owned(),
            model: provider.model().to_owned(),
            capabilities: ProviderCapabilitiesResponse {
                supports_usage: capabilities.supports_usage,
                supports_idempotency: capabilities.supports_idempotency,
            },
            policy: policy.into(),
            is_enabled: provider.is_enabled(),
            revision: provider.revision(),
            created_at: format_public_time(provider.created_at())?,
            updated_at: format_public_time(provider.updated_at())?,
        })
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ProviderCapabilitiesResponse {
    supports_usage: bool,
    supports_idempotency: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ProviderPolicyResponse {
    max_concurrency: u16,
    requests_per_minute: Option<u32>,
    max_input_tokens_per_request: u32,
    max_output_tokens_per_request: u32,
    input_cost_micros_per_million_tokens: Option<u64>,
    output_cost_micros_per_million_tokens: Option<u64>,
    max_cost_micros_per_request: Option<u64>,
}

impl From<ProviderPolicy> for ProviderPolicyResponse {
    fn from(value: ProviderPolicy) -> Self {
        Self {
            max_concurrency: value.max_concurrency,
            requests_per_minute: value.requests_per_minute,
            max_input_tokens_per_request: value.max_input_tokens_per_request,
            max_output_tokens_per_request: value.max_output_tokens_per_request,
            input_cost_micros_per_million_tokens: value.input_cost_micros_per_million_tokens,
            output_cost_micros_per_million_tokens: value.output_cost_micros_per_million_tokens,
            max_cost_micros_per_request: value.max_cost_micros_per_request,
        }
    }
}

async fn list_providers(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
) -> Result<Json<ProviderListResponse>, ApiError> {
    let items = repository(&state)?
        .list_for_user(&user.id)
        .await
        .map_err(map_provider_error)?
        .into_iter()
        .map(ProviderResponse::from_metadata)
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Json(ProviderListResponse {
        keyring_status: if state.provider_keyring().is_some() {
            "AVAILABLE"
        } else {
            "UNAVAILABLE"
        },
        items,
    }))
}

async fn get_provider(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    ApiPath(provider_id): ApiPath<String>,
) -> Result<Json<ProviderResponse>, ApiError> {
    validate_canonical_uuid(&provider_id)?;
    let provider = repository(&state)?
        .get_visible_for_user(&provider_id, &user.id)
        .await
        .map_err(map_provider_error)?;
    Ok(Json(ProviderResponse::from_metadata(provider)?))
}

async fn create_provider(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    _csrf: CsrfGuard,
    ApiJson(request): ApiJson<CreateProviderRequest>,
) -> Result<Response, ApiError> {
    state
        .provider_mutation_limiter
        .check(&user.id)
        .map_err(map_limiter_rejection)?;
    let repository = repository(&state)?;
    if repository
        .count_user_owned(&user.id)
        .await
        .map_err(map_provider_error)?
        >= USER_PROVIDER_LIMIT
    {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "PROVIDER_LIMIT_REACHED",
            "AI provider limit reached",
        ));
    }
    let provider = repository
        .create(CreateProvider {
            scope: ProviderScope::user(user.id).map_err(map_provider_error)?,
            display_name: request.display_name,
            kind: request.kind.into(),
            endpoint: request.endpoint.0,
            model: request.model,
            credential: SecretString::from(request.credential),
            capabilities: request.capabilities.into(),
            policy: request.policy.into(),
            is_enabled: request.is_enabled,
        })
        .await
        .map_err(map_provider_error)?;
    let location = HeaderValue::from_str(&format!("/api/v1/ai/providers/{}", provider.id()))
        .map_err(|_| ApiError::internal())?;
    let response = ProviderResponse::from_metadata(provider)?;
    Ok((StatusCode::CREATED, [(LOCATION, location)], Json(response)).into_response())
}

async fn update_provider(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    _csrf: CsrfGuard,
    ApiPath(provider_id): ApiPath<String>,
    ApiJson(request): ApiJson<UpdateProviderRequest>,
) -> Result<Json<ProviderResponse>, ApiError> {
    validate_canonical_uuid(&provider_id)?;
    state
        .provider_mutation_limiter
        .check(&user.id)
        .map_err(map_limiter_rejection)?;
    let scope = ProviderScope::user(user.id).map_err(map_provider_error)?;
    let provider = repository(&state)?
        .update(&provider_id, &scope, request.into_domain())
        .await
        .map_err(map_provider_error)?;
    Ok(Json(ProviderResponse::from_metadata(provider)?))
}

fn repository(state: &AppState) -> Result<ProviderRepository, ApiError> {
    state
        .setup
        .database()
        .map(|database| ProviderRepository::new(database, state.provider_keyring()))
        .map_err(|_| ApiError::internal())
}

fn validate_canonical_uuid(value: &str) -> Result<(), ApiError> {
    let parsed = Uuid::parse_str(value)
        .map_err(|_| ApiError::validation().with_field("providerId", "Identifier is invalid"))?;
    if parsed.to_string() == value {
        Ok(())
    } else {
        Err(ApiError::validation().with_field("providerId", "Identifier is invalid"))
    }
}

fn map_provider_error(error: ProviderCoreError) -> ApiError {
    match error.kind() {
        ProviderCoreErrorKind::InvalidProviderId => {
            ApiError::validation().with_field("providerId", "Identifier is invalid")
        }
        ProviderCoreErrorKind::InvalidDisplayName => {
            ApiError::validation().with_field("displayName", "Display name is invalid")
        }
        ProviderCoreErrorKind::InvalidEndpoint => {
            ApiError::validation().with_field("endpoint", "Endpoint is invalid")
        }
        ProviderCoreErrorKind::InvalidModel => {
            ApiError::validation().with_field("model", "Model is invalid")
        }
        ProviderCoreErrorKind::InvalidCredential => {
            ApiError::validation().with_field("credential", "Credential is invalid")
        }
        ProviderCoreErrorKind::UnsupportedCapability => ApiError::validation()
            .with_field("capabilities", "Provider capabilities are not supported"),
        ProviderCoreErrorKind::InvalidPolicy => {
            ApiError::validation().with_field("policy", "Provider policy is invalid")
        }
        ProviderCoreErrorKind::InvalidPatch => ApiError::validation(),
        ProviderCoreErrorKind::NotFound | ProviderCoreErrorKind::ProviderDisabled => {
            ApiError::not_found()
        }
        ProviderCoreErrorKind::RevisionConflict => ApiError::new(
            StatusCode::CONFLICT,
            "REVISION_CONFLICT",
            "The resource changed; reload and try again",
        ),
        ProviderCoreErrorKind::SecretUnavailable => ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "AI_PROVIDER_KEYRING_UNAVAILABLE",
            "AI provider credentials are unavailable",
        ),
        ProviderCoreErrorKind::InvalidUserId
        | ProviderCoreErrorKind::Database
        | ProviderCoreErrorKind::CorruptData => ApiError::internal(),
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
