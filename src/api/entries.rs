use axum::{
    Json, Router,
    extract::{FromRequestParts, Path, State},
    http::request::Parts,
    middleware,
    routing::get,
};
use serde::{Deserialize, Serialize, de::DeserializeOwned};

use crate::{
    app::AppState,
    auth::CurrentUser,
    feeds::{
        EnclosureDto, EntryDetailDto, EntryListItemDto, EntryListState, FeedRepository,
        InertImageDto, ListEntriesQuery, RepositoryError,
    },
};

use super::{ApiError, routes::sensitive_cache_headers};

pub(super) fn router() -> Router<AppState> {
    let reader = Router::new()
        .route("/", get(list_entries))
        .route("/{entry_id}", get(get_entry))
        .fallback(reader_not_found)
        .method_not_allowed_fallback(reader_method_not_allowed);
    Router::new()
        .route("/api/v1/entries/", axum::routing::any(reader_not_found))
        .nest("/api/v1/entries", reader)
        .layer(middleware::map_response(sensitive_cache_headers))
}

async fn reader_not_found() -> ApiError {
    ApiError::not_found()
}

async fn reader_method_not_allowed() -> ApiError {
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

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ListEntriesParams {
    cursor: Option<String>,
    limit: Option<u16>,
    feed_id: Option<String>,
    state: Option<EntryStateParam>,
}

impl ListEntriesParams {
    fn into_query(self) -> ListEntriesQuery {
        let defaults = ListEntriesQuery::default();
        ListEntriesQuery {
            state: self.state.map_or(defaults.state, EntryListState::from),
            feed_id: self.feed_id,
            limit: self.limit.unwrap_or(defaults.limit),
            cursor: self.cursor,
        }
    }
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
enum EntryStateParam {
    All,
    Unread,
    Starred,
}

impl From<EntryStateParam> for EntryListState {
    fn from(state: EntryStateParam) -> Self {
        match state {
            EntryStateParam::All => Self::All,
            EntryStateParam::Unread => Self::Unread,
            EntryStateParam::Starred => Self::Starred,
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct EntryPageResponse {
    items: Vec<EntryListItemResponse>,
    next_cursor: Option<String>,
    snapshot_generation: i64,
}

impl From<crate::feeds::EntryPage> for EntryPageResponse {
    fn from(page: crate::feeds::EntryPage) -> Self {
        Self {
            items: page.items.into_iter().map(Into::into).collect(),
            next_cursor: page.next_cursor,
            snapshot_generation: page.snapshot_generation,
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct EntryListItemResponse {
    entry_id: String,
    feed_id: String,
    feed_title: String,
    site_url: Option<String>,
    title: Option<String>,
    author: Option<String>,
    summary: Option<String>,
    canonical_url: Option<String>,
    published_at_us: Option<i64>,
    sort_at_us: i64,
    is_read: bool,
    is_starred: bool,
}

impl From<EntryListItemDto> for EntryListItemResponse {
    fn from(item: EntryListItemDto) -> Self {
        Self {
            entry_id: item.entry_id,
            feed_id: item.feed_id,
            feed_title: item.feed_title,
            site_url: item.site_url,
            title: item.title,
            author: item.author,
            summary: item.summary,
            canonical_url: item.canonical_url,
            published_at_us: item.published_at_us,
            sort_at_us: item.sort_at_us,
            is_read: item.is_read,
            is_starred: item.is_starred,
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct EntryDetailResponse {
    entry_id: String,
    feed_id: String,
    feed_title: String,
    site_url: Option<String>,
    title: Option<String>,
    author: Option<String>,
    summary: Option<String>,
    canonical_url: Option<String>,
    published_at_us: Option<i64>,
    sort_at_us: i64,
    is_read: bool,
    is_starred: bool,
    content_html: String,
    inert_images: Vec<InertImageResponse>,
    enclosures: Vec<EnclosureResponse>,
}

impl From<EntryDetailDto> for EntryDetailResponse {
    fn from(detail: EntryDetailDto) -> Self {
        Self {
            entry_id: detail.entry_id,
            feed_id: detail.feed_id,
            feed_title: detail.feed_title,
            site_url: detail.site_url,
            title: detail.title,
            author: detail.author,
            summary: detail.summary,
            canonical_url: detail.canonical_url,
            published_at_us: detail.published_at_us,
            sort_at_us: detail.sort_at_us,
            is_read: detail.is_read,
            is_starred: detail.is_starred,
            content_html: detail.content_html,
            inert_images: detail.inert_images.into_iter().map(Into::into).collect(),
            enclosures: detail
                .enclosures
                .unwrap_or_default()
                .into_iter()
                .map(Into::into)
                .collect(),
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct InertImageResponse {
    image_index: u32,
    source_url: String,
    alt: Option<String>,
    width: Option<u32>,
    height: Option<u32>,
}

impl From<InertImageDto> for InertImageResponse {
    fn from(image: InertImageDto) -> Self {
        Self {
            image_index: image.image_index,
            source_url: image.source_url,
            alt: image.alt,
            width: image.width,
            height: image.height,
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct EnclosureResponse {
    url: String,
    media_type: Option<String>,
    length: Option<String>,
    title: Option<String>,
    duration: Option<String>,
}

impl From<EnclosureDto> for EnclosureResponse {
    fn from(enclosure: EnclosureDto) -> Self {
        Self {
            url: enclosure.url,
            media_type: enclosure.media_type,
            length: enclosure.length,
            title: enclosure.title,
            duration: enclosure.duration,
        }
    }
}

fn repository(state: &AppState) -> Result<FeedRepository, ApiError> {
    state
        .setup
        .database()
        .map(FeedRepository::new)
        .map_err(|_| ApiError::internal())
}

async fn list_entries(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    ApiQuery(params): ApiQuery<ListEntriesParams>,
) -> Result<Json<EntryPageResponse>, ApiError> {
    let page = repository(&state)?
        .list_for_user(&user.id, params.into_query())
        .await
        .map_err(map_repository_error)?;
    Ok(Json(page.into()))
}

async fn get_entry(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(entry_id): Path<String>,
) -> Result<Json<EntryDetailResponse>, ApiError> {
    let detail = repository(&state)?
        .get_detail_for_user(&user.id, &entry_id)
        .await
        .map_err(map_repository_error)?
        .ok_or_else(ApiError::not_found)?;
    Ok(Json(detail.into()))
}

fn map_repository_error(error: RepositoryError) -> ApiError {
    match error {
        RepositoryError::InvalidUserId => ApiError::validation(),
        RepositoryError::InvalidFeedId => {
            ApiError::validation().with_field("feedId", "Feed identifier is invalid")
        }
        RepositoryError::InvalidEntryId => {
            ApiError::validation().with_field("entryId", "Entry identifier is invalid")
        }
        RepositoryError::InvalidLimit => {
            ApiError::validation().with_field("limit", "Limit must be between 1 and 100")
        }
        RepositoryError::InvalidCursor => {
            ApiError::validation().with_field("cursor", "Cursor is invalid")
        }
        RepositoryError::Database(_)
        | RepositoryError::CorruptData
        | RepositoryError::Content(_) => ApiError::internal(),
    }
}
