use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ConnectionTrait, DatabaseBackend, DatabaseConnection,
    DbBackend, EntityTrait, IntoActiveModel, Statement, TransactionTrait,
};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::db::entities::{user, user_preference};

use super::{Locale, PreferenceError, UpdateUserPreferences, UserPreferences};

#[derive(Clone)]
pub struct PreferenceRepository {
    database: DatabaseConnection,
}

impl PreferenceRepository {
    #[must_use]
    pub fn new(database: DatabaseConnection) -> Self {
        Self { database }
    }

    pub async fn get(
        &self,
        user_id: &str,
        default_locale: Locale,
    ) -> Result<UserPreferences, PreferenceError> {
        validate_user_id(user_id)?;
        ensure_active_user(&self.database, user_id).await?;
        let stored = user_preference::Entity::find_by_id(user_id)
            .one(&self.database)
            .await?;
        stored.map_or_else(
            || Ok(UserPreferences::defaults(default_locale)),
            |model| UserPreferences::from_model(&model),
        )
    }

    pub async fn update(
        &self,
        user_id: &str,
        default_locale: Locale,
        patch: UpdateUserPreferences,
    ) -> Result<UserPreferences, PreferenceError> {
        validate_user_id(user_id)?;
        patch.validate()?;
        let transaction = self.database.begin().await?;
        let backend = self.database.get_database_backend();
        let result = async {
            lock_active_user(&transaction, backend, user_id).await?;
            let stored = user_preference::Entity::find_by_id(user_id)
                .one(&transaction)
                .await?;
            let current = match &stored {
                Some(model) => UserPreferences::from_model(model)?,
                None => UserPreferences::defaults(default_locale),
            };
            let updated = current.apply(&patch);
            let now = OffsetDateTime::now_utc();
            let model = match stored {
                Some(stored) => {
                    let mut active = stored.into_active_model();
                    active.locale = Set(updated.locale.as_str().to_owned());
                    active.theme_mode = Set(updated.theme_mode.as_str().to_owned());
                    active.layout_density = Set(updated.layout_density.as_str().to_owned());
                    active.reading_font_scale = Set(updated.reading_font_scale);
                    active.reading_font_family =
                        Set(updated.reading_font_family.as_str().to_owned());
                    active.reading_color_scheme =
                        Set(updated.reading_color_scheme.as_str().to_owned());
                    active.link_open_mode = Set(updated.link_open_mode.as_str().to_owned());
                    active.updated_at = Set(now);
                    active.update(&transaction).await?
                }
                None => {
                    user_preference::ActiveModel {
                        user_id: Set(user_id.to_owned()),
                        locale: Set(updated.locale.as_str().to_owned()),
                        theme_mode: Set(updated.theme_mode.as_str().to_owned()),
                        layout_density: Set(updated.layout_density.as_str().to_owned()),
                        reading_font_scale: Set(updated.reading_font_scale),
                        reading_font_family: Set(updated.reading_font_family.as_str().to_owned()),
                        reading_color_scheme: Set(updated.reading_color_scheme.as_str().to_owned()),
                        link_open_mode: Set(updated.link_open_mode.as_str().to_owned()),
                        created_at: Set(now),
                        updated_at: Set(now),
                    }
                    .insert(&transaction)
                    .await?
                }
            };
            UserPreferences::from_model(&model)
        }
        .await;
        finish_transaction(transaction, result).await
    }
}

async fn ensure_active_user<C>(connection: &C, user_id: &str) -> Result<(), PreferenceError>
where
    C: ConnectionTrait,
{
    let stored = user::Entity::find_by_id(user_id).one(connection).await?;
    if stored.is_some_and(|user| !user.is_disabled) {
        Ok(())
    } else {
        Err(PreferenceError::UserUnavailable)
    }
}

async fn lock_active_user<C>(
    connection: &C,
    backend: DbBackend,
    user_id: &str,
) -> Result<(), PreferenceError>
where
    C: ConnectionTrait,
{
    match backend {
        DatabaseBackend::Sqlite => {
            let result = connection
                .execute(Statement::from_sql_and_values(
                    backend,
                    "UPDATE users SET is_disabled = is_disabled WHERE id = ? AND is_disabled = FALSE",
                    [user_id.into()],
                ))
                .await?;
            if result.rows_affected() != 1 {
                return Err(PreferenceError::UserUnavailable);
            }
        }
        DatabaseBackend::Postgres | DatabaseBackend::MySql => {
            let sql = if backend == DatabaseBackend::Postgres {
                "SELECT is_disabled FROM users WHERE id = $1 FOR UPDATE"
            } else {
                "SELECT is_disabled FROM users WHERE id = ? FOR UPDATE"
            };
            let row = connection
                .query_one(Statement::from_sql_and_values(
                    backend,
                    sql,
                    [user_id.into()],
                ))
                .await?
                .ok_or(PreferenceError::UserUnavailable)?;
            let is_disabled: bool = row
                .try_get("", "is_disabled")
                .map_err(|_| PreferenceError::CorruptData)?;
            if is_disabled {
                return Err(PreferenceError::UserUnavailable);
            }
        }
    }
    Ok(())
}

async fn finish_transaction<T>(
    transaction: sea_orm::DatabaseTransaction,
    result: Result<T, PreferenceError>,
) -> Result<T, PreferenceError> {
    match result {
        Ok(value) => {
            transaction.commit().await?;
            Ok(value)
        }
        Err(error) => {
            transaction.rollback().await?;
            Err(error)
        }
    }
}

fn validate_user_id(value: &str) -> Result<(), PreferenceError> {
    Uuid::parse_str(value)
        .map(|_| ())
        .map_err(|_| PreferenceError::InvalidUserId)
}
