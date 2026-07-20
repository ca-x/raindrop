mod repository;
mod types;

pub use repository::PreferenceRepository;
pub use types::{
    LayoutDensity, LinkOpenMode, Locale, PreferenceError, ReadingColorScheme, ReadingFontFamily,
    ThemeMode, UpdateUserPreferences, UserPreferences,
};
