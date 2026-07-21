use axum::{
    Json, Router,
    extract::{FromRequestParts, Path, State},
    http::{HeaderValue, StatusCode, header::LOCATION, request::Parts},
    middleware,
    response::{IntoResponse, Response},
    routing::get,
};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use time::{OffsetDateTime, UtcOffset, macros::format_description};

use crate::{
    app::AppState,
    auth::{CsrfGuard, CurrentUser},
    feeds::{
        FeedCommandService, FeedRepository, FeedServiceError, FeedUrlPolicy,
        ListSubscriptionsQuery, PatchValue, QueueSubscriptionRefresh, RefreshDto,
        RefreshRepositoryError, RefreshStatus, RepositoryError, SubscribeInput,
        SubscriptionListItemDto, SubscriptionPage, SubscriptionPatchError, UpdateSubscription,
    },
};
use url::Url;
use uuid::Uuid;

use super::{
    ApiError, ApiJson, RateLimitRejection,
    media::{fetch_raster_image, media_cache_headers},
    routes::sensitive_cache_headers,
};

mod opml;

const PUBLIC_TIME_FORMAT: &[time::format_description::FormatItem<'static>] =
    format_description!("[year]-[month]-[day]T[hour]:[minute]:[second].[subsecond digits:6]Z");
const MAX_FAVICON_BYTES: usize = 512 * 1024;
const FAVICON_CACHE_CONTROL: &str = "private, no-cache, max-age=0, must-revalidate";

pub(super) fn router() -> Router<AppState> {
    let subscriptions = Router::new()
        .route("/", get(list_subscriptions).post(create_subscription))
        .route(
            "/{subscription_id}",
            get(get_subscription)
                .patch(update_subscription)
                .delete(delete_subscription),
        )
        .route(
            "/{subscription_id}/refresh",
            axum::routing::post(refresh_subscription),
        )
        .fallback(subscription_not_found)
        .method_not_allowed_fallback(subscription_method_not_allowed);
    Router::new()
        .route(
            "/api/v1/subscriptions/",
            axum::routing::any(subscription_not_found),
        )
        .nest("/api/v1/subscriptions", subscriptions)
        .merge(opml::router())
        .layer(middleware::map_response(sensitive_cache_headers))
}

pub(super) fn media_router() -> Router<AppState> {
    Router::new()
        .route(
            "/reader-assets/subscriptions/{subscription_id}/favicon",
            get(subscription_favicon),
        )
        .layer(middleware::map_response(media_cache_headers))
}

async fn subscription_not_found() -> ApiError {
    ApiError::not_found()
}

async fn subscription_method_not_allowed() -> ApiError {
    ApiError::method_not_allowed()
}

struct ApiQuery<T>(T);

impl<T, S> FromRequestParts<S> for ApiQuery<T>
where
    T: DeserializeOwned,
    S: Send + Sync,
{
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        axum::extract::Query::<T>::from_request_parts(parts, state)
            .await
            .map(|axum::extract::Query(value)| Self(value))
            .map_err(|_| ApiError::validation())
    }
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
struct ListSubscriptionsParams {
    cursor: Option<String>,
    limit: Option<u16>,
}

impl ListSubscriptionsParams {
    fn into_query(self) -> ListSubscriptionsQuery {
        let defaults = ListSubscriptionsQuery::default();
        ListSubscriptionsQuery {
            cursor: self.cursor,
            limit: self.limit.unwrap_or(defaults.limit),
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CreateSubscriptionRequest {
    url: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RefreshSubscriptionRequest {
    request_id: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct UpdateSubscriptionRequest {
    #[serde(default)]
    category_id: PatchValue<String>,
    #[serde(default)]
    title_override: PatchValue<String>,
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
struct SubscriptionPageResponse {
    items: Vec<SubscriptionResponse>,
    next_cursor: Option<String>,
}

impl TryFrom<SubscriptionPage> for SubscriptionPageResponse {
    type Error = ApiError;

    fn try_from(page: SubscriptionPage) -> Result<Self, Self::Error> {
        Ok(Self {
            items: page
                .items
                .into_iter()
                .map(SubscriptionResponse::try_from)
                .collect::<Result<_, _>>()?,
            next_cursor: page.next_cursor,
        })
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CreateSubscriptionResponse {
    created: bool,
    subscription: SubscriptionResponse,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SubscriptionResponse {
    subscription_id: String,
    feed_id: String,
    category_id: Option<String>,
    title_override: Option<String>,
    position: i64,
    title: String,
    feed_url: String,
    site_url: Option<String>,
    unread_count: i64,
    refresh: Option<RefreshResponse>,
}

impl TryFrom<SubscriptionListItemDto> for SubscriptionResponse {
    type Error = ApiError;

    fn try_from(item: SubscriptionListItemDto) -> Result<Self, Self::Error> {
        Ok(Self {
            subscription_id: item.subscription_id,
            feed_id: item.feed_id,
            category_id: item.category_id,
            title_override: item.title_override,
            position: item.position,
            title: item.title,
            feed_url: item.feed_url,
            site_url: item.site_url,
            unread_count: item.unread_count,
            refresh: item.refresh.map(RefreshResponse::try_from).transpose()?,
        })
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RefreshResponse {
    operation_id: String,
    state: &'static str,
    pending_state: Option<&'static str>,
    new_count: i32,
    updated_count: i32,
    dropped_count: i32,
    entry_issues: Vec<RefreshEntryIssueResponse>,
    generation: Option<i64>,
    error_code: Option<&'static str>,
    retry_at: Option<String>,
    last_success_at: Option<String>,
    queued_at: String,
    started_at: Option<String>,
    completed_at: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RefreshEntryIssueResponse {
    code: &'static str,
    count: i32,
}

impl TryFrom<RefreshDto> for RefreshResponse {
    type Error = ApiError;

    fn try_from(refresh: RefreshDto) -> Result<Self, Self::Error> {
        let pending_state = public_pending_state(&refresh);
        let entry_issues = public_entry_issues(&refresh);
        Ok(Self {
            operation_id: refresh.run_id,
            state: public_refresh_state(refresh.status, refresh.retry_at),
            pending_state,
            new_count: refresh.new_count,
            updated_count: refresh.updated_count,
            dropped_count: refresh.dropped_count,
            entry_issues,
            generation: refresh.generation,
            error_code: refresh.error_code.as_deref().map(public_refresh_error_code),
            retry_at: refresh.retry_at.map(format_public_time).transpose()?,
            last_success_at: refresh
                .last_success_at
                .map(format_public_time)
                .transpose()?,
            queued_at: format_public_time(refresh.queued_at)?,
            started_at: refresh.started_at.map(format_public_time).transpose()?,
            completed_at: refresh.completed_at.map(format_public_time).transpose()?,
        })
    }
}

fn public_pending_state(refresh: &RefreshDto) -> Option<&'static str> {
    match (refresh.status, refresh.started_at, refresh.completed_at) {
        (RefreshStatus::Queued, None, None) => Some("QUEUED"),
        (RefreshStatus::Running, Some(_), None) => Some("RUNNING"),
        _ => None,
    }
}

fn public_entry_issues(refresh: &RefreshDto) -> Vec<RefreshEntryIssueResponse> {
    if refresh.status == RefreshStatus::Partial && refresh.dropped_count > 0 {
        vec![RefreshEntryIssueResponse {
            code: "DUPLICATE_ENTRY",
            count: refresh.dropped_count,
        }]
    } else {
        Vec::new()
    }
}

fn public_refresh_state(status: RefreshStatus, retry_at: Option<OffsetDateTime>) -> &'static str {
    match status {
        RefreshStatus::Queued | RefreshStatus::Running => "PENDING",
        RefreshStatus::Success | RefreshStatus::NotModified => "READY",
        RefreshStatus::Partial => "DEGRADED",
        RefreshStatus::Error if retry_at.is_some() => "BACKING_OFF",
        RefreshStatus::Error | RefreshStatus::LeaseLost | RefreshStatus::Cancelled => "ERROR",
    }
}

fn public_refresh_error_code(error_code: &str) -> &'static str {
    if error_code == "UPSTREAM_RATE_LIMITED" {
        "UPSTREAM_RATE_LIMITED"
    } else {
        "REFRESH_FAILED"
    }
}

fn format_public_time(value: OffsetDateTime) -> Result<String, ApiError> {
    value
        .to_offset(UtcOffset::UTC)
        .format(PUBLIC_TIME_FORMAT)
        .map_err(|_| ApiError::internal())
}

fn command_service(state: &AppState) -> Result<FeedCommandService, ApiError> {
    state
        .setup
        .database()
        .map(FeedRepository::new)
        .map(FeedCommandService::new)
        .map_err(|_| ApiError::internal())
}

async fn subscription_favicon(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    ApiPath(subscription_id): ApiPath<String>,
) -> Result<Response, ApiError> {
    let subscription = command_service(&state)?
        .get_subscription(&user.id, &subscription_id)
        .await
        .map_err(map_favicon_subscription_error)?
        .ok_or_else(ApiError::not_found)?;
    let site_url = subscription.site_url.ok_or_else(ApiError::not_found)?;
    let favicon_url = favicon_request_url(&site_url).ok_or_else(ApiError::not_found)?;
    fetch_raster_image(
        &state,
        &favicon_url,
        MAX_FAVICON_BYTES,
        FAVICON_CACHE_CONTROL,
    )
    .await
}

fn map_favicon_subscription_error(error: FeedServiceError) -> ApiError {
    match error {
        FeedServiceError::InvalidSubscriptionId | FeedServiceError::Unauthorized => {
            ApiError::not_found()
        }
        other => map_feed_service_error(other),
    }
}

fn favicon_request_url(site_url: &str) -> Option<String> {
    let mut url = Url::parse(site_url).ok()?;
    if url.scheme() != "https" || !url.username().is_empty() || url.password().is_some() {
        return None;
    }
    url.set_path("/favicon.ico");
    url.set_query(None);
    url.set_fragment(None);
    FeedUrlPolicy::new(false)
        .normalize(url.as_str())
        .ok()
        .map(|normalized| normalized.complete().to_owned())
}

async fn list_subscriptions(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    ApiQuery(params): ApiQuery<ListSubscriptionsParams>,
) -> Result<Json<SubscriptionPageResponse>, ApiError> {
    let page = command_service(&state)?
        .list_subscriptions(&user.id, params.into_query())
        .await
        .map_err(map_feed_service_error)?;
    Ok(Json(page.try_into()?))
}

async fn create_subscription(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    _csrf: CsrfGuard,
    ApiJson(request): ApiJson<CreateSubscriptionRequest>,
) -> Result<Response, ApiError> {
    FeedUrlPolicy::new(false)
        .normalize(&request.url)
        .map_err(|_| ApiError::validation().with_field("url", "Feed URL is invalid"))?;
    state
        .subscription_mutation_limiter
        .check(&user.id)
        .map_err(map_limiter_rejection)?;
    let service = command_service(&state)?;
    let outcome = state
        .commit_and_notify_feed_runtime(
            service.subscribe(&user.id, SubscribeInput { url: request.url }),
        )
        .await
        .map_err(map_feed_service_error)?;
    let status = if outcome.created {
        StatusCode::CREATED
    } else {
        StatusCode::OK
    };
    let location = HeaderValue::from_str(&format!(
        "/api/v1/subscriptions/{}",
        outcome.subscription.subscription_id
    ))
    .map_err(|_| ApiError::internal())?;
    Ok((
        status,
        [(LOCATION, location)],
        Json(CreateSubscriptionResponse {
            created: outcome.created,
            subscription: outcome.subscription.try_into()?,
        }),
    )
        .into_response())
}

async fn get_subscription(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    ApiPath(subscription_id): ApiPath<String>,
) -> Result<Json<SubscriptionResponse>, ApiError> {
    let subscription = command_service(&state)?
        .get_subscription(&user.id, &subscription_id)
        .await
        .map_err(map_feed_service_error)?
        .ok_or_else(ApiError::not_found)?;
    Ok(Json(subscription.try_into()?))
}

async fn update_subscription(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    _csrf: CsrfGuard,
    ApiPath(subscription_id): ApiPath<String>,
    ApiJson(request): ApiJson<UpdateSubscriptionRequest>,
) -> Result<Json<SubscriptionResponse>, ApiError> {
    validate_canonical_uuid(&subscription_id, "subscriptionId")?;
    state
        .subscription_mutation_limiter
        .check(&user.id)
        .map_err(map_limiter_rejection)?;
    let subscription = command_service(&state)?
        .update_subscription(
            &user.id,
            &subscription_id,
            UpdateSubscription {
                category_id: request.category_id,
                title_override: request.title_override,
                position: request.position,
            },
        )
        .await
        .map_err(map_feed_service_error)?;
    Ok(Json(subscription.try_into()?))
}

async fn refresh_subscription(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    _csrf: CsrfGuard,
    ApiPath(subscription_id): ApiPath<String>,
    ApiJson(request): ApiJson<RefreshSubscriptionRequest>,
) -> Result<Response, ApiError> {
    validate_canonical_uuid(&subscription_id, "subscriptionId")?;
    validate_canonical_uuid(&request.request_id, "requestId")?;
    state
        .subscription_mutation_limiter
        .check(&user.id)
        .map_err(map_limiter_rejection)?;
    let service = command_service(&state)?;
    let refresh = state
        .commit_and_notify_feed_runtime(service.queue_subscription_refresh(
            &user.id,
            &subscription_id,
            QueueSubscriptionRefresh {
                request_id: request.request_id,
            },
        ))
        .await
        .map_err(map_feed_service_error)?;
    let status = if matches!(
        refresh.status,
        RefreshStatus::Queued | RefreshStatus::Running
    ) {
        StatusCode::ACCEPTED
    } else {
        StatusCode::OK
    };
    Ok((status, Json(RefreshResponse::try_from(refresh)?)).into_response())
}

async fn delete_subscription(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    _csrf: CsrfGuard,
    ApiPath(subscription_id): ApiPath<String>,
) -> Result<StatusCode, ApiError> {
    validate_canonical_uuid(&subscription_id, "subscriptionId")?;
    state
        .subscription_mutation_limiter
        .check(&user.id)
        .map_err(map_limiter_rejection)?;
    let service = command_service(&state)?;
    state
        .commit_and_notify_feed_runtime(service.unsubscribe(&user.id, &subscription_id))
        .await
        .map_err(map_feed_service_error)?;
    Ok(StatusCode::NO_CONTENT)
}

fn validate_canonical_uuid(value: &str, field: &'static str) -> Result<(), ApiError> {
    let parsed = Uuid::parse_str(value)
        .map_err(|_| ApiError::validation().with_field(field, "Identifier is invalid"))?;
    if parsed.to_string() == value {
        Ok(())
    } else {
        Err(ApiError::validation().with_field(field, "Identifier is invalid"))
    }
}

fn map_feed_service_error(error: FeedServiceError) -> ApiError {
    match error {
        FeedServiceError::InvalidUserId => ApiError::internal(),
        FeedServiceError::InvalidSubscriptionId | FeedServiceError::Url(_) => {
            ApiError::validation()
        }
        FeedServiceError::SubscriptionPatch(error) => map_subscription_patch_error(error),
        FeedServiceError::Unauthorized => ApiError::not_found(),
        FeedServiceError::RefreshRepository(error) => map_refresh_repository_error(error),
        FeedServiceError::EntryRepository(error) => map_repository_error(error),
        FeedServiceError::RunMismatch => conflict_error(),
        FeedServiceError::CorruptFeed
        | FeedServiceError::RuntimeSupervision
        | FeedServiceError::ExecutorInitialization(_)
        | FeedServiceError::Schedule(_) => ApiError::internal(),
    }
}

fn map_subscription_patch_error(error: SubscriptionPatchError) -> ApiError {
    match error {
        SubscriptionPatchError::Empty => ApiError::validation(),
        SubscriptionPatchError::InvalidCategoryId => {
            ApiError::validation().with_field("categoryId", "Identifier is invalid")
        }
        SubscriptionPatchError::InvalidTitleOverride => {
            ApiError::validation().with_field("titleOverride", "Title override is invalid")
        }
        SubscriptionPatchError::InvalidPosition => {
            ApiError::validation().with_field("position", "Position must be non-negative")
        }
    }
}

fn map_repository_error(error: RepositoryError) -> ApiError {
    match error {
        RepositoryError::InvalidLimit => {
            ApiError::validation().with_field("limit", "Limit must be between 1 and 100")
        }
        RepositoryError::InvalidCursor => {
            ApiError::validation().with_field("cursor", "Cursor is invalid")
        }
        RepositoryError::InvalidUserId
        | RepositoryError::InvalidFeedId
        | RepositoryError::InvalidCategoryId
        | RepositoryError::InvalidSourceFilter
        | RepositoryError::InvalidSearch
        | RepositoryError::InvalidSnapshotGeneration
        | RepositoryError::InvalidEntryId
        | RepositoryError::InvalidStatePatch
        | RepositoryError::Database(_)
        | RepositoryError::CorruptData
        | RepositoryError::Content(_) => ApiError::internal(),
    }
}

fn map_refresh_repository_error(error: RefreshRepositoryError) -> ApiError {
    match error {
        RefreshRepositoryError::InvalidRequest => ApiError::validation(),
        RefreshRepositoryError::RefreshCooldown {
            retry_at,
            retry_after_seconds,
        } => rate_limited_at(retry_at, retry_after_seconds),
        RefreshRepositoryError::SubscriptionLimit | RefreshRepositoryError::ActiveRefreshLimit => {
            rate_limited_without_retry()
        }
        RefreshRepositoryError::IdempotencyConflict
        | RefreshRepositoryError::FeedDisabled
        | RefreshRepositoryError::IdentityHashCollision => conflict_error(),
        RefreshRepositoryError::RefreshInProgress { operation_id } => ApiError::new(
            StatusCode::CONFLICT,
            "REFRESH_IN_PROGRESS",
            "A refresh is already in progress",
        )
        .with_field("operationId", operation_id),
        RefreshRepositoryError::Database(_)
        | RefreshRepositoryError::LifecycleEventConflict
        | RefreshRepositoryError::LifecyclePayloadTooLarge
        | RefreshRepositoryError::InvalidLifecyclePayload
        | RefreshRepositoryError::TokenExhausted
        | RefreshRepositoryError::CorruptData
        | RefreshRepositoryError::LeaseLost
        | RefreshRepositoryError::InvalidTransition
        | RefreshRepositoryError::RunNotFound
        | RefreshRepositoryError::Content(_)
        | RefreshRepositoryError::InvalidContent
        | RefreshRepositoryError::CorruptValidator
        | RefreshRepositoryError::InvalidTime
        | RefreshRepositoryError::GenerationExhausted
        | RefreshRepositoryError::SequenceExhausted
        | RefreshRepositoryError::CountOverflow => ApiError::internal(),
    }
}

fn conflict_error() -> ApiError {
    ApiError::new(
        StatusCode::CONFLICT,
        "CONFLICT",
        "The request conflicts with state",
    )
}

fn rate_limited_without_retry() -> ApiError {
    ApiError::new(
        StatusCode::TOO_MANY_REQUESTS,
        "RATE_LIMITED",
        "Too many requests",
    )
}

fn map_limiter_rejection(rejection: RateLimitRejection) -> ApiError {
    match format_public_time(rejection.retry_at) {
        Ok(retry_at) => ApiError::rate_limited_with_retry(retry_at, rejection.retry_after_seconds),
        Err(error) => error,
    }
}

fn rate_limited_at(retry_at: OffsetDateTime, retry_after_seconds: u64) -> ApiError {
    match format_public_time(retry_at) {
        Ok(retry_at) => ApiError::rate_limited_with_retry(retry_at, retry_after_seconds),
        Err(error) => error,
    }
}

#[cfg(test)]
mod favicon_tests {
    use super::*;

    #[test]
    fn favicon_request_uses_the_https_site_origin() {
        let url = favicon_request_url("https://example.com/articles/feed?q=1#section")
            .expect("HTTPS site URL should produce a favicon request");
        assert_eq!(url, "https://example.com/favicon.ico");

        assert!(favicon_request_url("http://example.com/").is_none());
        assert!(favicon_request_url("https://user:secret@example.com/").is_none());
        assert!(favicon_request_url("not a URL").is_none());
    }
}
