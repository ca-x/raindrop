use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, ConnectionTrait, DatabaseConnection,
    EntityTrait, IntoActiveModel, PaginatorTrait, QueryFilter, QueryOrder, TransactionTrait,
};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::db::entities::category;

mod queries;
mod types;

pub use types::{CategoryDto, CategoryError, CreateCategory, UpdateCategory};

use queries::{
    find_by_normalized_title, finish_transaction, lock_active_user, next_position, validate_uuid,
};
use types::normalize_title;

const MAX_CATEGORIES_PER_USER: u64 = 250;
const POSITION_STEP: i64 = 1024;

#[derive(Clone)]
pub struct CategoryRepository {
    database: DatabaseConnection,
}

impl CategoryRepository {
    #[must_use]
    pub fn new(database: DatabaseConnection) -> Self {
        Self { database }
    }

    pub async fn list(&self, user_id: &str) -> Result<Vec<CategoryDto>, CategoryError> {
        validate_uuid(user_id).map_err(|()| CategoryError::InvalidUserId)?;
        category::Entity::find()
            .filter(category::Column::UserId.eq(user_id))
            .order_by_asc(category::Column::Position)
            .order_by_asc(category::Column::Id)
            .all(&self.database)
            .await
            .map(|categories| categories.into_iter().map(Into::into).collect())
            .map_err(CategoryError::Database)
    }

    pub async fn create(
        &self,
        user_id: &str,
        input: CreateCategory,
    ) -> Result<CategoryDto, CategoryError> {
        validate_uuid(user_id).map_err(|()| CategoryError::InvalidUserId)?;
        let title = normalize_title(&input.title)?;
        let transaction = self.database.begin().await?;
        let backend = self.database.get_database_backend();
        let result = async {
            lock_active_user(&transaction, backend, user_id).await?;
            if find_by_normalized_title(&transaction, user_id, &title.normalized)
                .await?
                .is_some()
            {
                return Err(CategoryError::Conflict);
            }
            if category::Entity::find()
                .filter(category::Column::UserId.eq(user_id))
                .count(&transaction)
                .await?
                >= MAX_CATEGORIES_PER_USER
            {
                return Err(CategoryError::Limit);
            }
            let position = next_position(&transaction, user_id).await?;
            let now = OffsetDateTime::now_utc();
            category::ActiveModel {
                id: Set(Uuid::new_v4().to_string()),
                user_id: Set(user_id.to_owned()),
                title: Set(title.display),
                normalized_title: Set(title.normalized),
                position: Set(position),
                created_at: Set(now),
                updated_at: Set(now),
            }
            .insert(&transaction)
            .await
            .map(Into::into)
            .map_err(CategoryError::Database)
        }
        .await;
        finish_transaction(transaction, result).await
    }

    pub async fn update(
        &self,
        user_id: &str,
        category_id: &str,
        input: UpdateCategory,
    ) -> Result<CategoryDto, CategoryError> {
        validate_uuid(user_id).map_err(|()| CategoryError::InvalidUserId)?;
        validate_uuid(category_id).map_err(|()| CategoryError::InvalidCategoryId)?;
        if input.title.is_none() && input.position.is_none() {
            return Err(CategoryError::InvalidPatch);
        }
        if input.position.is_some_and(|position| position < 0) {
            return Err(CategoryError::InvalidPosition);
        }
        let title = input.title.as_deref().map(normalize_title).transpose()?;
        let transaction = self.database.begin().await?;
        let backend = self.database.get_database_backend();
        let result = async {
            lock_active_user(&transaction, backend, user_id).await?;
            let stored = category::Entity::find_by_id(category_id)
                .filter(category::Column::UserId.eq(user_id))
                .one(&transaction)
                .await?
                .ok_or(CategoryError::NotFound)?;
            if let Some(title) = &title
                && title.normalized != stored.normalized_title
                && find_by_normalized_title(&transaction, user_id, &title.normalized)
                    .await?
                    .is_some()
            {
                return Err(CategoryError::Conflict);
            }
            let mut active = stored.into_active_model();
            if let Some(title) = title {
                active.title = Set(title.display);
                active.normalized_title = Set(title.normalized);
            }
            if let Some(position) = input.position {
                active.position = Set(position);
            }
            active.updated_at = Set(OffsetDateTime::now_utc());
            active
                .update(&transaction)
                .await
                .map(Into::into)
                .map_err(CategoryError::Database)
        }
        .await;
        finish_transaction(transaction, result).await
    }

    pub async fn delete(&self, user_id: &str, category_id: &str) -> Result<(), CategoryError> {
        validate_uuid(user_id).map_err(|()| CategoryError::InvalidUserId)?;
        validate_uuid(category_id).map_err(|()| CategoryError::InvalidCategoryId)?;
        let transaction = self.database.begin().await?;
        let backend = self.database.get_database_backend();
        let result = async {
            lock_active_user(&transaction, backend, user_id).await?;
            let deleted = category::Entity::delete_many()
                .filter(category::Column::Id.eq(category_id))
                .filter(category::Column::UserId.eq(user_id))
                .exec(&transaction)
                .await?;
            if deleted.rows_affected == 0 {
                return Err(CategoryError::NotFound);
            }
            Ok(())
        }
        .await;
        finish_transaction(transaction, result).await
    }
}
