use axum::{Json, Router, extract::State, middleware, routing::get};
use serde::{Deserialize, Serialize};

use crate::{
    app::AppState,
    auth::{
        CsrfGuard, CurrentUser, ProfileError, UpdateUserProfile, UserProfile, load_user_profile,
        update_user_profile,
    },
};

use super::{ApiError, ApiJson, RateLimitRejection, routes::sensitive_cache_headers};

pub(super) fn router() -> Router<AppState> {
    let profile = Router::new()
        .route("/", get(get_profile).patch(patch_profile))
        .fallback(profile_not_found)
        .method_not_allowed_fallback(profile_method_not_allowed);
    Router::new()
        .route("/api/v2/profile/", axum::routing::any(profile_not_found))
        .nest("/api/v2/profile", profile)
        .layer(middleware::map_response(sensitive_cache_headers))
}

async fn profile_not_found() -> ApiError {
    ApiError::not_found()
}

async fn profile_method_not_allowed() -> ApiError {
    ApiError::method_not_allowed()
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ProfileResponse {
    user_id: String,
    username: String,
    display_name: Option<String>,
    email: Option<String>,
}

impl From<UserProfile> for ProfileResponse {
    fn from(profile: UserProfile) -> Self {
        Self {
            user_id: profile.user_id,
            username: profile.username,
            display_name: profile.display_name,
            email: profile.email,
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PatchProfileRequest {
    #[serde(default, deserialize_with = "deserialize_present")]
    display_name: Option<Option<String>>,
    #[serde(default, deserialize_with = "deserialize_present")]
    email: Option<Option<String>>,
}

fn deserialize_present<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    T::deserialize(deserializer).map(Some)
}

async fn get_profile(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
) -> Result<Json<ProfileResponse>, ApiError> {
    let database = state.setup.database().map_err(|_| ApiError::internal())?;
    let profile = load_user_profile(&database, &user.id)
        .await
        .map_err(map_profile_error)?;
    Ok(Json(profile.into()))
}

async fn patch_profile(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    _csrf: CsrfGuard,
    ApiJson(request): ApiJson<PatchProfileRequest>,
) -> Result<Json<ProfileResponse>, ApiError> {
    state
        .preferences_mutation_limiter
        .check(&user.id)
        .map_err(map_limiter_rejection)?;
    let database = state.setup.database().map_err(|_| ApiError::internal())?;
    let profile = update_user_profile(
        &database,
        &user.id,
        UpdateUserProfile {
            display_name: request.display_name,
            email: request.email,
        },
    )
    .await
    .map_err(map_profile_error)?;
    Ok(Json(profile.into()))
}

fn map_profile_error(error: ProfileError) -> ApiError {
    match error {
        ProfileError::EmptyPatch => ApiError::validation(),
        ProfileError::InvalidDisplayName(_) => ApiError::validation().with_field(
            "displayName",
            "Display name must contain at most 80 visible characters",
        ),
        ProfileError::InvalidEmail(_) => ApiError::email_invalid(),
        ProfileError::EmailTaken => ApiError::new(
            axum::http::StatusCode::CONFLICT,
            "PROFILE_EMAIL_TAKEN",
            "Email address is already used by another account",
        )
        .with_field("email", "Email address is already used"),
        ProfileError::NotFound | ProfileError::Database(_) => ApiError::internal(),
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
