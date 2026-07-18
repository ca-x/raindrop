use cookie::{Cookie, SameSite};
use secrecy::ExposeSecret;
use time::{Duration, OffsetDateTime};

use super::CreatedSession;

pub const SESSION_COOKIE_NAME: &str = "raindrop_session";

#[must_use]
pub fn build_session_cookie(session: &CreatedSession, secure: bool) -> Cookie<'static> {
    Cookie::build((
        SESSION_COOKIE_NAME,
        session.cookie_token.expose_secret().to_owned(),
    ))
    .http_only(true)
    .same_site(SameSite::Lax)
    .secure(secure)
    .path("/")
    .expires(session.expires_at)
    .build()
}

#[must_use]
pub fn build_clear_session_cookie(secure: bool) -> Cookie<'static> {
    Cookie::build((SESSION_COOKIE_NAME, ""))
        .http_only(true)
        .same_site(SameSite::Lax)
        .secure(secure)
        .path("/")
        .max_age(Duration::ZERO)
        .expires(OffsetDateTime::UNIX_EPOCH)
        .build()
}
