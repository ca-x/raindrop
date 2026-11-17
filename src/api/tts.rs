use std::sync::Arc;

use axum::{
    Json, Router,
    extract::{Path, State},
    http::{
        StatusCode,
        header::{CONTENT_TYPE, X_CONTENT_TYPE_OPTIONS},
    },
    middleware,
    response::{IntoResponse, Response},
    routing::{get, post},
};
use serde::Deserialize;
use tokio::sync::OwnedSemaphorePermit;

use super::{ApiError, ApiJson, routes::sensitive_cache_headers};
use crate::{
    app::AppState,
    auth::{CsrfGuard, CurrentUser},
    content::provider::HttpsProviderTransport,
    feeds::FeedRepository,
    tts::{SpeechConfig, SpeechDraft, SpeechError, SpeechPlan, SpeechRepository, SpeechService},
};

pub(super) fn router() -> Router<AppState> {
    Router::new()
        .route("/api/v1/plugins/tts", get(get_config).put(put_config))
        .route("/api/v1/plugins/tts/test", post(test_connection))
        .route("/api/v1/plugins/tts/entries/{entry_id}/plan", post(plan))
        .route("/api/v1/plugins/tts/entries/{entry_id}/speak", post(speak))
        .layer(middleware::map_response(sensitive_cache_headers))
}

async fn get_config(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
) -> Result<Json<SpeechConfig>, ApiError> {
    Ok(Json(
        repository(&state, true)?
            .get(&user.id)
            .await
            .map_err(map_error)?,
    ))
}

async fn put_config(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    _csrf: CsrfGuard,
    ApiJson(draft): ApiJson<SpeechDraft>,
) -> Result<Json<SpeechConfig>, ApiError> {
    state
        .provider_mutation_limiter
        .check(&user.id)
        .map_err(|_| ApiError::rate_limited())?;
    Ok(Json(
        repository(&state, false)?
            .save(&user.id, draft)
            .await
            .map_err(map_error)?,
    ))
}

async fn test_connection(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    _csrf: CsrfGuard,
    ApiJson(draft): ApiJson<SpeechDraft>,
) -> Result<Response, ApiError> {
    let _permits = admit(&state, &user.id)?;
    let audio = service(&state)?
        .test(&user.id, draft)
        .await
        .map_err(map_error)?;
    Ok(audio_response(audio))
}

async fn plan(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    _csrf: CsrfGuard,
    Path(entry_id): Path<String>,
) -> Result<Json<SpeechPlan>, ApiError> {
    let _permits = admit(&state, &user.id)?;
    Ok(Json(
        service(&state)?
            .plan(&user.id, &entry_id)
            .await
            .map_err(map_error)?,
    ))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SpeakRequest {
    content_hash: String,
    chunk_index: usize,
}

async fn speak(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    _csrf: CsrfGuard,
    Path(entry_id): Path<String>,
    ApiJson(request): ApiJson<SpeakRequest>,
) -> Result<Response, ApiError> {
    let _permits = admit(&state, &user.id)?;
    let audio = service(&state)?
        .speak(
            &user.id,
            &entry_id,
            &request.content_hash,
            request.chunk_index,
        )
        .await
        .map_err(map_error)?;
    Ok(audio_response(audio))
}

fn audio_response(audio: Vec<u8>) -> Response {
    (
        [
            (CONTENT_TYPE, "audio/mpeg"),
            (X_CONTENT_TYPE_OPTIONS, "nosniff"),
        ],
        audio,
    )
        .into_response()
}

fn repository(state: &AppState, read_only: bool) -> Result<SpeechRepository, ApiError> {
    let database = if read_only {
        state.setup.reader_database()
    } else {
        state.setup.database()
    }
    .map_err(|_| ApiError::internal())?;
    Ok(SpeechRepository::new(database, state.provider_keyring()))
}

fn service(state: &AppState) -> Result<SpeechService, ApiError> {
    let database = state.setup.database().map_err(|_| ApiError::internal())?;
    let transport = match &state.tts_transport {
        Some(transport) => Arc::clone(transport),
        None => Arc::new(
            HttpsProviderTransport::public_only().map_err(|_| map_error(SpeechError::Upstream))?,
        ),
    };
    Ok(SpeechService::new(
        SpeechRepository::new(database.clone(), state.provider_keyring()),
        FeedRepository::new(database),
        transport,
    ))
}

fn admit(
    state: &AppState,
    user_id: &str,
) -> Result<(OwnedSemaphorePermit, OwnedSemaphorePermit), ApiError> {
    state
        .tts_limiter
        .check(user_id)
        .map_err(|_| map_error(SpeechError::RateLimited))?;
    let user = state
        .tts_user_concurrency
        .try_acquire(user_id)
        .ok_or_else(|| map_error(SpeechError::RateLimited))?;
    let global = state
        .tts_semaphore
        .clone()
        .try_acquire_owned()
        .map_err(|_| map_error(SpeechError::RateLimited))?;
    Ok((user, global))
}

fn map_error(error: SpeechError) -> ApiError {
    let (status, code) = match error {
        SpeechError::InvalidInput => (StatusCode::BAD_REQUEST, "TTS_INVALID_INPUT"),
        SpeechError::Conflict => (StatusCode::CONFLICT, "TTS_CONFLICT"),
        SpeechError::Disabled => (StatusCode::CONFLICT, "TTS_DISABLED"),
        SpeechError::NotFound => (StatusCode::NOT_FOUND, "TTS_NOT_FOUND"),
        SpeechError::KeyringUnavailable => {
            (StatusCode::SERVICE_UNAVAILABLE, "TTS_KEYRING_UNAVAILABLE")
        }
        SpeechError::UpstreamAuth => (StatusCode::BAD_GATEWAY, "TTS_UPSTREAM_AUTH"),
        SpeechError::UpstreamInvalid => (StatusCode::BAD_GATEWAY, "TTS_UPSTREAM_INVALID"),
        SpeechError::Upstream => (StatusCode::BAD_GATEWAY, "TTS_UPSTREAM"),
        SpeechError::Timeout => (StatusCode::GATEWAY_TIMEOUT, "TTS_TIMEOUT"),
        SpeechError::RateLimited => (StatusCode::TOO_MANY_REQUESTS, "TTS_RATE_LIMITED"),
        SpeechError::Storage => (StatusCode::INTERNAL_SERVER_ERROR, "TTS_STORAGE"),
    };
    let message = match error {
        SpeechError::InvalidInput => "Invalid speech configuration or input",
        SpeechError::Conflict => "Speech configuration or article changed; reload before retrying",
        SpeechError::Disabled => "Speech playback is disabled",
        SpeechError::NotFound => "Article not found",
        SpeechError::KeyringUnavailable => "Speech credential encryption is unavailable",
        SpeechError::UpstreamAuth => "Speech provider authentication failed",
        SpeechError::UpstreamInvalid => "Speech provider returned invalid audio",
        SpeechError::Upstream => "Speech provider is unavailable",
        SpeechError::Timeout => "Speech provider timed out",
        SpeechError::RateLimited => "Too many speech requests",
        SpeechError::Storage => "Speech storage is unavailable",
    };
    ApiError::new(status, code, message)
}
