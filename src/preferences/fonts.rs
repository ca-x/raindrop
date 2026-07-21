use std::{fmt, io};

use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, ConnectionTrait, DatabaseConnection,
    EntityTrait, IntoActiveModel, ModelTrait, PaginatorTrait, QueryFilter, QueryOrder, QuerySelect,
    TransactionTrait,
};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::db::entities::{user_font, user_preference};

use super::repository::{
    ensure_active_user, finish_transaction, lock_active_user, validate_user_id,
};

pub const MAX_USER_FONT_BYTES: usize = 5 * 1024 * 1024;
pub const MAX_USER_FONTS: u64 = 8;
const MAX_DECOMPRESSED_FONT_BYTES: usize = 64 * 1024 * 1024;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UserFont {
    pub id: String,
    pub display_name: String,
    pub byte_size: i32,
    pub content_hash: String,
    pub created_at: OffsetDateTime,
}

#[derive(Clone, Eq, PartialEq)]
pub struct UserFontFile {
    pub content_hash: String,
    pub bytes: Vec<u8>,
}

impl fmt::Debug for UserFontFile {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("UserFontFile")
            .field("content_hash", &self.content_hash)
            .field("bytes", &format_args!("[{} bytes]", self.bytes.len()))
            .finish()
    }
}

#[derive(Clone)]
pub struct UserFontRepository {
    database: DatabaseConnection,
}

impl UserFontRepository {
    #[must_use]
    pub fn new(database: DatabaseConnection) -> Self {
        Self { database }
    }

    pub async fn list(&self, user_id: &str) -> Result<Vec<UserFont>, UserFontError> {
        validate_user_id(user_id).map_err(|_| UserFontError::InvalidUserId)?;
        ensure_active_user(&self.database, user_id)
            .await
            .map_err(map_preference_error)?;
        user_font::Entity::find()
            .select_only()
            .columns([
                user_font::Column::Id,
                user_font::Column::DisplayName,
                user_font::Column::ByteSize,
                user_font::Column::ContentHash,
                user_font::Column::CreatedAt,
            ])
            .filter(user_font::Column::UserId.eq(user_id))
            .order_by_asc(user_font::Column::CreatedAt)
            .order_by_asc(user_font::Column::Id)
            .into_tuple::<(String, String, i32, String, OffsetDateTime)>()
            .all(&self.database)
            .await
            .map(|fonts| {
                fonts
                    .into_iter()
                    .map(
                        |(id, display_name, byte_size, content_hash, created_at)| UserFont {
                            id,
                            display_name,
                            byte_size,
                            content_hash,
                            created_at,
                        },
                    )
                    .collect()
            })
            .map_err(UserFontError::Database)
    }

    pub async fn create(
        &self,
        user_id: &str,
        display_name: &str,
        bytes: &[u8],
    ) -> Result<UserFont, UserFontError> {
        validate_user_id(user_id).map_err(|_| UserFontError::InvalidUserId)?;
        let (display_name, normalized_name) = normalize_display_name(display_name)?;
        validate_woff2(bytes)?;
        let content_hash = blake3::hash(bytes).to_hex().to_string();
        let transaction = self.database.begin().await?;
        let backend = self.database.get_database_backend();
        let result = async {
            lock_active_user(&transaction, backend, user_id)
                .await
                .map_err(map_preference_error)?;
            let count = user_font::Entity::find()
                .filter(user_font::Column::UserId.eq(user_id))
                .count(&transaction)
                .await?;
            if count >= MAX_USER_FONTS {
                return Err(UserFontError::QuotaExceeded);
            }
            let duplicate = user_font::Entity::find()
                .filter(user_font::Column::UserId.eq(user_id))
                .filter(
                    user_font::Column::ContentHash
                        .eq(&content_hash)
                        .or(user_font::Column::NormalizedName.eq(&normalized_name)),
                )
                .one(&transaction)
                .await?;
            if duplicate.is_some() {
                return Err(UserFontError::Duplicate);
            }
            let now = OffsetDateTime::now_utc();
            let model = user_font::ActiveModel {
                id: Set(Uuid::new_v4().to_string()),
                user_id: Set(user_id.to_owned()),
                display_name: Set(display_name),
                normalized_name: Set(normalized_name),
                content_hash: Set(content_hash),
                font_bytes: Set(bytes.to_vec()),
                byte_size: Set(i32::try_from(bytes.len()).expect("font size is bounded")),
                created_at: Set(now),
                updated_at: Set(now),
            }
            .insert(&transaction)
            .await?;
            Ok(UserFont::from(model))
        }
        .await;
        finish_transaction(transaction, result).await
    }

    pub async fn file(
        &self,
        user_id: &str,
        font_id: &str,
    ) -> Result<Option<UserFontFile>, UserFontError> {
        validate_user_id(user_id).map_err(|_| UserFontError::InvalidUserId)?;
        validate_font_id(font_id)?;
        ensure_active_user(&self.database, user_id)
            .await
            .map_err(map_preference_error)?;
        user_font::Entity::find_by_id(font_id)
            .filter(user_font::Column::UserId.eq(user_id))
            .one(&self.database)
            .await?
            .map(|font| {
                if font.font_bytes.len() != usize::try_from(font.byte_size).unwrap_or_default()
                    || validate_woff2_header(&font.font_bytes).is_err()
                    || blake3::hash(&font.font_bytes).to_hex().as_str() != font.content_hash
                {
                    return Err(UserFontError::CorruptData);
                }
                Ok(UserFontFile {
                    content_hash: font.content_hash,
                    bytes: font.font_bytes,
                })
            })
            .transpose()
    }

    pub async fn delete(&self, user_id: &str, font_id: &str) -> Result<bool, UserFontError> {
        validate_user_id(user_id).map_err(|_| UserFontError::InvalidUserId)?;
        validate_font_id(font_id)?;
        let transaction = self.database.begin().await?;
        let backend = self.database.get_database_backend();
        let result = async {
            lock_active_user(&transaction, backend, user_id)
                .await
                .map_err(map_preference_error)?;
            let stored = user_font::Entity::find_by_id(font_id)
                .filter(user_font::Column::UserId.eq(user_id))
                .one(&transaction)
                .await?;
            let Some(stored) = stored else {
                return Ok(false);
            };
            if let Some(preferences) = user_preference::Entity::find_by_id(user_id)
                .one(&transaction)
                .await?
                && preferences.reading_custom_font_id.as_deref() == Some(font_id)
            {
                let mut active = preferences.into_active_model();
                active.reading_custom_font_id = Set(None);
                active.updated_at = Set(OffsetDateTime::now_utc());
                active.update(&transaction).await?;
            }
            stored.delete(&transaction).await?;
            Ok(true)
        }
        .await;
        finish_transaction(transaction, result).await
    }
}

impl From<user_font::Model> for UserFont {
    fn from(value: user_font::Model) -> Self {
        Self {
            id: value.id,
            display_name: value.display_name,
            byte_size: value.byte_size,
            content_hash: value.content_hash,
            created_at: value.created_at,
        }
    }
}

fn normalize_display_name(value: &str) -> Result<(String, String), UserFontError> {
    let display_name = value.trim();
    if display_name.is_empty()
        || display_name.chars().count() > 80
        || display_name.chars().any(char::is_control)
    {
        return Err(UserFontError::InvalidDisplayName);
    }
    let normalized_name = display_name.to_lowercase();
    if normalized_name.chars().count() > 80 {
        return Err(UserFontError::InvalidDisplayName);
    }
    Ok((display_name.to_owned(), normalized_name))
}

fn validate_woff2(bytes: &[u8]) -> Result<(), UserFontError> {
    if bytes.is_empty() || bytes.len() > MAX_USER_FONT_BYTES {
        return Err(UserFontError::InvalidSize);
    }
    validate_woff2_header(bytes)?;
    let decoded = wuff::decompress_woff2_with_custom_brotli(bytes, &mut decompress_brotli)
        .map_err(|_| UserFontError::InvalidFormat)?;
    if decoded.is_empty()
        || decoded.len() > MAX_DECOMPRESSED_FONT_BYTES
        || !matches!(
            decoded.get(..4),
            Some(b"\0\x01\0\0" | b"OTTO" | b"ttcf" | b"true")
        )
    {
        return Err(UserFontError::InvalidFormat);
    }
    Ok(())
}

fn validate_woff2_header(bytes: &[u8]) -> Result<(), UserFontError> {
    if bytes.len() < 48 || !bytes.starts_with(b"wOF2") {
        return Err(UserFontError::InvalidFormat);
    }
    let declared_length = read_u32(bytes, 8)? as usize;
    let table_count = read_u16(bytes, 12)?;
    let total_sfnt_size = read_u32(bytes, 16)? as usize;
    if declared_length != bytes.len()
        || table_count == 0
        || table_count > 256
        || total_sfnt_size == 0
        || total_sfnt_size > MAX_DECOMPRESSED_FONT_BYTES
    {
        return Err(UserFontError::InvalidFormat);
    }
    Ok(())
}

fn read_u16(bytes: &[u8], offset: usize) -> Result<u16, UserFontError> {
    bytes
        .get(offset..offset + 2)
        .and_then(|value| value.try_into().ok())
        .map(u16::from_be_bytes)
        .ok_or(UserFontError::InvalidFormat)
}

fn read_u32(bytes: &[u8], offset: usize) -> Result<u32, UserFontError> {
    bytes
        .get(offset..offset + 4)
        .and_then(|value| value.try_into().ok())
        .map(u32::from_be_bytes)
        .ok_or(UserFontError::InvalidFormat)
}

fn decompress_brotli(
    input: &[u8],
    expected_size: usize,
) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    if expected_size == 0 || expected_size > MAX_DECOMPRESSED_FONT_BYTES {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "font expands past limit").into());
    }
    let mut reader = io::Cursor::new(input);
    let mut output = BoundedFontBuffer::new(expected_size);
    brotli_decompressor::BrotliDecompress(&mut reader, &mut output)?;
    if output.bytes.len() != expected_size {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "font size mismatch").into());
    }
    Ok(output.bytes)
}

struct BoundedFontBuffer {
    bytes: Vec<u8>,
    limit: usize,
}

impl BoundedFontBuffer {
    fn new(limit: usize) -> Self {
        Self {
            bytes: Vec::with_capacity(limit),
            limit,
        }
    }
}

impl io::Write for BoundedFontBuffer {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if self.bytes.len().saturating_add(bytes.len()) > self.limit {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "font expands past limit",
            ));
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn validate_font_id(value: &str) -> Result<(), UserFontError> {
    let parsed = Uuid::parse_str(value).map_err(|_| UserFontError::InvalidFontId)?;
    if parsed.to_string() != value {
        return Err(UserFontError::InvalidFontId);
    }
    Ok(())
}

fn map_preference_error(error: super::PreferenceError) -> UserFontError {
    match error {
        super::PreferenceError::Database(error) => UserFontError::Database(error),
        super::PreferenceError::UserUnavailable => UserFontError::UserUnavailable,
        _ => UserFontError::CorruptData,
    }
}

#[derive(thiserror::Error)]
pub enum UserFontError {
    #[error("font repository database operation failed")]
    Database(#[from] sea_orm::DbErr),
    #[error("user identifier is invalid")]
    InvalidUserId,
    #[error("font identifier is invalid")]
    InvalidFontId,
    #[error("font display name is invalid")]
    InvalidDisplayName,
    #[error("font size is invalid")]
    InvalidSize,
    #[error("font format is invalid")]
    InvalidFormat,
    #[error("font already exists")]
    Duplicate,
    #[error("font quota exceeded")]
    QuotaExceeded,
    #[error("font owner is unavailable")]
    UserUnavailable,
    #[error("font repository data is corrupt")]
    CorruptData,
}

impl fmt::Debug for UserFontError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Database(_) => "UserFontError::Database([REDACTED])",
            Self::InvalidUserId => "UserFontError::InvalidUserId",
            Self::InvalidFontId => "UserFontError::InvalidFontId",
            Self::InvalidDisplayName => "UserFontError::InvalidDisplayName",
            Self::InvalidSize => "UserFontError::InvalidSize",
            Self::InvalidFormat => "UserFontError::InvalidFormat",
            Self::Duplicate => "UserFontError::Duplicate",
            Self::QuotaExceeded => "UserFontError::QuotaExceeded",
            Self::UserUnavailable => "UserFontError::UserUnavailable",
            Self::CorruptData => "UserFontError::CorruptData",
        })
    }
}
