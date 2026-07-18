use axum::{
    Json, Router,
    extract::State,
    http::{HeaderMap, header::ACCEPT_LANGUAGE},
    middleware,
    routing::get,
};
use serde::{Deserialize, Serialize};
use time::{OffsetDateTime, UtcOffset, macros::format_description};

use crate::{
    app::AppState,
    auth::{CsrfGuard, CurrentUser},
    preferences::{
        LayoutDensity, Locale, PreferenceError, PreferenceRepository, ThemeMode,
        UpdateUserPreferences, UserPreferences,
    },
};

use super::{ApiError, ApiJson, RateLimitRejection, routes::sensitive_cache_headers};

const PUBLIC_TIME_FORMAT: &[time::format_description::FormatItem<'static>] =
    format_description!("[year]-[month]-[day]T[hour]:[minute]:[second].[subsecond digits:6]Z");

pub(super) fn router() -> Router<AppState> {
    let preferences = Router::new()
        .route("/", get(get_preferences).patch(patch_preferences))
        .fallback(preference_not_found)
        .method_not_allowed_fallback(preference_method_not_allowed);
    Router::new()
        .route(
            "/api/v1/preferences/",
            axum::routing::any(preference_not_found),
        )
        .nest("/api/v1/preferences", preferences)
        .layer(middleware::map_response(sensitive_cache_headers))
}

async fn preference_not_found() -> ApiError {
    ApiError::not_found()
}

async fn preference_method_not_allowed() -> ApiError {
    ApiError::method_not_allowed()
}

#[derive(Deserialize)]
enum LocaleRequest {
    #[serde(rename = "zh-CN")]
    ZhCn,
    #[serde(rename = "en")]
    En,
}

impl From<LocaleRequest> for Locale {
    fn from(value: LocaleRequest) -> Self {
        match value {
            LocaleRequest::ZhCn => Self::ZhCn,
            LocaleRequest::En => Self::En,
        }
    }
}

#[derive(Deserialize)]
enum ThemeModeRequest {
    #[serde(rename = "SYSTEM")]
    System,
    #[serde(rename = "LIGHT")]
    Light,
    #[serde(rename = "DARK")]
    Dark,
}

impl From<ThemeModeRequest> for ThemeMode {
    fn from(value: ThemeModeRequest) -> Self {
        match value {
            ThemeModeRequest::System => Self::System,
            ThemeModeRequest::Light => Self::Light,
            ThemeModeRequest::Dark => Self::Dark,
        }
    }
}

#[derive(Deserialize)]
enum LayoutDensityRequest {
    #[serde(rename = "COMPACT")]
    Compact,
    #[serde(rename = "BALANCED")]
    Balanced,
    #[serde(rename = "SPACIOUS")]
    Spacious,
}

impl From<LayoutDensityRequest> for LayoutDensity {
    fn from(value: LayoutDensityRequest) -> Self {
        match value {
            LayoutDensityRequest::Compact => Self::Compact,
            LayoutDensityRequest::Balanced => Self::Balanced,
            LayoutDensityRequest::Spacious => Self::Spacious,
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PatchPreferencesRequest {
    #[serde(default, deserialize_with = "deserialize_present")]
    locale: Option<LocaleRequest>,
    #[serde(default, deserialize_with = "deserialize_present")]
    theme_mode: Option<ThemeModeRequest>,
    #[serde(default, deserialize_with = "deserialize_present")]
    layout_density: Option<LayoutDensityRequest>,
    #[serde(default, deserialize_with = "deserialize_present")]
    reading_font_scale: Option<i32>,
}

fn deserialize_present<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    T::deserialize(deserializer).map(Some)
}

impl From<PatchPreferencesRequest> for UpdateUserPreferences {
    fn from(request: PatchPreferencesRequest) -> Self {
        Self {
            locale: request.locale.map(Into::into),
            theme_mode: request.theme_mode.map(Into::into),
            layout_density: request.layout_density.map(Into::into),
            reading_font_scale: request.reading_font_scale,
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PreferencesResponse {
    locale: &'static str,
    theme_mode: &'static str,
    layout_density: &'static str,
    reading_font_scale: i32,
}

impl From<UserPreferences> for PreferencesResponse {
    fn from(preferences: UserPreferences) -> Self {
        Self {
            locale: preferences.locale.as_str(),
            theme_mode: preferences.theme_mode.as_str(),
            layout_density: preferences.layout_density.as_str(),
            reading_font_scale: preferences.reading_font_scale,
        }
    }
}

async fn get_preferences(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    headers: HeaderMap,
) -> Result<Json<PreferencesResponse>, ApiError> {
    let preferences = repository(&state)?
        .get(&user.id, default_locale(&headers))
        .await
        .map_err(map_preference_error)?;
    Ok(Json(preferences.into()))
}

async fn patch_preferences(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    _csrf: CsrfGuard,
    headers: HeaderMap,
    ApiJson(request): ApiJson<PatchPreferencesRequest>,
) -> Result<Json<PreferencesResponse>, ApiError> {
    state
        .preferences_mutation_limiter
        .check(&user.id)
        .map_err(map_limiter_rejection)?;
    let preferences = repository(&state)?
        .update(&user.id, default_locale(&headers), request.into())
        .await
        .map_err(map_preference_error)?;
    Ok(Json(preferences.into()))
}

fn repository(state: &AppState) -> Result<PreferenceRepository, ApiError> {
    state
        .setup
        .database()
        .map(PreferenceRepository::new)
        .map_err(|_| ApiError::internal())
}

fn default_locale(headers: &HeaderMap) -> Locale {
    let Some(value) = headers.get(ACCEPT_LANGUAGE) else {
        return Locale::En;
    };
    let Ok(value) = value.to_str() else {
        return Locale::En;
    };
    let language_range = value
        .split(',')
        .next()
        .unwrap_or_default()
        .split(';')
        .next()
        .unwrap_or_default()
        .trim();
    if valid_language_range(language_range)
        && language_range
            .split('-')
            .next()
            .is_some_and(|language| language.eq_ignore_ascii_case("zh"))
    {
        Locale::ZhCn
    } else {
        Locale::En
    }
}

fn valid_language_range(value: &str) -> bool {
    !value.is_empty()
        && value.split('-').all(|subtag| {
            (1..=8).contains(&subtag.len())
                && subtag.bytes().all(|byte| byte.is_ascii_alphanumeric())
        })
}

fn map_preference_error(error: PreferenceError) -> ApiError {
    match error {
        PreferenceError::InvalidPatch => ApiError::validation(),
        PreferenceError::InvalidFontScale => ApiError::validation().with_field(
            "readingFontScale",
            "Reading font scale must be between 85 and 130",
        ),
        PreferenceError::Database(_)
        | PreferenceError::InvalidUserId
        | PreferenceError::UserUnavailable
        | PreferenceError::CorruptData => ApiError::internal(),
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
