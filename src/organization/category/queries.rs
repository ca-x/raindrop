use sea_orm::{
    ColumnTrait, ConnectionTrait, DatabaseBackend, DbBackend, DbErr, EntityTrait, QueryFilter,
    QueryOrder, Statement,
};
use uuid::Uuid;

use crate::db::entities::category;

use super::{CategoryError, POSITION_STEP};

pub(super) async fn find_by_normalized_title<C>(
    connection: &C,
    user_id: &str,
    normalized_title: &str,
) -> Result<Option<category::Model>, DbErr>
where
    C: ConnectionTrait,
{
    category::Entity::find()
        .filter(category::Column::UserId.eq(user_id))
        .filter(category::Column::NormalizedTitle.eq(normalized_title))
        .one(connection)
        .await
}

pub(super) async fn next_position<C>(connection: &C, user_id: &str) -> Result<i64, CategoryError>
where
    C: ConnectionTrait,
{
    let last = category::Entity::find()
        .filter(category::Column::UserId.eq(user_id))
        .order_by_desc(category::Column::Position)
        .order_by_desc(category::Column::Id)
        .one(connection)
        .await?;
    match last {
        Some(category) if category.position >= 0 => category
            .position
            .checked_add(POSITION_STEP)
            .ok_or(CategoryError::CorruptData),
        Some(_) => Err(CategoryError::CorruptData),
        None => Ok(POSITION_STEP),
    }
}

pub(super) async fn lock_active_user<C>(
    connection: &C,
    backend: DbBackend,
    user_id: &str,
) -> Result<(), CategoryError>
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
                return Err(CategoryError::UserUnavailable);
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
                .ok_or(CategoryError::UserUnavailable)?;
            let is_disabled: bool = row
                .try_get("", "is_disabled")
                .map_err(|_| CategoryError::CorruptData)?;
            if is_disabled {
                return Err(CategoryError::UserUnavailable);
            }
        }
    }
    Ok(())
}

pub(super) async fn finish_transaction<T>(
    transaction: sea_orm::DatabaseTransaction,
    result: Result<T, CategoryError>,
) -> Result<T, CategoryError> {
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

pub(super) fn validate_uuid(value: &str) -> Result<(), ()> {
    Uuid::parse_str(value).map(|_| ()).map_err(|_| ())
}
