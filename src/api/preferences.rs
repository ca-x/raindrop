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
        LayoutDensity, LinkOpenMode, Locale, PreferenceError, PreferenceRepository,
        ReadingColorScheme, ReadingFontFamily, ThemeMode, UpdateUserPreferences, UserPreferences,
    },
};

use super::{ApiError, ApiJson, RateLimitRejection, routes::sensitive_cache_headers};

mod fonts;

const PUBLIC_TIME_FORMAT: &[time::format_description::FormatItem<'static>] =
    format_description!("[year]-[month]-[day]T[hour]:[minute]:[second].[subsecond digits:6]Z");

pub(super) fn router() -> Router<AppState> {
    let preferences_v1 = Router::new()
        .route("/", get(get_preferences_v1).patch(patch_preferences_v1))
        .fallback(preference_not_found)
        .method_not_allowed_fallback(preference_method_not_allowed);
    let preferences_v2 = Router::new()
        .route("/", get(get_preferences_v2).patch(patch_preferences_v2))
        .fallback(preference_not_found)
        .method_not_allowed_fallback(preference_method_not_allowed);
    let sensitive = Router::new()
        .route(
            "/api/v1/preferences/",
            axum::routing::any(preference_not_found),
        )
        .route(
            "/api/v2/preferences/",
            axum::routing::any(preference_not_found),
        )
        .nest("/api/v1/preferences", preferences_v1)
        .nest("/api/v2/preferences", preferences_v2)
        .merge(fonts::command_router())
        .layer(middleware::map_response(sensitive_cache_headers));
    sensitive.merge(fonts::file_router())
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
enum ReadingFontFamilyRequest {
    #[serde(rename = "SERIF")]
    Serif,
    #[serde(rename = "SANS")]
    Sans,
}

impl From<ReadingFontFamilyRequest> for ReadingFontFamily {
    fn from(value: ReadingFontFamilyRequest) -> Self {
        match value {
            ReadingFontFamilyRequest::Serif => Self::Serif,
            ReadingFontFamilyRequest::Sans => Self::Sans,
        }
    }
}

#[derive(Deserialize)]
enum ReadingColorSchemeRequest {
    #[serde(rename = "AUTO")]
    Auto,
    #[serde(rename = "PAPER")]
    Paper,
    #[serde(rename = "SEPIA")]
    Sepia,
    #[serde(rename = "GRAY")]
    Gray,
}

impl From<ReadingColorSchemeRequest> for ReadingColorScheme {
    fn from(value: ReadingColorSchemeRequest) -> Self {
        match value {
            ReadingColorSchemeRequest::Auto => Self::Auto,
            ReadingColorSchemeRequest::Paper => Self::Paper,
            ReadingColorSchemeRequest::Sepia => Self::Sepia,
            ReadingColorSchemeRequest::Gray => Self::Gray,
        }
    }
}

#[derive(Deserialize)]
enum LinkOpenModeRequest {
    #[serde(rename = "CURRENT_TAB")]
    CurrentTab,
    #[serde(rename = "NEW_TAB")]
    NewTab,
}

impl From<LinkOpenModeRequest> for LinkOpenMode {
    fn from(value: LinkOpenModeRequest) -> Self {
        match value {
            LinkOpenModeRequest::CurrentTab => Self::CurrentTab,
            LinkOpenModeRequest::NewTab => Self::NewTab,
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
    #[serde(default, deserialize_with = "deserialize_present")]
    reading_font_family: Option<ReadingFontFamilyRequest>,
    #[serde(default, deserialize_with = "deserialize_present")]
    reading_custom_font_id: Option<Option<String>>,
    #[serde(default, deserialize_with = "deserialize_present")]
    reading_color_scheme: Option<ReadingColorSchemeRequest>,
    #[serde(default, deserialize_with = "deserialize_present")]
    link_open_mode: Option<LinkOpenModeRequest>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PatchPreferencesV1Request {
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
            reading_font_family: request.reading_font_family.map(Into::into),
            reading_custom_font_id: request.reading_custom_font_id,
            reading_color_scheme: request.reading_color_scheme.map(Into::into),
            link_open_mode: request.link_open_mode.map(Into::into),
        }
    }
}

impl From<PatchPreferencesV1Request> for UpdateUserPreferences {
    fn from(request: PatchPreferencesV1Request) -> Self {
        Self {
            locale: request.locale.map(Into::into),
            theme_mode: request.theme_mode.map(Into::into),
            layout_density: request.layout_density.map(Into::into),
            reading_font_scale: request.reading_font_scale,
            ..Self::default()
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PreferencesV1Response {
    locale: &'static str,
    theme_mode: &'static str,
    layout_density: &'static str,
    reading_font_scale: i32,
}

impl From<UserPreferences> for PreferencesV1Response {
    fn from(preferences: UserPreferences) -> Self {
        Self {
            locale: preferences.locale.as_str(),
            theme_mode: preferences.theme_mode.as_str(),
            layout_density: preferences.layout_density.as_str(),
            reading_font_scale: preferences.reading_font_scale,
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PreferencesV2Response {
    locale: &'static str,
    theme_mode: &'static str,
    layout_density: &'static str,
    reading_font_scale: i32,
    reading_font_family: &'static str,
    reading_custom_font_id: Option<String>,
    reading_color_scheme: &'static str,
    link_open_mode: &'static str,
}

impl From<UserPreferences> for PreferencesV2Response {
    fn from(preferences: UserPreferences) -> Self {
        Self {
            locale: preferences.locale.as_str(),
            theme_mode: preferences.theme_mode.as_str(),
            layout_density: preferences.layout_density.as_str(),
            reading_font_scale: preferences.reading_font_scale,
            reading_font_family: preferences.reading_font_family.as_str(),
            reading_custom_font_id: preferences.reading_custom_font_id,
            reading_color_scheme: preferences.reading_color_scheme.as_str(),
            link_open_mode: preferences.link_open_mode.as_str(),
        }
    }
}

async fn get_preferences_v1(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    headers: HeaderMap,
) -> Result<Json<PreferencesV1Response>, ApiError> {
    let preferences = get_user_preferences(&state, &user.id, &headers).await?;
    Ok(Json(preferences.into()))
}

async fn get_preferences_v2(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    headers: HeaderMap,
) -> Result<Json<PreferencesV2Response>, ApiError> {
    let preferences = get_user_preferences(&state, &user.id, &headers).await?;
    Ok(Json(preferences.into()))
}

async fn patch_preferences_v1(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    _csrf: CsrfGuard,
    headers: HeaderMap,
    ApiJson(request): ApiJson<PatchPreferencesV1Request>,
) -> Result<Json<PreferencesV1Response>, ApiError> {
    let preferences = update_user_preferences(&state, &user.id, &headers, request.into()).await?;
    Ok(Json(preferences.into()))
}

async fn patch_preferences_v2(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    _csrf: CsrfGuard,
    headers: HeaderMap,
    ApiJson(request): ApiJson<PatchPreferencesRequest>,
) -> Result<Json<PreferencesV2Response>, ApiError> {
    let preferences = update_user_preferences(&state, &user.id, &headers, request.into()).await?;
    Ok(Json(preferences.into()))
}

async fn get_user_preferences(
    state: &AppState,
    user_id: &str,
    headers: &HeaderMap,
) -> Result<UserPreferences, ApiError> {
    repository(state)?
        .get(user_id, default_locale(headers))
        .await
        .map_err(map_preference_error)
}

async fn update_user_preferences(
    state: &AppState,
    user_id: &str,
    headers: &HeaderMap,
    request: UpdateUserPreferences,
) -> Result<UserPreferences, ApiError> {
    state
        .preferences_mutation_limiter
        .check(user_id)
        .map_err(map_limiter_rejection)?;
    repository(state)?
        .update(user_id, default_locale(headers), request)
        .await
        .map_err(map_preference_error)
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
        PreferenceError::InvalidCustomFont => ApiError::validation().with_field(
            "readingCustomFontId",
            "Custom font must belong to the current user",
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
