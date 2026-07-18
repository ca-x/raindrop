use std::fmt;

use sea_orm::DbErr;

use crate::db::entities::category;

const MAX_TITLE_SCALARS: usize = 80;
const MAX_TITLE_BYTES: usize = 200;
const MAX_NORMALIZED_TITLE_BYTES: usize = 320;

#[derive(Clone, Eq, PartialEq)]
pub struct CategoryDto {
    pub category_id: String,
    pub title: String,
    pub position: i64,
}

impl fmt::Debug for CategoryDto {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CategoryDto")
            .field("category_id", &self.category_id)
            .field("title", &"[REDACTED]")
            .field("position", &self.position)
            .finish()
    }
}

impl From<category::Model> for CategoryDto {
    fn from(category: category::Model) -> Self {
        Self {
            category_id: category.id,
            title: category.title,
            position: category.position,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CreateCategory {
    pub title: String,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct UpdateCategory {
    pub title: Option<String>,
    pub position: Option<i64>,
}

#[derive(thiserror::Error)]
pub enum CategoryError {
    #[error("category repository database operation failed")]
    Database(#[from] DbErr),
    #[error("user identifier is invalid")]
    InvalidUserId,
    #[error("category identifier is invalid")]
    InvalidCategoryId,
    #[error("category title is invalid")]
    InvalidTitle,
    #[error("category position is invalid")]
    InvalidPosition,
    #[error("category patch is empty")]
    InvalidPatch,
    #[error("category owner is unavailable")]
    UserUnavailable,
    #[error("category does not exist")]
    NotFound,
    #[error("category title already exists")]
    Conflict,
    #[error("category limit reached")]
    Limit,
    #[error("category repository data is corrupt")]
    CorruptData,
}

impl fmt::Debug for CategoryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Database(_) => "CategoryError::Database([REDACTED])",
            Self::InvalidUserId => "CategoryError::InvalidUserId",
            Self::InvalidCategoryId => "CategoryError::InvalidCategoryId",
            Self::InvalidTitle => "CategoryError::InvalidTitle",
            Self::InvalidPosition => "CategoryError::InvalidPosition",
            Self::InvalidPatch => "CategoryError::InvalidPatch",
            Self::UserUnavailable => "CategoryError::UserUnavailable",
            Self::NotFound => "CategoryError::NotFound",
            Self::Conflict => "CategoryError::Conflict",
            Self::Limit => "CategoryError::Limit",
            Self::CorruptData => "CategoryError::CorruptData",
        })
    }
}

impl PartialEq for CategoryError {
    fn eq(&self, other: &Self) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }
}

impl Eq for CategoryError {}

pub(super) struct NormalizedCategoryTitle {
    pub(super) display: String,
    pub(super) normalized: String,
}

pub(super) fn normalize_title(raw: &str) -> Result<NormalizedCategoryTitle, CategoryError> {
    if raw.chars().any(is_disallowed_control) {
        return Err(CategoryError::InvalidTitle);
    }
    let display = raw.trim();
    if display.is_empty()
        || display.chars().count() > MAX_TITLE_SCALARS
        || display.len() > MAX_TITLE_BYTES
    {
        return Err(CategoryError::InvalidTitle);
    }
    let normalized = display.to_lowercase();
    if normalized.len() > MAX_NORMALIZED_TITLE_BYTES {
        return Err(CategoryError::InvalidTitle);
    }
    Ok(NormalizedCategoryTitle {
        display: display.to_owned(),
        normalized,
    })
}

fn is_disallowed_control(character: char) -> bool {
    matches!(u32::from(character), 0..=31 | 127..=159)
}
