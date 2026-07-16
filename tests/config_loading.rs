use std::{collections::HashMap, fs};

use raindrop::config::{BootstrapMode, ConfigArgs, DatabaseKind, EnvSource, load};
use tempfile::tempdir;

#[derive(Default)]
struct MapEnv(HashMap<String, String>);

impl MapEnv {
    fn from<const N: usize>(values: [(&str, &str); N]) -> Self {
        Self(
            values
                .into_iter()
                .map(|(key, value)| (key.to_owned(), value.to_owned()))
                .collect(),
        )
    }
}

impl EnvSource for MapEnv {
    fn get(&self, key: &str) -> Option<String> {
        self.0.get(key).cloned()
    }
}

#[test]
fn no_database_source_enters_setup_mode() {
    let data = tempdir().expect("temporary directory should be created");
    let loaded = load(&ConfigArgs::for_test(data.path()), &MapEnv::default())
        .expect("default configuration should load");

    assert!(matches!(loaded.mode, BootstrapMode::SetupRequired { .. }));
    assert_eq!(loaded.runtime.bind.to_string(), "0.0.0.0:8080");
}

#[test]
fn env_overrides_toml_without_echoing_secret() {
    let data = tempdir().expect("temporary directory should be created");
    fs::write(
        data.path().join("config.toml"),
        "database_url = 'sqlite://file-value.db?mode=rwc'\n",
    )
    .expect("configuration file should be written");
    let env = MapEnv::from([(
        "RAINDROP_DATABASE_URL",
        "postgres://reader:super-secret@db/raindrop",
    )]);

    let loaded =
        load(&ConfigArgs::for_test(data.path()), &env).expect("environment override should load");

    assert_eq!(loaded.runtime.database_kind(), Some(DatabaseKind::Postgres));
    assert!(!format!("{loaded:?}").contains("super-secret"));
}

#[test]
fn invalid_bind_address_names_the_variable() {
    let data = tempdir().expect("temporary directory should be created");
    let env = MapEnv::from([("RAINDROP_BIND", "not-an-address")]);

    let error = load(&ConfigArgs::for_test(data.path()), &env)
        .expect_err("invalid bind address should fail");
    let message = error.to_string();

    assert!(message.contains("RAINDROP_BIND"));
    assert!(!message.contains("database"));
}

#[test]
fn partial_bootstrap_admin_is_rejected_without_password_value() {
    let data = tempdir().expect("temporary directory should be created");
    let env = MapEnv::from([
        (
            "RAINDROP_DATABASE_URL",
            "sqlite://data/raindrop.db?mode=rwc",
        ),
        ("RAINDROP_BOOTSTRAP_ADMIN_PASSWORD", "do-not-print-this"),
    ]);

    let error = load(&ConfigArgs::for_test(data.path()), &env)
        .expect_err("partial bootstrap admin should fail");
    let message = error.to_string();

    assert!(message.contains("RAINDROP_BOOTSTRAP_ADMIN_USERNAME"));
    assert!(!message.contains("do-not-print-this"));
}
