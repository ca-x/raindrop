use axum::{
    extract::{FromRef, FromRequestParts},
    http::{
        HeaderMap,
        header::{COOKIE, HOST, ORIGIN},
        request::Parts,
        uri::Authority,
    },
    response::{IntoResponse, Response},
};
use secrecy::SecretString;
use url::Url;

use super::{
    SessionError, SessionService, User,
    sessions::{AuthenticatedSession, hash_token, parse_token},
};

pub const CSRF_HEADER_NAME: &str = "x-csrf-token";

pub struct CurrentUser(pub User);

pub struct SessionToken(SecretString);

impl SessionToken {
    #[must_use]
    pub fn as_secret(&self) -> &SecretString {
        &self.0
    }
}

impl<S> FromRequestParts<S> for SessionToken
where
    S: Send + Sync,
{
    type Rejection = AuthRejection;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        session_cookie(&parts.headers)
            .map(Self)
            .ok_or(AuthRejection::Unauthenticated)
    }
}

impl<S> FromRequestParts<S> for CurrentUser
where
    S: Send + Sync,
    SessionService: FromRef<S>,
{
    type Rejection = AuthRejection;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let authenticated = authenticated_session(parts, state).await?;
        Ok(Self(authenticated.user))
    }
}

pub struct CsrfGuard;

impl<S> FromRequestParts<S> for CsrfGuard
where
    S: Send + Sync,
    SessionService: FromRef<S>,
{
    type Rejection = AuthRejection;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let authenticated = authenticated_session(parts, state).await?;
        let csrf_value =
            single_header(&parts.headers, CSRF_HEADER_NAME)?.ok_or(AuthRejection::Forbidden)?;
        let csrf_token = parse_token(csrf_value).ok_or(AuthRejection::Forbidden)?;
        let candidate_hash = hash_token(&csrf_token);
        if !constant_time_eq::constant_time_eq(
            candidate_hash.as_bytes(),
            authenticated.csrf_hash.as_bytes(),
        ) {
            return Err(AuthRejection::Forbidden);
        }
        verify_origin(&parts.headers)?;
        Ok(Self)
    }
}

#[derive(Debug, Clone, Copy)]
pub enum AuthRejection {
    Unauthenticated,
    Forbidden,
    Internal,
}

impl IntoResponse for AuthRejection {
    fn into_response(self) -> Response {
        match self {
            Self::Unauthenticated => {
                crate::api::ApiError::authentication_required().into_response()
            }
            Self::Forbidden => crate::api::ApiError::forbidden().into_response(),
            Self::Internal => crate::api::ApiError::internal().into_response(),
        }
    }
}

async fn authenticated_session<S>(
    parts: &mut Parts,
    state: &S,
) -> Result<AuthenticatedSession, AuthRejection>
where
    S: Send + Sync,
    SessionService: FromRef<S>,
{
    if let Some(session) = parts.extensions.get::<AuthenticatedSession>() {
        return Ok(session.clone());
    }

    let cookie_token = session_cookie(&parts.headers).ok_or(AuthRejection::Unauthenticated)?;
    let service = SessionService::from_ref(state);
    let authenticated = service
        .resolve(&cookie_token)
        .await
        .map_err(|error| match error {
            SessionError::Invalid | SessionError::Expired | SessionError::Disabled => {
                AuthRejection::Unauthenticated
            }
            SessionError::Unavailable | SessionError::Database(_) => AuthRejection::Internal,
        })?;
    parts.extensions.insert(authenticated.clone());
    Ok(authenticated)
}

fn session_cookie(headers: &axum::http::HeaderMap) -> Option<SecretString> {
    let mut found = None;
    for value in headers.get_all(COOKIE) {
        let value = value.to_str().ok()?;
        for pair in value.split(';') {
            let (name, value) = pair.trim().split_once('=')?;
            if name == super::SESSION_COOKIE_NAME {
                if found.is_some() {
                    return None;
                }
                found = parse_token(value);
                found.as_ref()?;
            }
        }
    }
    found
}

fn single_header<'a>(headers: &'a HeaderMap, name: &str) -> Result<Option<&'a str>, AuthRejection> {
    let mut values = headers.get_all(name).iter();
    let Some(value) = values.next() else {
        return Ok(None);
    };
    if values.next().is_some() {
        return Err(AuthRejection::Forbidden);
    }
    value
        .to_str()
        .map(Some)
        .map_err(|_| AuthRejection::Forbidden)
}

fn verify_origin(headers: &HeaderMap) -> Result<(), AuthRejection> {
    let Some(origin) = single_header(headers, ORIGIN.as_str())? else {
        return Ok(());
    };
    let host = single_header(headers, HOST.as_str())?
        .ok_or(AuthRejection::Forbidden)?
        .parse::<Authority>()
        .map_err(|_| AuthRejection::Forbidden)?;
    let origin = Url::parse(origin).map_err(|_| AuthRejection::Forbidden)?;
    if !matches!(origin.scheme(), "http" | "https")
        || !origin.username().is_empty()
        || origin.password().is_some()
        || origin.path() != "/"
        || origin.query().is_some()
        || origin.fragment().is_some()
    {
        return Err(AuthRejection::Forbidden);
    }
    let origin_host = origin
        .host_str()
        .ok_or(AuthRejection::Forbidden)?
        .trim_matches(['[', ']']);
    if !origin_host.eq_ignore_ascii_case(host.host().trim_matches(['[', ']'])) {
        return Err(AuthRejection::Forbidden);
    }
    let default_port = match origin.scheme() {
        "http" => 80,
        "https" => 443,
        _ => return Err(AuthRejection::Forbidden),
    };
    if origin.port_or_known_default() != Some(host.port_u16().unwrap_or(default_port)) {
        return Err(AuthRejection::Forbidden);
    }
    Ok(())
}
