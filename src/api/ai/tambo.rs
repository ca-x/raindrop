use axum::{
    Json, Router,
    body::Bytes,
    extract::{FromRequestParts, OriginalUri, Path, State},
    http::{Method, StatusCode, request::Parts},
    middleware,
    response::Response,
    routing::{any, get},
};
use serde::{Deserialize, Serialize};

use crate::{
    app::AppState,
    auth::{CsrfGuard, CurrentUser},
    plugins::{
        PluginRegistryError, PluginRegistryErrorKind, PluginRegistryRepository, PluginSystemState,
        TamboPreferences,
    },
};

use super::super::{ApiError, ApiJson, routes::sensitive_cache_headers};
use super::config::RequiredNullableRevision;

pub(super) fn router() -> Router<AppState> {
    Router::new()
        .route("/api/v1/ai/tambo", get(status).put(save_settings))
        .route("/api/v1/ai/tambo/api/{*path}", any(proxy))
        .layer(middleware::map_response(sensitive_cache_headers))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct TamboStatus {
    configured: bool,
    origin: Option<String>,
    #[serde(flatten)]
    preferences: TamboPreferences,
}

async fn status(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
) -> Result<Json<TamboStatus>, ApiError> {
    let registry = repository(&state)?;
    let preferences = match registry.get_tambo_preferences(&user.id).await {
        Ok(preferences) => preferences,
        Err(error) if error.kind() == PluginRegistryErrorKind::NotFound => {
            TamboPreferences::default()
        }
        Err(error) => return Err(map_storage_error(error)),
    };
    Ok(Json(TamboStatus {
        configured: state.tambo.is_some(),
        origin: state.tambo.as_ref().map(|client| client.origin()),
        preferences,
    }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct SaveSettings {
    expected_revision: RequiredNullableRevision,
    enabled: bool,
}

async fn save_settings(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    _csrf: CsrfGuard,
    ApiJson(request): ApiJson<SaveSettings>,
) -> Result<Json<TamboStatus>, ApiError> {
    check_rate_limit(&state, &user.id)?;
    let registry = repository(&state)?;
    if request.enabled {
        if state.tambo.is_none() {
            return Err(disabled());
        }
        let installation = registry
            .get_installation(raindrop_ai_content::PLUGIN_KEY)
            .await
            .map_err(map_storage_error)?;
        if installation.system_state() != PluginSystemState::Enabled {
            return Err(disabled());
        }
    }
    let preferences = registry
        .replace_tambo_preferences(&user.id, request.expected_revision.0, request.enabled)
        .await
        .map_err(map_storage_error)?;
    Ok(Json(TamboStatus {
        configured: state.tambo.is_some(),
        origin: state.tambo.as_ref().map(|client| client.origin()),
        preferences,
    }))
}

struct TamboCsrf;

impl FromRequestParts<AppState> for TamboCsrf {
    type Rejection = crate::auth::AuthRejection;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        if parts.method != Method::GET {
            let mut headers = parts.headers.get_all("x-api-key").iter();
            let csrf = headers
                .next()
                .cloned()
                .ok_or(crate::auth::AuthRejection::Forbidden)?;
            if headers.next().is_some() {
                return Err(crate::auth::AuthRejection::Forbidden);
            }
            parts.headers.insert("x-csrf-token", csrf);
            CsrfGuard::from_request_parts(parts, state).await?;
        }
        Ok(Self)
    }
}

async fn proxy(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    _csrf: TamboCsrf,
    Path(path): Path<String>,
    OriginalUri(uri): OriginalUri,
    method: Method,
    body: Bytes,
) -> Result<Response, ApiError> {
    let client = state.tambo.as_ref().ok_or_else(disabled)?;
    check_rate_limit(&state, &user.id)?;
    let registry = repository(&state)?;
    let installation = registry
        .get_installation(raindrop_ai_content::PLUGIN_KEY)
        .await
        .map_err(map_storage_error)?;
    if installation.system_state() != PluginSystemState::Enabled
        || !registry
            .get_tambo_preferences(&user.id)
            .await
            .map_err(map_storage_error)?
            .enabled
    {
        return Err(disabled());
    }
    client
        .forward(&user.id, method, &path, uri.query(), body)
        .await
}

fn check_rate_limit(state: &AppState, user_id: &str) -> Result<(), ApiError> {
    state
        .tambo_mutation_limiter
        .check(user_id)
        .map_err(|rejection| {
            ApiError::new(
                StatusCode::TOO_MANY_REQUESTS,
                "TAMBO_BUSY",
                "The optional assistant is busy",
            )
            .with_header(
                axum::http::header::RETRY_AFTER,
                axum::http::HeaderValue::from_str(&rejection.retry_after_seconds.to_string())
                    .unwrap_or_else(|_| axum::http::HeaderValue::from_static("30")),
            )
        })
}

fn repository(state: &AppState) -> Result<PluginRegistryRepository, ApiError> {
    state
        .setup
        .database()
        .map(PluginRegistryRepository::new)
        .map_err(|_| ApiError::internal())
}

fn disabled() -> ApiError {
    ApiError::new(
        StatusCode::SERVICE_UNAVAILABLE,
        "TAMBO_DISABLED",
        "The optional assistant is disabled",
    )
}

fn map_storage_error(error: PluginRegistryError) -> ApiError {
    match error.kind() {
        PluginRegistryErrorKind::RevisionConflict => ApiError::new(
            StatusCode::CONFLICT,
            "TAMBO_CONFLICT",
            "Assistant settings changed; reload before saving",
        ),
        PluginRegistryErrorKind::NotFound => disabled(),
        _ => ApiError::internal(),
    }
}
