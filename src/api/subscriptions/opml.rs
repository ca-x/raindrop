use axum::{
    Router,
    body::Bytes,
    extract::{DefaultBodyLimit, FromRequestParts, State},
    http::{
        HeaderValue, StatusCode,
        header::{CONTENT_DISPOSITION, CONTENT_TYPE},
        request::Parts,
    },
    response::{IntoResponse, Response},
    routing::{get, post},
};
use serde::{Deserialize, Serialize, de::DeserializeOwned};

use crate::{
    app::AppState,
    auth::{CsrfGuard, CurrentUser},
    feeds::{
        FeedRepository, MAX_OPML_BYTES, OpmlDocument, OpmlError, OpmlImportResult, OpmlPreview,
    },
};

use super::super::ApiError;

pub(super) fn router() -> Router<AppState> {
    Router::new()
        .route("/api/v1/imports/opml", post(import_opml))
        .route("/api/v1/exports/opml", get(export_opml))
        .layer(DefaultBodyLimit::max(MAX_OPML_BYTES))
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
            .map_err(|_| ApiError::validation().with_field("mode", "Import mode is invalid"))
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ImportParams {
    mode: Option<String>,
}

#[derive(Clone, Copy)]
enum ImportMode {
    Preview,
    Commit,
}

impl ImportMode {
    fn parse(value: Option<&str>) -> Result<Self, ApiError> {
        match value.unwrap_or("preview") {
            "preview" => Ok(Self::Preview),
            "commit" => Ok(Self::Commit),
            _ => Err(ApiError::validation().with_field("mode", "Import mode is invalid")),
        }
    }

    const fn as_str(self) -> &'static str {
        match self {
            Self::Preview => "PREVIEW",
            Self::Commit => "COMMIT",
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct OpmlImportResponse {
    mode: &'static str,
    outline_count: usize,
    valid_count: usize,
    new_count: usize,
    imported_count: usize,
    duplicate_count: usize,
    invalid_count: usize,
    category_count: usize,
    created_category_count: usize,
}

impl OpmlImportResponse {
    fn preview(preview: OpmlPreview) -> Self {
        Self {
            mode: ImportMode::Preview.as_str(),
            outline_count: preview.outline_count,
            valid_count: preview.valid_count,
            new_count: preview.new_count,
            imported_count: 0,
            duplicate_count: preview.duplicate_count,
            invalid_count: preview.invalid_count,
            category_count: preview.category_count,
            created_category_count: 0,
        }
    }

    fn imported(imported: OpmlImportResult) -> Self {
        Self {
            mode: ImportMode::Commit.as_str(),
            outline_count: imported.outline_count,
            valid_count: imported.valid_count,
            new_count: imported.imported_count,
            imported_count: imported.imported_count,
            duplicate_count: imported.duplicate_count,
            invalid_count: imported.invalid_count,
            category_count: imported.created_category_count,
            created_category_count: imported.created_category_count,
        }
    }
}

async fn import_opml(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    _csrf: CsrfGuard,
    ApiQuery(params): ApiQuery<ImportParams>,
    body: Bytes,
) -> Result<axum::Json<OpmlImportResponse>, ApiError> {
    let mode = ImportMode::parse(params.mode.as_deref())?;
    let document = OpmlDocument::parse(&body).map_err(map_opml_error)?;
    let repository = repository(&state)?;
    let response = match mode {
        ImportMode::Preview => OpmlImportResponse::preview(
            repository
                .preview_opml(&user.id, &document)
                .await
                .map_err(map_opml_error)?,
        ),
        ImportMode::Commit => {
            state
                .subscription_mutation_limiter
                .check(&user.id)
                .map_err(super::map_limiter_rejection)?;
            let imported = state
                .commit_and_notify_feed_runtime(repository.import_opml(&user.id, &document))
                .await
                .map_err(map_opml_error)?;
            OpmlImportResponse::imported(imported)
        }
    };
    Ok(axum::Json(response))
}

async fn export_opml(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
) -> Result<Response, ApiError> {
    let document = repository(&state)?
        .export_opml(&user.id)
        .await
        .map_err(map_opml_error)?;
    Ok((
        [
            (
                CONTENT_TYPE,
                HeaderValue::from_static("application/xml; charset=utf-8"),
            ),
            (
                CONTENT_DISPOSITION,
                HeaderValue::from_static("attachment; filename=\"raindrop.opml\""),
            ),
        ],
        document,
    )
        .into_response())
}

fn repository(state: &AppState) -> Result<FeedRepository, ApiError> {
    state
        .setup
        .database()
        .map(FeedRepository::new)
        .map_err(|_| ApiError::internal())
}

fn map_opml_error(error: OpmlError) -> ApiError {
    match error {
        OpmlError::DocumentTooLarge => ApiError::new(
            StatusCode::PAYLOAD_TOO_LARGE,
            "PAYLOAD_TOO_LARGE",
            "OPML file is too large",
        ),
        OpmlError::OutlineLimit => {
            ApiError::validation().with_field("file", "OPML contains more than 10,000 outlines")
        }
        OpmlError::DepthLimit | OpmlError::EventLimit => {
            ApiError::validation().with_field("file", "OPML structure is too complex")
        }
        OpmlError::ForbiddenXmlConstruct => {
            ApiError::validation().with_field("file", "OPML contains a forbidden XML construct")
        }
        OpmlError::Malformed => ApiError::validation().with_field("file", "OPML file is malformed"),
        OpmlError::SubscriptionLimit => ApiError::new(
            StatusCode::CONFLICT,
            "SUBSCRIPTION_LIMIT_REACHED",
            "Subscription limit reached",
        ),
        OpmlError::CategoryLimit => ApiError::new(
            StatusCode::CONFLICT,
            "CATEGORY_LIMIT_REACHED",
            "Category limit reached",
        ),
        OpmlError::IdentityCollision => ApiError::new(
            StatusCode::CONFLICT,
            "CONFLICT",
            "The request conflicts with state",
        ),
        OpmlError::InvalidUserId | OpmlError::CorruptData | OpmlError::Database(_) => {
            ApiError::internal()
        }
    }
}
