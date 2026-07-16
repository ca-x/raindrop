use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use rand_core::{OsRng, RngCore};
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter,
    sea_query::Expr,
};
use secrecy::{ExposeSecret, SecretString};
use time::{Duration, OffsetDateTime};
use zeroize::{Zeroize, Zeroizing};

use crate::db::entities::session;

use super::{User, users::load_user_by_id};

const SESSION_LIFETIME: Duration = Duration::days(30);
const LAST_SEEN_WRITE_INTERVAL: Duration = Duration::minutes(15);

#[derive(Clone)]
pub struct SessionService {
    database: DatabaseConnection,
}

impl SessionService {
    #[must_use]
    pub fn new(database: DatabaseConnection) -> Self {
        Self { database }
    }

    pub async fn create(&self, user_id: &str) -> Result<CreatedSession, SessionError> {
        let cookie_token = generate_token();
        let csrf_token = generate_token();
        let created_at = OffsetDateTime::now_utc();
        let expires_at = created_at + SESSION_LIFETIME;

        session::ActiveModel {
            token_hash: Set(hash_token(&cookie_token)),
            user_id: Set(user_id.to_owned()),
            csrf_hash: Set(hash_token(&csrf_token)),
            created_at: Set(created_at),
            last_seen_at: Set(created_at),
            expires_at: Set(expires_at),
        }
        .insert(&self.database)
        .await
        .map_err(SessionError::Database)?;

        Ok(CreatedSession {
            cookie_token,
            csrf_token,
            expires_at,
        })
    }

    pub async fn revoke(&self, cookie_token: &SecretString) -> Result<(), SessionError> {
        session::Entity::delete_by_id(hash_token(cookie_token))
            .exec(&self.database)
            .await
            .map_err(SessionError::Database)?;
        Ok(())
    }

    pub(crate) async fn resolve(
        &self,
        cookie_token: &SecretString,
    ) -> Result<AuthenticatedSession, SessionError> {
        let token_hash = hash_token(cookie_token);
        let stored = session::Entity::find_by_id(&token_hash)
            .one(&self.database)
            .await
            .map_err(SessionError::Database)?
            .ok_or(SessionError::Invalid)?;
        let now = OffsetDateTime::now_utc();
        if stored.expires_at <= now {
            return Err(SessionError::Expired);
        }
        let user = load_user_by_id(&self.database, &stored.user_id)
            .await
            .map_err(SessionError::Database)?
            .ok_or(SessionError::Invalid)?;
        if user.is_disabled {
            return Err(SessionError::Disabled);
        }
        if stored.last_seen_at <= now - LAST_SEEN_WRITE_INTERVAL {
            session::Entity::update_many()
                .col_expr(session::Column::LastSeenAt, Expr::value(now))
                .filter(session::Column::TokenHash.eq(&token_hash))
                .filter(session::Column::LastSeenAt.lte(now - LAST_SEEN_WRITE_INTERVAL))
                .exec(&self.database)
                .await
                .map_err(SessionError::Database)?;
        }

        Ok(AuthenticatedSession {
            user,
            csrf_hash: stored.csrf_hash,
        })
    }
}

#[derive(Debug)]
pub struct CreatedSession {
    pub cookie_token: SecretString,
    pub csrf_token: SecretString,
    pub expires_at: OffsetDateTime,
}

#[derive(Debug, thiserror::Error)]
pub enum SessionError {
    #[error("session is invalid")]
    Invalid,
    #[error("session has expired")]
    Expired,
    #[error("account is disabled")]
    Disabled,
    #[error("database operation failed")]
    Database(#[source] sea_orm::DbErr),
}

#[derive(Clone)]
pub(crate) struct AuthenticatedSession {
    pub user: User,
    pub csrf_hash: String,
}

pub(crate) fn hash_token(token: &SecretString) -> String {
    blake3::hash(token.expose_secret().as_bytes())
        .to_hex()
        .to_string()
}

pub(crate) fn parse_token(value: &str) -> Option<SecretString> {
    if value.len() != 43 {
        return None;
    }
    let decoded = Zeroizing::new(URL_SAFE_NO_PAD.decode(value).ok()?);
    (decoded.len() == 32).then(|| SecretString::from(value.to_owned()))
}

fn generate_token() -> SecretString {
    let mut bytes = [0_u8; 32];
    OsRng.fill_bytes(&mut bytes);
    let token = URL_SAFE_NO_PAD.encode(bytes);
    bytes.zeroize();
    SecretString::from(token)
}
