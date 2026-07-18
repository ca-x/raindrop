use std::fmt;

use sea_orm::DbErr;

use crate::db::entities::user_preference;

pub const MIN_READING_FONT_SCALE: i32 = 85;
pub const MAX_READING_FONT_SCALE: i32 = 130;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Locale {
    ZhCn,
    En,
}

impl Locale {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ZhCn => "zh-CN",
            Self::En => "en",
        }
    }

    fn from_storage(value: &str) -> Result<Self, PreferenceError> {
        match value {
            "zh-CN" => Ok(Self::ZhCn),
            "en" => Ok(Self::En),
            _ => Err(PreferenceError::CorruptData),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ThemeMode {
    System,
    Light,
    Dark,
}

impl ThemeMode {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::System => "SYSTEM",
            Self::Light => "LIGHT",
            Self::Dark => "DARK",
        }
    }

    fn from_storage(value: &str) -> Result<Self, PreferenceError> {
        match value {
            "SYSTEM" => Ok(Self::System),
            "LIGHT" => Ok(Self::Light),
            "DARK" => Ok(Self::Dark),
            _ => Err(PreferenceError::CorruptData),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LayoutDensity {
    Compact,
    Balanced,
    Spacious,
}

impl LayoutDensity {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Compact => "COMPACT",
            Self::Balanced => "BALANCED",
            Self::Spacious => "SPACIOUS",
        }
    }

    fn from_storage(value: &str) -> Result<Self, PreferenceError> {
        match value {
            "COMPACT" => Ok(Self::Compact),
            "BALANCED" => Ok(Self::Balanced),
            "SPACIOUS" => Ok(Self::Spacious),
            _ => Err(PreferenceError::CorruptData),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UserPreferences {
    pub locale: Locale,
    pub theme_mode: ThemeMode,
    pub layout_density: LayoutDensity,
    pub reading_font_scale: i32,
}

impl UserPreferences {
    #[must_use]
    pub const fn defaults(locale: Locale) -> Self {
        Self {
            locale,
            theme_mode: ThemeMode::System,
            layout_density: LayoutDensity::Balanced,
            reading_font_scale: 100,
        }
    }

    pub(super) fn from_model(model: &user_preference::Model) -> Result<Self, PreferenceError> {
        if !(MIN_READING_FONT_SCALE..=MAX_READING_FONT_SCALE).contains(&model.reading_font_scale) {
            return Err(PreferenceError::CorruptData);
        }
        Ok(Self {
            locale: Locale::from_storage(&model.locale)?,
            theme_mode: ThemeMode::from_storage(&model.theme_mode)?,
            layout_density: LayoutDensity::from_storage(&model.layout_density)?,
            reading_font_scale: model.reading_font_scale,
        })
    }

    pub(super) fn apply(&self, patch: &UpdateUserPreferences) -> Self {
        Self {
            locale: patch.locale.unwrap_or(self.locale),
            theme_mode: patch.theme_mode.unwrap_or(self.theme_mode),
            layout_density: patch.layout_density.unwrap_or(self.layout_density),
            reading_font_scale: patch.reading_font_scale.unwrap_or(self.reading_font_scale),
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct UpdateUserPreferences {
    pub locale: Option<Locale>,
    pub theme_mode: Option<ThemeMode>,
    pub layout_density: Option<LayoutDensity>,
    pub reading_font_scale: Option<i32>,
}

impl UpdateUserPreferences {
    pub(super) fn validate(&self) -> Result<(), PreferenceError> {
        if self.locale.is_none()
            && self.theme_mode.is_none()
            && self.layout_density.is_none()
            && self.reading_font_scale.is_none()
        {
            return Err(PreferenceError::InvalidPatch);
        }
        if self.reading_font_scale.is_some_and(|scale| {
            !(MIN_READING_FONT_SCALE..=MAX_READING_FONT_SCALE).contains(&scale)
        }) {
            return Err(PreferenceError::InvalidFontScale);
        }
        Ok(())
    }
}

#[derive(thiserror::Error)]
pub enum PreferenceError {
    #[error("preference repository database operation failed")]
    Database(#[from] DbErr),
    #[error("user identifier is invalid")]
    InvalidUserId,
    #[error("preference patch is empty")]
    InvalidPatch,
    #[error("reading font scale is invalid")]
    InvalidFontScale,
    #[error("preference owner is unavailable")]
    UserUnavailable,
    #[error("preference repository data is corrupt")]
    CorruptData,
}

impl fmt::Debug for PreferenceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Database(_) => "PreferenceError::Database([REDACTED])",
            Self::InvalidUserId => "PreferenceError::InvalidUserId",
            Self::InvalidPatch => "PreferenceError::InvalidPatch",
            Self::InvalidFontScale => "PreferenceError::InvalidFontScale",
            Self::UserUnavailable => "PreferenceError::UserUnavailable",
            Self::CorruptData => "PreferenceError::CorruptData",
        })
    }
}

impl PartialEq for PreferenceError {
    fn eq(&self, other: &Self) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }
}

impl Eq for PreferenceError {}
