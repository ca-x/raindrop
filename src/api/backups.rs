use std::sync::Arc;

use axum::{
    Json, Router,
    extract::{FromRequestParts, Path, State},
    http::{HeaderValue, StatusCode, header::LOCATION, request::Parts},
    middleware,
    response::{IntoResponse, Response},
    routing::{get, patch, post},
};
use secrecy::SecretString;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

use crate::{
    app::AppState,
    auth::{CsrfGuard, CurrentUser},
    backups::{
        BackupError, BackupErrorKind, BackupJob, BackupPublicConfig, BackupRepository,
        BackupSchedule, BackupSecretConfig, BackupTarget, BackupTransport, CreateBackupTarget,
        ProductionBackupTransport, RetentionPolicy, S3SecretConfig, UpdateBackupTarget,
        WebDavSecretConfig,
    },
};

use super::{ApiError, ApiJson, RateLimitRejection, routes::sensitive_cache_headers};

pub(super) fn router() -> Router<AppState> {
    let targets = Router::new()
        .route("/", get(list_targets).post(create_target))
        .route("/{target_id}", patch(update_target).delete(delete_target))
        .route("/{target_id}/test", post(test_target));
    let backups = Router::new()
        .nest("/targets", targets)
        .route("/schedule", get(get_schedule).put(put_schedule))
        .route("/jobs", get(list_jobs).post(create_job))
        .route("/jobs/{job_id}", get(get_job))
        .fallback(backup_not_found)
        .method_not_allowed_fallback(backup_method_not_allowed);
    Router::new()
        .route("/api/v1/backups/", axum::routing::any(backup_not_found))
        .nest("/api/v1/backups", backups)
        .layer(middleware::map_response(sensitive_cache_headers))
}

async fn backup_not_found() -> ApiError {
    ApiError::not_found()
}

async fn backup_method_not_allowed() -> ApiError {
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
struct CreateTargetRequest {
    display_name: String,
    #[serde(default = "default_true")]
    enabled: bool,
    config: BackupPublicConfig,
    credentials: SecretRequest,
    #[serde(default)]
    retention: RetentionPolicy,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct UpdateTargetRequest {
    display_name: String,
    enabled: bool,
    config: BackupPublicConfig,
    credentials: Option<SecretRequest>,
    #[serde(default)]
    retention: RetentionPolicy,
}

#[derive(Deserialize)]
#[serde(tag = "kind", content = "values", rename_all = "SCREAMING_SNAKE_CASE")]
enum SecretRequest {
    S3 {
        #[serde(rename = "accessKeyId")]
        access_key_id: String,
        #[serde(rename = "secretAccessKey")]
        secret_access_key: String,
        #[serde(rename = "sessionToken")]
        session_token: Option<String>,
    },
    Webdav {
        username: String,
        password: String,
    },
}

impl From<SecretRequest> for BackupSecretConfig {
    fn from(value: SecretRequest) -> Self {
        match value {
            SecretRequest::S3 {
                access_key_id,
                secret_access_key,
                session_token,
            } => Self::S3(S3SecretConfig {
                access_key_id: SecretString::from(access_key_id),
                secret_access_key: SecretString::from(secret_access_key),
                session_token: session_token.map(SecretString::from),
            }),
            SecretRequest::Webdav { username, password } => Self::Webdav(WebDavSecretConfig {
                username: SecretString::from(username),
                password: SecretString::from(password),
            }),
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PutScheduleRequest {
    enabled: bool,
    interval_hours: u16,
    target_ids: Vec<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CreateJobRequest {
    target_ids: Vec<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct JobQuery {
    since: Option<String>,
    limit: Option<u16>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct TargetListResponse {
    items: Vec<BackupTarget>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct JobListResponse {
    items: Vec<BackupJob>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct TestTargetResponse {
    ok: bool,
}

async fn list_targets(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
) -> Result<Json<TargetListResponse>, ApiError> {
    let items = repository(&state)?
        .list_targets(&user.id)
        .await
        .map_err(map_backup_error)?;
    Ok(Json(TargetListResponse { items }))
}

async fn create_target(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    _csrf: CsrfGuard,
    ApiJson(request): ApiJson<CreateTargetRequest>,
) -> Result<Response, ApiError> {
    admit_mutation(&state, &user.id)?;
    let target = state
        .commit_and_notify_backup_runtime(repository(&state)?.create_target(
            &user.id,
            CreateBackupTarget {
                display_name: request.display_name,
                enabled: request.enabled,
                config: request.config,
                secret: request.credentials.into(),
                retention: request.retention,
            },
        ))
        .await
        .map_err(map_backup_error)?;
    let location = HeaderValue::from_str(&format!("/api/v1/backups/targets/{}", target.target_id))
        .map_err(|_| ApiError::internal())?;
    Ok((StatusCode::CREATED, [(LOCATION, location)], Json(target)).into_response())
}

async fn update_target(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    _csrf: CsrfGuard,
    ApiPath(target_id): ApiPath<String>,
    ApiJson(request): ApiJson<UpdateTargetRequest>,
) -> Result<Json<BackupTarget>, ApiError> {
    admit_mutation(&state, &user.id)?;
    let target = state
        .commit_and_notify_backup_runtime(repository(&state)?.update_target(
            &user.id,
            &target_id,
            UpdateBackupTarget {
                display_name: request.display_name,
                enabled: request.enabled,
                config: request.config,
                secret: request.credentials.map(Into::into),
                retention: request.retention,
            },
        ))
        .await
        .map_err(map_backup_error)?;
    Ok(Json(target))
}

async fn delete_target(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    _csrf: CsrfGuard,
    ApiPath(target_id): ApiPath<String>,
) -> Result<StatusCode, ApiError> {
    admit_mutation(&state, &user.id)?;
    state
        .commit_and_notify_backup_runtime(repository(&state)?.delete_target(&user.id, &target_id))
        .await
        .map_err(map_backup_error)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn test_target(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    _csrf: CsrfGuard,
    ApiPath(target_id): ApiPath<String>,
) -> Result<Json<TestTargetResponse>, ApiError> {
    admit_mutation(&state, &user.id)?;
    let target = repository(&state)?
        .execution_target_for_test(&user.id, &target_id)
        .await
        .map_err(map_backup_error)?;
    transport(&state)?
        .test(&target)
        .await
        .map_err(map_backup_error)?;
    Ok(Json(TestTargetResponse { ok: true }))
}

async fn get_schedule(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
) -> Result<Json<BackupSchedule>, ApiError> {
    repository(&state)?
        .get_schedule(&user.id)
        .await
        .map(Json)
        .map_err(map_backup_error)
}

async fn put_schedule(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    _csrf: CsrfGuard,
    ApiJson(request): ApiJson<PutScheduleRequest>,
) -> Result<Json<BackupSchedule>, ApiError> {
    admit_mutation(&state, &user.id)?;
    state
        .commit_and_notify_backup_runtime(repository(&state)?.put_schedule(
            &user.id,
            request.enabled,
            request.interval_hours,
            &request.target_ids,
        ))
        .await
        .map(Json)
        .map_err(map_backup_error)
}

async fn create_job(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    _csrf: CsrfGuard,
    ApiJson(request): ApiJson<CreateJobRequest>,
) -> Result<Response, ApiError> {
    admit_mutation(&state, &user.id)?;
    let job = state
        .commit_and_notify_backup_runtime(
            repository(&state)?.enqueue_manual(&user.id, &request.target_ids),
        )
        .await
        .map_err(map_backup_error)?;
    let location = HeaderValue::from_str(&format!("/api/v1/backups/jobs/{}", job.job_id))
        .map_err(|_| ApiError::internal())?;
    Ok((StatusCode::ACCEPTED, [(LOCATION, location)], Json(job)).into_response())
}

async fn list_jobs(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    ApiQuery(query): ApiQuery<JobQuery>,
) -> Result<Json<JobListResponse>, ApiError> {
    let since = query
        .since
        .as_deref()
        .map(|value| {
            OffsetDateTime::parse(value, &Rfc3339)
                .map_err(|_| ApiError::validation().with_field("since", "Timestamp is invalid"))
        })
        .transpose()?;
    let items = repository(&state)?
        .list_jobs(&user.id, since, query.limit.unwrap_or(50))
        .await
        .map_err(map_backup_error)?;
    Ok(Json(JobListResponse { items }))
}

async fn get_job(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    ApiPath(job_id): ApiPath<String>,
) -> Result<Json<BackupJob>, ApiError> {
    repository(&state)?
        .get_job(&user.id, &job_id)
        .await
        .map(Json)
        .map_err(map_backup_error)
}

fn repository(state: &AppState) -> Result<BackupRepository, ApiError> {
    state
        .setup
        .database()
        .map(|database| BackupRepository::new(database, state.provider_keyring()))
        .map_err(|_| ApiError::internal())
}

fn transport(state: &AppState) -> Result<Arc<dyn BackupTransport>, ApiError> {
    if let Some(transport) = &state.backup_transport {
        return Ok(Arc::clone(transport));
    }
    ProductionBackupTransport::new()
        .map(|transport| Arc::new(transport) as Arc<dyn BackupTransport>)
        .map_err(map_backup_error)
}

fn admit_mutation(state: &AppState, user_id: &str) -> Result<(), ApiError> {
    state
        .backup_mutation_limiter
        .check(user_id)
        .map_err(map_limiter_rejection)
}

fn map_limiter_rejection(rejection: RateLimitRejection) -> ApiError {
    ApiError::rate_limited_with_retry(
        rejection
            .retry_at
            .format(&Rfc3339)
            .unwrap_or_else(|_| rejection.retry_at.unix_timestamp().to_string()),
        rejection.retry_after_seconds,
    )
}

fn map_backup_error(error: BackupError) -> ApiError {
    match error.kind() {
        BackupErrorKind::InvalidInput => ApiError::validation(),
        BackupErrorKind::NotFound => ApiError::not_found(),
        BackupErrorKind::Conflict => ApiError::new(
            StatusCode::CONFLICT,
            "CONFLICT",
            "The request conflicts with state",
        ),
        BackupErrorKind::SecretUnavailable => ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "BACKUP_KEYRING_UNAVAILABLE",
            "Backup credentials are unavailable",
        ),
        BackupErrorKind::TargetChanged => ApiError::new(
            StatusCode::CONFLICT,
            "TARGET_CHANGED",
            "The backup target changed before execution",
        ),
        BackupErrorKind::TargetUnreachable => ApiError::new(
            StatusCode::BAD_GATEWAY,
            "TARGET_UNREACHABLE",
            "The backup target is unavailable",
        ),
        BackupErrorKind::TargetAuthentication => ApiError::new(
            StatusCode::BAD_GATEWAY,
            "TARGET_AUTH_FAILED",
            "The backup target rejected authentication",
        ),
        BackupErrorKind::TargetProtocol => ApiError::new(
            StatusCode::BAD_GATEWAY,
            "TARGET_PROTOCOL_ERROR",
            "The backup target returned an invalid response",
        ),
        BackupErrorKind::ExportFailed => ApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "BACKUP_EXPORT_FAILED",
            "The subscription export could not be created",
        ),
        BackupErrorKind::LeaseLost | BackupErrorKind::CorruptData | BackupErrorKind::Database => {
            ApiError::internal()
        }
    }
}

const fn default_true() -> bool {
    true
}
