use nsh_core::platform::Appearance;
use sea_orm::{ConnectionTrait, DbBackend, Statement};

use crate::{api::ApiError, local::Local};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum Language {
    #[default]
    Chinese,
    English,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum Theme {
    #[default]
    System,
    Light,
    Dark,
}

impl Theme {
    pub(crate) fn appearance(self) -> Appearance {
        match self {
            Self::System => Appearance::System,
            Self::Light => Appearance::Light,
            Self::Dark => Appearance::Dark,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct Preferences {
    pub(crate) language: Language,
    pub(crate) theme: Theme,
}

impl Preferences {
    pub(crate) fn load(local: &Local) -> Result<Self, ApiError> {
        local.block_on(async {
            local.database.execute_unprepared("CREATE TABLE IF NOT EXISTS desktop_preferences (id INTEGER PRIMARY KEY CHECK(id = 1), schema_version INTEGER NOT NULL, language TEXT NOT NULL, theme TEXT NOT NULL)")
                .await.map_err(|_| ApiError::LocalData)?;
            let row = local.database.query_one(Statement::from_string(DbBackend::Sqlite,
                "SELECT schema_version, language, theme FROM desktop_preferences WHERE id = 1"))
                .await.map_err(|_| ApiError::LocalData)?;
            let Some(row) = row else { return Ok(Self::default()); };
            let version: i32 = row.try_get("", "schema_version").map_err(|_| ApiError::LocalData)?;
            let language: String = row.try_get("", "language").map_err(|_| ApiError::LocalData)?;
            let theme: String = row.try_get("", "theme").map_err(|_| ApiError::LocalData)?;
            if version != 1 { return Err(ApiError::LocalData); }
            Ok(Self {
                language: match language.as_str() {
                    "zh-CN" => Language::Chinese,
                    "en" => Language::English,
                    _ => return Err(ApiError::LocalData),
                },
                theme: match theme.as_str() {
                    "system" => Theme::System,
                    "light" => Theme::Light,
                    "dark" => Theme::Dark,
                    _ => return Err(ApiError::LocalData),
                },
            })
        })
    }

    pub(crate) fn save(self, local: &Local) -> Result<(), ApiError> {
        let language = match self.language {
            Language::Chinese => "zh-CN",
            Language::English => "en",
        };
        let theme = match self.theme {
            Theme::System => "system",
            Theme::Light => "light",
            Theme::Dark => "dark",
        };
        local.block_on(local.database.execute(Statement::from_sql_and_values(DbBackend::Sqlite,
            "INSERT INTO desktop_preferences(id, schema_version, language, theme) VALUES(1, 1, ?, ?) ON CONFLICT(id) DO UPDATE SET language = excluded.language, theme = excluded.theme WHERE desktop_preferences.schema_version = 1",
            [language.into(), theme.into()])))
            .map_err(|_| ApiError::LocalData)
            .and_then(|result| if result.rows_affected() == 1 { Ok(()) } else { Err(ApiError::LocalData) })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::Api;

    #[test]
    fn preferences_survive_restart_and_are_independent_of_sync_records() {
        let directory = tempfile::tempdir().unwrap();
        let api = Api::isolated(directory.path().to_path_buf()).unwrap();
        let local = api.local_core().unwrap();
        assert_eq!(Preferences::load(&local).unwrap(), Preferences::default());
        let preferences = Preferences {
            language: Language::English,
            theme: Theme::Dark,
        };
        preferences.save(&local).unwrap();
        crate::sync::profiles(&api).unwrap();
        local
            .block_on(local.database.execute_unprepared(
                "INSERT INTO desktop_sync_state(id, document) VALUES(1, 'broken')",
            ))
            .unwrap();
        assert!(crate::sync::profiles(&api).is_err());
        assert_eq!(Preferences::load(&local).unwrap(), preferences);
        drop(local);
        drop(api);
        let started = std::time::Instant::now();
        let restarted = loop {
            match Api::isolated(directory.path().to_path_buf()) {
                Ok(api) => break api,
                Err(ApiError::LocalBusy)
                    if started.elapsed() < std::time::Duration::from_secs(10) =>
                {
                    std::thread::sleep(std::time::Duration::from_millis(10))
                }
                Err(error) => panic!("restart failed: {error}"),
            }
        };
        assert_eq!(
            Preferences::load(&restarted.local_core().unwrap()).unwrap(),
            preferences
        );
        assert_eq!(Theme::System.appearance(), Appearance::System);
        assert_eq!(Theme::Light.appearance(), Appearance::Light);
        assert_eq!(Theme::Dark.appearance(), Appearance::Dark);
    }

    #[test]
    fn unknown_preferences_version_is_not_overwritten() {
        let directory = tempfile::tempdir().unwrap();
        let api = Api::isolated(directory.path().to_path_buf()).unwrap();
        let local = api.local_core().unwrap();
        Preferences::load(&local).unwrap();
        local.block_on(local.database.execute_unprepared("INSERT INTO desktop_preferences(id, schema_version, language, theme) VALUES(1, 2, 'future', 'future')")).unwrap();
        assert_eq!(Preferences::load(&local), Err(ApiError::LocalData));
        assert_eq!(
            Preferences::default().save(&local),
            Err(ApiError::LocalData)
        );
        let row = local
            .block_on(local.database.query_one(Statement::from_string(
                DbBackend::Sqlite,
                "SELECT schema_version FROM desktop_preferences WHERE id = 1",
            )))
            .unwrap()
            .unwrap();
        assert_eq!(row.try_get::<i32>("", "schema_version").unwrap(), 2);
    }

    #[test]
    fn failed_preferences_save_keeps_previous_complete_values() {
        let directory = tempfile::tempdir().unwrap();
        let api = Api::isolated(directory.path().to_path_buf()).unwrap();
        let local = api.local_core().unwrap();
        Preferences::load(&local).unwrap();
        Preferences::default().save(&local).unwrap();
        local.block_on(local.database.execute_unprepared("CREATE TRIGGER fail_preferences BEFORE UPDATE ON desktop_preferences BEGIN SELECT RAISE(ABORT, 'fixture'); END")).unwrap();
        assert_eq!(
            Preferences {
                language: Language::English,
                theme: Theme::Dark
            }
            .save(&local),
            Err(ApiError::LocalData)
        );
        assert_eq!(Preferences::load(&local).unwrap(), Preferences::default());
    }
}
