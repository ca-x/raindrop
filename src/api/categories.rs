use axum::{
    Json, Router,
    extract::{FromRequestParts, Path, State},
    http::{HeaderValue, StatusCode, header::LOCATION, request::Parts},
    middleware,
    response::{IntoResponse, Response},
    routing::{get, patch},
};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use time::{OffsetDateTime, UtcOffset, macros::format_description};
use uuid::Uuid;

use crate::{
    app::AppState,
    auth::{CsrfGuard, CurrentUser},
    organization::{
        CategoryDto, CategoryError, CategoryRepository, CreateCategory, UpdateCategory,
    },
};

use super::{ApiError, ApiJson, RateLimitRejection, routes::sensitive_cache_headers};

const PUBLIC_TIME_FORMAT: &[time::format_description::FormatItem<'static>] =
    format_description!("[year]-[month]-[day]T[hour]:[minute]:[second].[subsecond digits:6]Z");

pub(super) fn router() -> Router<AppState> {
    let categories = Router::new()
        .route("/", get(list_categories).post(create_category))
        .route(
            "/{category_id}",
            patch(update_category).delete(delete_category),
        )
        .fallback(category_not_found)
        .method_not_allowed_fallback(category_method_not_allowed);
    Router::new()
        .route(
            "/api/v1/categories/",
            axum::routing::any(category_not_found),
        )
        .nest("/api/v1/categories", categories)
        .layer(middleware::map_response(sensitive_cache_headers))
}

async fn category_not_found() -> ApiError {
    ApiError::not_found()
}

async fn category_method_not_allowed() -> ApiError {
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

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CreateCategoryRequest {
    title: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct UpdateCategoryRequest {
    #[serde(default, deserialize_with = "deserialize_present")]
    title: Option<String>,
    #[serde(default, deserialize_with = "deserialize_present")]
    position: Option<i64>,
}

fn deserialize_present<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    T::deserialize(deserializer).map(Some)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CategoryListResponse {
    items: Vec<CategoryResponse>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CategoryResponse {
    category_id: String,
    title: String,
    position: i64,
}

impl From<CategoryDto> for CategoryResponse {
    fn from(category: CategoryDto) -> Self {
        Self {
            category_id: category.category_id,
            title: category.title,
            position: category.position,
        }
    }
}

async fn list_categories(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
) -> Result<Json<CategoryListResponse>, ApiError> {
    let items = repository(&state)?
        .list(&user.id)
        .await
        .map_err(map_category_error)?
        .into_iter()
        .map(Into::into)
        .collect();
    Ok(Json(CategoryListResponse { items }))
}

async fn create_category(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    _csrf: CsrfGuard,
    ApiJson(request): ApiJson<CreateCategoryRequest>,
) -> Result<Response, ApiError> {
    state
        .organization_mutation_limiter
        .check(&user.id)
        .map_err(map_limiter_rejection)?;
    let category = repository(&state)?
        .create(
            &user.id,
            CreateCategory {
                title: request.title,
            },
        )
        .await
        .map_err(map_category_error)?;
    let location = HeaderValue::from_str(&format!("/api/v1/categories/{}", category.category_id))
        .map_err(|_| ApiError::internal())?;
    Ok((
        StatusCode::CREATED,
        [(LOCATION, location)],
        Json(CategoryResponse::from(category)),
    )
        .into_response())
}

async fn update_category(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    _csrf: CsrfGuard,
    ApiPath(category_id): ApiPath<String>,
    ApiJson(request): ApiJson<UpdateCategoryRequest>,
) -> Result<Json<CategoryResponse>, ApiError> {
    validate_canonical_uuid(&category_id)?;
    state
        .organization_mutation_limiter
        .check(&user.id)
        .map_err(map_limiter_rejection)?;
    let category = repository(&state)?
        .update(
            &user.id,
            &category_id,
            UpdateCategory {
                title: request.title,
                position: request.position,
            },
        )
        .await
        .map_err(map_category_error)?;
    Ok(Json(category.into()))
}

async fn delete_category(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    _csrf: CsrfGuard,
    ApiPath(category_id): ApiPath<String>,
) -> Result<StatusCode, ApiError> {
    validate_canonical_uuid(&category_id)?;
    state
        .organization_mutation_limiter
        .check(&user.id)
        .map_err(map_limiter_rejection)?;
    repository(&state)?
        .delete(&user.id, &category_id)
        .await
        .map_err(map_category_error)?;
    Ok(StatusCode::NO_CONTENT)
}

fn repository(state: &AppState) -> Result<CategoryRepository, ApiError> {
    state
        .setup
        .database()
        .map(CategoryRepository::new)
        .map_err(|_| ApiError::internal())
}

fn validate_canonical_uuid(value: &str) -> Result<(), ApiError> {
    let parsed = Uuid::parse_str(value)
        .map_err(|_| ApiError::validation().with_field("categoryId", "Identifier is invalid"))?;
    if parsed.to_string() == value {
        Ok(())
    } else {
        Err(ApiError::validation().with_field("categoryId", "Identifier is invalid"))
    }
}

fn map_category_error(error: CategoryError) -> ApiError {
    match error {
        CategoryError::InvalidTitle => {
            ApiError::validation().with_field("title", "Category title is invalid")
        }
        CategoryError::InvalidPosition => {
            ApiError::validation().with_field("position", "Position must be non-negative")
        }
        CategoryError::InvalidCategoryId => {
            ApiError::validation().with_field("categoryId", "Identifier is invalid")
        }
        CategoryError::InvalidPatch => ApiError::validation(),
        CategoryError::NotFound => ApiError::not_found(),
        CategoryError::Conflict => ApiError::new(
            StatusCode::CONFLICT,
            "CONFLICT",
            "The request conflicts with state",
        ),
        CategoryError::Limit => ApiError::new(
            StatusCode::CONFLICT,
            "CATEGORY_LIMIT_REACHED",
            "Category limit reached",
        ),
        CategoryError::InvalidUserId
        | CategoryError::UserUnavailable
        | CategoryError::Database(_)
        | CategoryError::CorruptData => ApiError::internal(),
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
