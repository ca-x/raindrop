use axum::{
    Json, Router,
    body::{Body, Bytes},
    extract::{DefaultBodyLimit, FromRequest, FromRequestParts, Path, Query, Request, State},
    http::{
        HeaderMap, HeaderValue, StatusCode,
        header::{CACHE_CONTROL, CONTENT_TYPE, ETAG, PRAGMA, VARY, X_CONTENT_TYPE_OPTIONS},
        request::Parts,
    },
    response::{IntoResponse, Response},
    routing::{delete, get},
};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use tokio::sync::{OwnedSemaphorePermit, TryAcquireError};

use crate::{
    app::AppState,
    auth::{CsrfGuard, CurrentUser},
    preferences::{
        MAX_USER_FONT_BYTES, MAX_USER_FONTS, UserFont, UserFontError, UserFontRepository,
    },
};

use super::{super::ApiError, map_limiter_rejection};

pub(super) fn command_router() -> Router<AppState> {
    Router::new()
        .route(
            "/api/v2/preferences/fonts",
            get(list_fonts).post(upload_font),
        )
        .route("/api/v2/preferences/fonts/{font_id}", delete(delete_font))
        .layer(DefaultBodyLimit::max(MAX_USER_FONT_BYTES))
}

pub(super) fn file_router() -> Router<AppState> {
    Router::new()
        .route("/api/v2/preferences/fonts/{font_id}/file", get(font_file))
        .layer(axum::middleware::map_response(font_file_cache_headers))
}

struct FontUploadAdmission {
    user: crate::auth::User,
    _permit: OwnedSemaphorePermit,
}

impl FromRequestParts<AppState> for FontUploadAdmission {
    type Rejection = Response;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let CurrentUser(user) = CurrentUser::from_request_parts(parts, state)
            .await
            .map_err(IntoResponse::into_response)?;
        CsrfGuard::from_request_parts(parts, state)
            .await
            .map_err(IntoResponse::into_response)?;
        state
            .preferences_mutation_limiter
            .check(&user.id)
            .map_err(|error| map_limiter_rejection(error).into_response())?;
        validate_content_type(&parts.headers).map_err(IntoResponse::into_response)?;
        let permit = state
            .font_upload_semaphore
            .clone()
            .try_acquire_owned()
            .map_err(|error| match error {
                TryAcquireError::NoPermits => ApiError::rate_limited().into_response(),
                TryAcquireError::Closed => ApiError::internal().into_response(),
            })?;
        Ok(Self {
            user,
            _permit: permit,
        })
    }
}

struct FontBytes(Bytes);

impl FromRequest<AppState> for FontBytes {
    type Rejection = ApiError;

    async fn from_request(request: Request, state: &AppState) -> Result<Self, Self::Rejection> {
        Bytes::from_request(request, state)
            .await
            .map(Self)
            .map_err(|error| {
                if error.status() == StatusCode::PAYLOAD_TOO_LARGE {
                    font_too_large_error()
                } else {
                    ApiError::validation().with_field("font", "Font upload body is invalid")
                }
            })
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
            .map_err(|_| ApiError::validation().with_field("name", "Font name is invalid"))
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct UploadFontParams {
    name: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct UserFontResponse {
    font_id: String,
    display_name: String,
    byte_size: i32,
    file_url: String,
}

impl From<UserFont> for UserFontResponse {
    fn from(font: UserFont) -> Self {
        let font_id = font.id;
        Self {
            file_url: format!("/api/v2/preferences/fonts/{font_id}/file"),
            font_id,
            display_name: font.display_name,
            byte_size: font.byte_size,
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct UserFontListResponse {
    items: Vec<UserFontResponse>,
    maximum_count: u64,
    maximum_bytes: usize,
}

async fn list_fonts(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
) -> Result<Json<UserFontListResponse>, ApiError> {
    let items = repository(&state)?
        .list(&user.id)
        .await
        .map_err(map_font_error)?
        .into_iter()
        .map(Into::into)
        .collect();
    Ok(Json(UserFontListResponse {
        items,
        maximum_count: MAX_USER_FONTS,
        maximum_bytes: MAX_USER_FONT_BYTES,
    }))
}

async fn upload_font(
    State(state): State<AppState>,
    FontUploadAdmission { user, _permit }: FontUploadAdmission,
    ApiQuery(params): ApiQuery<UploadFontParams>,
    FontBytes(body): FontBytes,
) -> Result<impl IntoResponse, ApiError> {
    let font = repository(&state)?
        .create(&user.id, &params.name, &body)
        .await
        .map_err(map_font_error)?;
    Ok((StatusCode::CREATED, Json(UserFontResponse::from(font))))
}

async fn delete_font(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    _csrf: CsrfGuard,
    Path(font_id): Path<String>,
) -> Result<StatusCode, ApiError> {
    state
        .preferences_mutation_limiter
        .check(&user.id)
        .map_err(map_limiter_rejection)?;
    let deleted = repository(&state)?
        .delete(&user.id, &font_id)
        .await
        .map_err(map_font_error)?;
    if deleted {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ApiError::not_found())
    }
}

async fn font_file(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(font_id): Path<String>,
) -> Result<Response, ApiError> {
    let file = repository(&state)?
        .file(&user.id, &font_id)
        .await
        .map_err(map_font_error)?
        .ok_or_else(ApiError::not_found)?;
    let etag = HeaderValue::from_str(&format!("\"{}\"", file.content_hash))
        .map_err(|_| ApiError::internal())?;
    let mut response = Response::new(Body::from(file.bytes));
    response
        .headers_mut()
        .insert(CONTENT_TYPE, HeaderValue::from_static("font/woff2"));
    response.headers_mut().insert(
        CACHE_CONTROL,
        HeaderValue::from_static("private, max-age=31536000, immutable"),
    );
    response
        .headers_mut()
        .insert(X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
    response
        .headers_mut()
        .insert(VARY, HeaderValue::from_static("Cookie"));
    response.headers_mut().insert(ETAG, etag);
    Ok(response)
}

fn validate_content_type(headers: &HeaderMap) -> Result<(), ApiError> {
    let content_type = headers
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(';').next())
        .map(str::trim);
    if matches!(
        content_type,
        Some("font/woff2" | "application/font-woff2" | "application/octet-stream")
    ) {
        Ok(())
    } else {
        Err(ApiError::validation().with_field("font", "A WOFF2 font file is required"))
    }
}

fn repository(state: &AppState) -> Result<UserFontRepository, ApiError> {
    state
        .setup
        .database()
        .map(UserFontRepository::new)
        .map_err(|_| ApiError::internal())
}

fn map_font_error(error: UserFontError) -> ApiError {
    match error {
        UserFontError::InvalidDisplayName => {
            ApiError::validation().with_field("name", "Font name must be 1 to 80 characters")
        }
        UserFontError::InvalidSize => font_too_large_error(),
        UserFontError::InvalidFormat => {
            ApiError::validation().with_field("font", "Font file must be valid WOFF2")
        }
        UserFontError::Duplicate => ApiError::new(
            StatusCode::CONFLICT,
            "FONT_ALREADY_EXISTS",
            "The font is already uploaded",
        ),
        UserFontError::QuotaExceeded => ApiError::new(
            StatusCode::CONFLICT,
            "FONT_QUOTA_EXCEEDED",
            "The custom font limit has been reached",
        ),
        UserFontError::InvalidFontId => ApiError::not_found(),
        UserFontError::Database(_)
        | UserFontError::InvalidUserId
        | UserFontError::UserUnavailable
        | UserFontError::CorruptData => ApiError::internal(),
    }
}

fn font_too_large_error() -> ApiError {
    ApiError::new(
        StatusCode::PAYLOAD_TOO_LARGE,
        "FONT_TOO_LARGE",
        "Font file exceeds the upload limit",
    )
    .with_field("font", "Font file must not exceed 5 MiB")
}

async fn font_file_cache_headers(mut response: Response) -> Response {
    response
        .headers_mut()
        .insert(VARY, HeaderValue::from_static("Cookie"));
    if !response.status().is_success() || !response.headers().contains_key(CACHE_CONTROL) {
        response
            .headers_mut()
            .insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
        response
            .headers_mut()
            .insert(PRAGMA, HeaderValue::from_static("no-cache"));
    }
    response
}
