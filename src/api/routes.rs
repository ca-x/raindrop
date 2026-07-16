use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, State},
    http::{HeaderMap, StatusCode, header::SET_COOKIE},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};
use time::format_description::well_known::Rfc3339;

use crate::{
    app::AppState,
    auth::{
        AuthenticateError, CreateAdminError, CsrfGuard, LoginIdentifier, PasswordService,
        SessionError, SessionToken, authenticate, build_clear_session_cookie, build_session_cookie,
    },
    config::DatabaseKind,
    setup::{SetupCompleteInput, SetupError},
};

use super::{ApiError, ApiJson};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/v1/bootstrap", get(bootstrap))
        .route("/api/v1/setup/database-check", post(database_check))
        .route("/api/v1/setup/complete", post(setup_complete))
        .route("/api/v1/auth/login", post(login))
        .route("/api/v1/auth/logout", post(logout))
        .route("/api/v1/auth/session", get(session))
        .layer(DefaultBodyLimit::max(64 * 1024))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct BootstrapResponse {
    status: &'static str,
    version: &'static str,
}

async fn bootstrap(State(state): State<AppState>) -> Json<BootstrapResponse> {
    Json(BootstrapResponse {
        status: if state.setup.is_ready() {
            "READY"
        } else {
            "SETUP_REQUIRED"
        },
        version: state.version,
    })
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct LoginRequest {
    login: String,
    password: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SessionResponse {
    user: crate::auth::User,
    csrf_token: String,
    expires_at: String,
}

async fn login(
    State(state): State<AppState>,
    ApiJson(request): ApiJson<LoginRequest>,
) -> Result<Response, ApiError> {
    if !state.setup.is_ready() {
        return Err(ApiError::setup_required());
    }
    if request.login.trim().is_empty()
        || request.login.len() > 320
        || request.password.is_empty()
        || request.password.len() > 1024
    {
        return Err(ApiError::invalid_credentials());
    }
    let login_key = blake3::hash(request.login.trim().to_lowercase().as_bytes())
        .to_hex()
        .to_string();
    if !state.login_limiter.check(&login_key) {
        return Err(ApiError::rate_limited());
    }
    let database = state.setup.database().map_err(map_setup_error)?;
    let user = authenticate(
        &database,
        &PasswordService::default(),
        LoginIdentifier::new(request.login),
        &SecretString::from(request.password),
    )
    .await
    .map_err(map_authenticate_error)?;
    let created = state
        .setup
        .sessions()
        .create(&user.id)
        .await
        .map_err(map_session_error)?;
    let cookie = build_session_cookie(&created, state.setup.secure_cookie());
    let response = SessionResponse {
        user,
        csrf_token: created.csrf_token.expose_secret().to_owned(),
        expires_at: format_time(created.expires_at)?,
    };
    Ok(([(SET_COOKIE, cookie.to_string())], Json(response)).into_response())
}

async fn session(
    State(state): State<AppState>,
    token: SessionToken,
) -> Result<Json<SessionResponse>, ApiError> {
    let refreshed = state
        .setup
        .sessions()
        .details(token.as_secret())
        .await
        .map_err(map_session_error)?;
    Ok(Json(SessionResponse {
        user: refreshed.user,
        csrf_token: refreshed.csrf_token.expose_secret().to_owned(),
        expires_at: format_time(refreshed.expires_at)?,
    }))
}

async fn logout(
    State(state): State<AppState>,
    _csrf: CsrfGuard,
    token: SessionToken,
) -> Result<Response, ApiError> {
    state
        .setup
        .sessions()
        .revoke(token.as_secret())
        .await
        .map_err(map_session_error)?;
    let cookie = build_clear_session_cookie(state.setup.secure_cookie());
    Ok((StatusCode::NO_CONTENT, [(SET_COOKIE, cookie.to_string())]).into_response())
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct DatabaseCheckRequest {
    database_url: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DatabaseCheckResponse {
    status: &'static str,
    database_kind: &'static str,
}

async fn database_check(
    State(state): State<AppState>,
    headers: HeaderMap,
    ApiJson(request): ApiJson<DatabaseCheckRequest>,
) -> Result<Json<DatabaseCheckResponse>, ApiError> {
    let token = setup_token(&headers)?;
    state.setup.require_token(token).map_err(map_setup_error)?;
    if !state.setup_limiter.check("authorized-setup") {
        return Err(ApiError::rate_limited());
    }
    let kind = state
        .setup
        .database_check(token, &request.database_url)
        .await
        .map_err(map_setup_error)?;
    Ok(Json(DatabaseCheckResponse {
        status: "OK",
        database_kind: database_kind_name(kind),
    }))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SetupCompleteRequest {
    database_url: String,
    username: String,
    password: String,
    email: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SetupCompleteResponse {
    status: &'static str,
    user: crate::auth::User,
}

async fn setup_complete(
    State(state): State<AppState>,
    headers: HeaderMap,
    ApiJson(request): ApiJson<SetupCompleteRequest>,
) -> Result<Json<SetupCompleteResponse>, ApiError> {
    let token = setup_token(&headers)?;
    state.setup.require_token(token).map_err(map_setup_error)?;
    if !state.setup_limiter.check("authorized-setup") {
        return Err(ApiError::rate_limited());
    }
    let user = state
        .setup
        .complete(
            token,
            SetupCompleteInput {
                database_url: secrecy::SecretString::from(request.database_url),
                username: request.username,
                password: secrecy::SecretString::from(request.password),
                email: request.email,
            },
        )
        .await
        .map_err(map_setup_error)?;
    Ok(Json(SetupCompleteResponse {
        status: "READY",
        user,
    }))
}

fn setup_token(headers: &HeaderMap) -> Result<&str, ApiError> {
    let mut values = headers.get_all("x-setup-token").iter();
    let Some(value) = values.next() else {
        return Err(ApiError::setup_token_required());
    };
    if values.next().is_some() {
        return Err(ApiError::setup_token_required());
    }
    value.to_str().map_err(|_| ApiError::setup_token_required())
}

fn map_setup_error(error: SetupError) -> ApiError {
    match error {
        SetupError::Unauthorized => ApiError::setup_token_required(),
        SetupError::AlreadyComplete => ApiError::setup_already_complete(),
        SetupError::InvalidDatabase | SetupError::Database(_) => ApiError::database_url_invalid(),
        SetupError::CreateAdmin(CreateAdminError::InvalidUsername(_)) => {
            ApiError::username_invalid("Username must contain 3 to 64 non-space characters")
        }
        SetupError::CreateAdmin(CreateAdminError::InvalidPassword) => ApiError::password_invalid(),
        SetupError::CreateAdmin(CreateAdminError::UsernameTaken) => {
            ApiError::setup_already_complete()
        }
        SetupError::NotReady
        | SetupError::CreateAdmin(CreateAdminError::Password(_))
        | SetupError::CreateAdmin(CreateAdminError::Database(_))
        | SetupError::ParseConfig(_)
        | SetupError::SerializeConfig(_)
        | SetupError::WriteConfig(_) => ApiError::internal(),
    }
}

fn map_authenticate_error(error: AuthenticateError) -> ApiError {
    match error {
        AuthenticateError::InvalidCredentials | AuthenticateError::Disabled => {
            ApiError::invalid_credentials()
        }
        AuthenticateError::Password(_) | AuthenticateError::Database(_) => ApiError::internal(),
    }
}

fn map_session_error(error: SessionError) -> ApiError {
    match error {
        SessionError::Invalid | SessionError::Expired | SessionError::Disabled => {
            ApiError::authentication_required()
        }
        SessionError::Unavailable => ApiError::setup_required(),
        SessionError::Database(_) => ApiError::internal(),
    }
}

fn format_time(value: time::OffsetDateTime) -> Result<String, ApiError> {
    value.format(&Rfc3339).map_err(|_| ApiError::internal())
}

const fn database_kind_name(kind: DatabaseKind) -> &'static str {
    match kind {
        DatabaseKind::Sqlite => "SQLITE",
        DatabaseKind::Postgres => "POSTGRESQL",
        DatabaseKind::MySql => "MYSQL",
    }
}
