use std::sync::{Arc, RwLock};

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use rand_core::{OsRng, RngCore};
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, ConnectionTrait, DatabaseBackend,
    DatabaseConnection, DbErr, EntityTrait, QueryFilter, sea_query::Expr,
};
use secrecy::{ExposeSecret, SecretString};
use time::{Duration, OffsetDateTime};
use tokio::sync::Semaphore;
use zeroize::{Zeroize, Zeroizing};

use crate::db::entities::session;

use super::{User, users::load_user_by_id};

const SESSION_LIFETIME: Duration = Duration::days(30);
const LAST_SEEN_WRITE_INTERVAL: Duration = Duration::minutes(15);
const CSRF_DERIVATION_CONTEXT: &str = "raindrop.browser.csrf-token.v1";

#[derive(Clone)]
pub struct SessionService {
    databases: Arc<RwLock<Option<SessionDatabases>>>,
    last_seen_update_slot: Arc<Semaphore>,
}

#[derive(Clone)]
struct SessionDatabases {
    primary: DatabaseConnection,
    reader: DatabaseConnection,
}

impl SessionService {
    #[must_use]
    pub fn new(database: DatabaseConnection) -> Self {
        Self::with_reader(database.clone(), database)
    }

    #[must_use]
    pub fn with_reader(primary: DatabaseConnection, reader: DatabaseConnection) -> Self {
        Self {
            databases: Arc::new(RwLock::new(Some(SessionDatabases { primary, reader }))),
            last_seen_update_slot: Arc::new(Semaphore::new(1)),
        }
    }

    #[must_use]
    pub fn unavailable() -> Self {
        Self {
            databases: Arc::new(RwLock::new(None)),
            last_seen_update_slot: Arc::new(Semaphore::new(1)),
        }
    }

    pub(crate) fn attach_databases(&self, primary: DatabaseConnection, reader: DatabaseConnection) {
        *self
            .databases
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner) =
            Some(SessionDatabases { primary, reader });
    }

    pub(crate) fn database(&self) -> Result<DatabaseConnection, SessionError> {
        self.databases
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .as_ref()
            .map(|databases| databases.primary.clone())
            .ok_or(SessionError::Unavailable)
    }

    pub(crate) fn reader_database(&self) -> Result<DatabaseConnection, SessionError> {
        self.databases
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .as_ref()
            .map(|databases| databases.reader.clone())
            .ok_or(SessionError::Unavailable)
    }

    pub async fn create(&self, user_id: &str) -> Result<CreatedSession, SessionError> {
        let database = self.database()?;
        let cookie_token = generate_token();
        let csrf_token = derive_csrf_token(&cookie_token);
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
        .insert(&database)
        .await
        .map_err(SessionError::Database)?;

        Ok(CreatedSession {
            cookie_token,
            csrf_token,
            expires_at,
        })
    }

    pub async fn revoke(&self, cookie_token: &SecretString) -> Result<(), SessionError> {
        let database = self.database()?;
        session::Entity::delete_by_id(hash_token(cookie_token))
            .exec(&database)
            .await
            .map_err(SessionError::Database)?;
        Ok(())
    }

    pub async fn details(
        &self,
        cookie_token: &SecretString,
    ) -> Result<SessionDetails, SessionError> {
        let authenticated = self.resolve(cookie_token).await?;
        let csrf_token = derive_csrf_token(cookie_token);
        let candidate_hash = hash_token(&csrf_token);
        if !constant_time_eq::constant_time_eq(
            candidate_hash.as_bytes(),
            authenticated.csrf_hash.as_bytes(),
        ) {
            return Err(SessionError::Invalid);
        }
        Ok(SessionDetails {
            user: authenticated.user,
            csrf_token,
            expires_at: authenticated.expires_at,
        })
    }

    pub(crate) async fn resolve(
        &self,
        cookie_token: &SecretString,
    ) -> Result<AuthenticatedSession, SessionError> {
        let reader = self.reader_database()?;
        let token_hash = hash_token(cookie_token);
        let stored = session::Entity::find_by_id(&token_hash)
            .one(&reader)
            .await
            .map_err(SessionError::Database)?
            .ok_or(SessionError::Invalid)?;
        let now = OffsetDateTime::now_utc();
        if stored.expires_at <= now {
            return Err(SessionError::Expired);
        }
        let user = load_user_by_id(&reader, &stored.user_id)
            .await
            .map_err(SessionError::Database)?
            .ok_or(SessionError::Invalid)?;
        if user.is_disabled {
            return Err(SessionError::Disabled);
        }
        if stored.last_seen_at <= now - LAST_SEEN_WRITE_INTERVAL {
            let database = self.database()?;
            if database.get_database_backend() == DatabaseBackend::Sqlite {
                if let Ok(permit) = Arc::clone(&self.last_seen_update_slot).try_acquire_owned() {
                    tokio::spawn(async move {
                        let _permit = permit;
                        if refresh_last_seen(
                            &database,
                            &token_hash,
                            now,
                            now - LAST_SEEN_WRITE_INTERVAL,
                        )
                        .await
                        .is_err()
                        {
                            tracing::warn!("session last-seen refresh failed");
                        }
                    });
                }
            } else {
                refresh_last_seen(&database, &token_hash, now, now - LAST_SEEN_WRITE_INTERVAL)
                    .await
                    .map_err(SessionError::Database)?;
            }
        }

        Ok(AuthenticatedSession {
            user,
            csrf_hash: stored.csrf_hash,
            expires_at: stored.expires_at,
        })
    }
}

async fn refresh_last_seen(
    database: &DatabaseConnection,
    token_hash: &str,
    now: OffsetDateTime,
    stale_before: OffsetDateTime,
) -> Result<(), DbErr> {
    session::Entity::update_many()
        .col_expr(session::Column::LastSeenAt, Expr::value(now))
        .filter(session::Column::TokenHash.eq(token_hash))
        .filter(session::Column::LastSeenAt.lte(stale_before))
        .exec(database)
        .await?;
    Ok(())
}

#[derive(Debug)]
pub struct CreatedSession {
    pub cookie_token: SecretString,
    pub csrf_token: SecretString,
    pub expires_at: OffsetDateTime,
}

#[derive(Debug)]
pub struct SessionDetails {
    pub user: User,
    pub csrf_token: SecretString,
    pub expires_at: OffsetDateTime,
}

#[derive(Debug, thiserror::Error)]
pub enum SessionError {
    #[error("session service is unavailable")]
    Unavailable,
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
    pub expires_at: OffsetDateTime,
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

fn derive_csrf_token(cookie_token: &SecretString) -> SecretString {
    let mut bytes = blake3::derive_key(
        CSRF_DERIVATION_CONTEXT,
        cookie_token.expose_secret().as_bytes(),
    );
    let token = URL_SAFE_NO_PAD.encode(bytes);
    bytes.zeroize();
    SecretString::from(token)
}
