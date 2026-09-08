use std::sync::Arc;

use axum::{
    Json, Router,
    extract::State,
    http::StatusCode,
    middleware,
    routing::{get, post},
};
use serde::Serialize;
use tokio::sync::Mutex;

use super::{ApiError, ApiJson, routes::sensitive_cache_headers};
use crate::{
    app::AppState,
    auth::{CsrfGuard, CurrentUser},
    db::maintenance::{self, CompactionResult, DatabaseStorage, MaintenanceError},
};

#[derive(Clone, Default)]
pub(crate) struct DatabaseMaintenance(pub Arc<Mutex<MaintenanceStatus>>);

#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MaintenanceStatus {
    running: bool,
    storage: Option<DatabaseStorage>,
    result: Option<CompactionResult>,
    error: Option<&'static str>,
    article_retention: crate::feeds::ArticleRetentionSettings,
}

pub(super) fn router() -> Router<AppState> {
    Router::new()
        .route("/api/v1/database", get(status))
        .route("/api/v1/database/compact", post(compact))
        .route(
            "/api/v1/database/article-retention",
            axum::routing::put(save_article_retention),
        )
        .layer(middleware::map_response(sensitive_cache_headers))
}

async fn status(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
) -> Result<Json<MaintenanceStatus>, ApiError> {
    if !user.is_admin() {
        return Err(ApiError::forbidden());
    }
    // Serialize diagnostics with starting maintenance; repeated polls during VACUUM never acquire
    // the busy write connection. This also prevents a stale GET from replacing completion state.
    let mut status = state.database_maintenance.0.lock().await;
    if !status.running {
        let database = state
            .setup
            .reader_database()
            .map_err(|_| ApiError::internal())?;
        status.storage = Some(maintenance::storage(&database).await.map_err(map_error)?);
        status.article_retention = crate::feeds::ArticleRetentionSettings::load(&database)
            .await
            .map_err(|_| ApiError::internal())?;
    }
    Ok(Json(status.clone()))
}

async fn compact(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    _csrf: CsrfGuard,
) -> Result<(StatusCode, Json<MaintenanceStatus>), ApiError> {
    if !user.is_admin() {
        return Err(ApiError::forbidden());
    }
    let mut status = state.database_maintenance.0.lock().await;
    if status.running {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "DATABASE_BUSY",
            "Database compaction is already running",
        ));
    }
    let database = state.setup.database().map_err(|_| ApiError::internal())?;
    // Reject unsupported databases before accepting a background task.
    let reader = state
        .setup
        .reader_database()
        .map_err(|_| ApiError::internal())?;
    status.storage = Some(maintenance::storage(&reader).await.map_err(map_error)?);
    status.running = true;
    status.result = None;
    status.error = None;
    let response = status.clone();
    let progress = state.database_maintenance.clone();
    // A disconnected browser must not cancel VACUUM or release the operation guard early.
    tokio::spawn(async move {
        let task = tokio::spawn(async move { maintenance::compact(&database).await });
        let outcome = task.await;
        let mut status = progress.0.lock().await;
        match outcome {
            Ok(Ok(result)) => {
                status.storage = Some(result.after.clone());
                status.result = Some(result);
            }
            _ => status.error = Some("DATABASE_COMPACTION_FAILED"),
        }
        status.running = false;
    });
    Ok((StatusCode::ACCEPTED, Json(response)))
}

fn map_error(error: MaintenanceError) -> ApiError {
    match error {
        MaintenanceError::Unsupported => ApiError::new(
            StatusCode::BAD_REQUEST,
            "DATABASE_UNSUPPORTED",
            "Database compaction requires file-backed SQLite",
        ),
        MaintenanceError::Database(_) | MaintenanceError::Retention(_) => ApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "DATABASE_STORAGE_FAILED",
            "Could not inspect database storage",
        ),
    }
}

async fn save_article_retention(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    _csrf: CsrfGuard,
    ApiJson(settings): ApiJson<crate::feeds::ArticleRetentionSettings>,
) -> Result<Json<crate::feeds::ArticleRetentionSettings>, ApiError> {
    if !user.is_admin() {
        return Err(ApiError::forbidden());
    }
    settings.validate().map_err(|_| ApiError::validation())?;
    let database = state.setup.database().map_err(|_| ApiError::internal())?;
    settings
        .save(&database)
        .await
        .map_err(|_| ApiError::internal())?;
    state.database_maintenance.0.lock().await.article_retention = settings.clone();
    Ok(Json(settings))
}
