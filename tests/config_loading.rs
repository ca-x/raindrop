use std::{collections::HashMap, fs, time::Duration};

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use raindrop::config::{BootstrapMode, ConfigArgs, DatabaseKind, EnvSource, load};
use secrecy::ExposeSecret;
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
fn feed_orphan_retention_defaults_to_thirty_days() {
    let data = tempdir().expect("temporary directory should be created");
    let loaded = load(&ConfigArgs::for_test(data.path()), &MapEnv::default())
        .expect("default configuration should load");

    assert_eq!(
        loaded.runtime.feed_retention().orphan_grace,
        Some(Duration::from_secs(30 * 86_400))
    );
}

#[test]
fn feed_orphan_retention_environment_overrides_toml_and_zero_disables() {
    let data = tempdir().expect("temporary directory should be created");
    fs::write(
        data.path().join("config.toml"),
        "feed_orphan_retention_days = 90\n",
    )
    .expect("configuration file should be written");
    let env = MapEnv::from([("RAINDROP_FEED_ORPHAN_RETENTION_DAYS", "0")]);

    let loaded = load(&ConfigArgs::for_test(data.path()), &env)
        .expect("environment retention override should load");

    assert_eq!(loaded.runtime.feed_retention().orphan_grace, None);
}

#[test]
fn feed_orphan_retention_accepts_the_maximum() {
    let data = tempdir().expect("temporary directory should be created");
    let env = MapEnv::from([("RAINDROP_FEED_ORPHAN_RETENTION_DAYS", "3650")]);

    let loaded =
        load(&ConfigArgs::for_test(data.path()), &env).expect("maximum retention should load");

    assert_eq!(
        loaded.runtime.feed_retention().orphan_grace,
        Some(Duration::from_secs(3650 * 86_400))
    );
}

#[test]
fn feed_orphan_retention_rejects_invalid_environment_values() {
    for value in ["3651", "not-a-number"] {
        let data = tempdir().expect("temporary directory should be created");
        let env = MapEnv::from([("RAINDROP_FEED_ORPHAN_RETENTION_DAYS", value)]);

        let error = load(&ConfigArgs::for_test(data.path()), &env)
            .expect_err("invalid retention should fail");

        assert!(
            error
                .to_string()
                .contains("RAINDROP_FEED_ORPHAN_RETENTION_DAYS")
        );
    }
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

#[test]
fn bootstrap_admin_accepts_a_short_non_empty_password() {
    let data = tempdir().expect("temporary directory should be created");
    let env = MapEnv::from([
        (
            "RAINDROP_DATABASE_URL",
            "sqlite://data/raindrop.db?mode=rwc",
        ),
        ("RAINDROP_BOOTSTRAP_ADMIN_USERNAME", "Reader"),
        ("RAINDROP_BOOTSTRAP_ADMIN_PASSWORD", "a"),
    ]);

    let loaded = load(&ConfigArgs::for_test(data.path()), &env)
        .expect("a short non-empty bootstrap password should load");
    assert_eq!(
        loaded
            .runtime
            .bootstrap_admin
            .expect("bootstrap administrator should be present")
            .password
            .expose_secret(),
        "a"
    );
}

#[test]
fn bootstrap_admin_rejects_an_empty_password() {
    let data = tempdir().expect("temporary directory should be created");
    let env = MapEnv::from([
        (
            "RAINDROP_DATABASE_URL",
            "sqlite://data/raindrop.db?mode=rwc",
        ),
        ("RAINDROP_BOOTSTRAP_ADMIN_USERNAME", "Reader"),
        ("RAINDROP_BOOTSTRAP_ADMIN_PASSWORD", ""),
    ]);

    let error = load(&ConfigArgs::for_test(data.path()), &env)
        .expect_err("an empty bootstrap password should fail");
    assert!(
        error
            .to_string()
            .contains("RAINDROP_BOOTSTRAP_ADMIN_PASSWORD")
    );
}

#[test]
fn bootstrap_admin_merges_each_environment_field_over_toml() {
    let data = tempdir().expect("temporary directory should be created");
    fs::write(
        data.path().join("config.toml"),
        r#"
database_url = "sqlite://data/raindrop.db?mode=rwc"
[bootstrap_admin]
username = "FileReader"
password = "file password value"
email = "file@example.com"
"#,
    )
    .expect("configuration file should be written");
    let env = MapEnv::from([("RAINDROP_BOOTSTRAP_ADMIN_USERNAME", "EnvReader")]);

    let loaded = load(&ConfigArgs::for_test(data.path()), &env)
        .expect("merged bootstrap administrator should load");
    let admin = loaded
        .runtime
        .bootstrap_admin
        .expect("bootstrap administrator should be complete");

    assert_eq!(admin.username, "EnvReader");
    assert_eq!(admin.password.expose_secret(), "file password value");
    assert_eq!(admin.email.as_deref(), Some("file@example.com"));
}

#[test]
fn malformed_toml_discards_secret_source_input_from_the_error_chain() {
    let data = tempdir().expect("temporary directory should be created");
    let sentinels = [
        "database-sentinel-7a3e",
        "session-sentinel-6c4f",
        "password-sentinel-9b2d",
    ];
    fs::write(
        data.path().join("config.toml"),
        format!(
            "database_url = \"{}\"\nsession_secret = \"{}\"\n[bootstrap_admin]\npassword = \"{}\n",
            sentinels[0], sentinels[1], sentinels[2]
        ),
    )
    .expect("configuration file should be written");

    let error = load(&ConfigArgs::for_test(data.path()), &MapEnv::default())
        .expect_err("malformed configuration should fail");
    let chain = error_chain(&error);

    assert!(chain.contains("failed to parse configuration file"));
    for sentinel in sentinels {
        assert!(!chain.contains(sentinel), "error disclosed {sentinel}");
    }
}

#[test]
fn provider_secret_key_environment_replaces_toml() {
    let data = tempdir().expect("temporary directory should be created");
    let file_entry = provider_key_entry("file", 1);
    fs::write(
        data.path().join("config.toml"),
        format!("provider_secret_keys = ['{file_entry}']\n"),
    )
    .expect("configuration file should be written");
    let env_entry = provider_key_entry("environment", 2);
    let env = MapEnv::from([("RAINDROP_PROVIDER_SECRET_KEYS", env_entry.as_str())]);

    let mut loaded = load(&ConfigArgs::for_test(data.path()), &env)
        .expect("environment provider key should replace the file list");
    let entries = loaded.runtime.take_provider_secret_keys();

    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].expose_secret(), env_entry);
    assert!(!format!("{loaded:?}").contains(entries[0].expose_secret()));
    assert!(loaded.runtime.provider_secret_keys().is_empty());
}

#[test]
fn provider_secret_key_entries_are_validated_without_echo() {
    let duplicate_id = format!(
        "primary:{},primary:{}",
        URL_SAFE_NO_PAD.encode([1_u8; 32]),
        URL_SAFE_NO_PAD.encode([2_u8; 32])
    );
    let duplicate_key = format!(
        "primary:{0},previous:{0}",
        URL_SAFE_NO_PAD.encode([3_u8; 32])
    );
    for invalid in [
        "secret-sentinel-invalid-entry".to_owned(),
        "-invalid:secret-sentinel-invalid-id".to_owned(),
        "primary:secret-sentinel-invalid-base64".to_owned(),
        format!("primary:{}", URL_SAFE_NO_PAD.encode([4_u8; 31])),
        format!("primary:{}=", URL_SAFE_NO_PAD.encode([5_u8; 32])),
        duplicate_id,
        duplicate_key,
    ] {
        let data = tempdir().expect("temporary directory should be created");
        let env = MapEnv::from([("RAINDROP_PROVIDER_SECRET_KEYS", invalid.as_str())]);

        let error = load(&ConfigArgs::for_test(data.path()), &env)
            .expect_err("invalid provider key configuration should fail");
        let chain = error_chain(&error);

        assert!(chain.contains("RAINDROP_PROVIDER_SECRET_KEYS"));
        assert!(!chain.contains(&invalid));
        assert!(!chain.contains("secret-sentinel"));
    }
}

#[test]
fn provider_secret_key_configuration_is_optional() {
    let data = tempdir().expect("temporary directory should be created");
    let loaded = load(&ConfigArgs::for_test(data.path()), &MapEnv::default())
        .expect("configuration without provider keys should load");

    assert!(loaded.runtime.provider_secret_keys().is_empty());
}

fn provider_key_entry(id: &str, byte: u8) -> String {
    format!("{id}:{}", URL_SAFE_NO_PAD.encode([byte; 32]))
}

fn error_chain(error: &dyn std::error::Error) -> String {
    let mut messages = vec![error.to_string()];
    let mut source = error.source();
    while let Some(error) = source {
        messages.push(error.to_string());
        source = error.source();
    }
    messages.join(": ")
}
