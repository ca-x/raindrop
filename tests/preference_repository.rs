#[allow(dead_code)]
mod support;

use raindrop::{
    db::{DatabaseConfig, connect, entities::user_preference, migrate},
    preferences::{
        LayoutDensity, Locale, PreferenceError, PreferenceRepository, ThemeMode,
        UpdateUserPreferences,
    },
};
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ConnectionTrait, DatabaseConnection, EntityTrait,
    TransactionTrait,
};
use secrecy::SecretString;
use support::database::{USER_A_ID, USER_B_ID, insert_user};
use tempfile::TempDir;
use time::OffsetDateTime;

#[tokio::test]
async fn missing_rows_use_the_requested_locale_and_stable_defaults() {
    let fixture = RepositoryFixture::new().await;

    let zh = fixture
        .repository
        .get(USER_A_ID, Locale::ZhCn)
        .await
        .expect("missing Chinese preferences should resolve");
    assert_eq!(zh.locale, Locale::ZhCn);
    assert_eq!(zh.theme_mode, ThemeMode::System);
    assert_eq!(zh.layout_density, LayoutDensity::Balanced);
    assert_eq!(zh.reading_font_scale, 100);
    assert!(
        user_preference::Entity::find_by_id(USER_A_ID)
            .one(&fixture.database)
            .await
            .expect("preference row should query")
            .is_none(),
        "GET defaults must not create storage"
    );

    let en = fixture
        .repository
        .get(USER_B_ID, Locale::En)
        .await
        .expect("missing English preferences should resolve");
    assert_eq!(en.locale, Locale::En);
}

#[tokio::test]
async fn partial_updates_preserve_every_untouched_field() {
    let fixture = RepositoryFixture::new().await;

    let theme = fixture
        .repository
        .update(
            USER_A_ID,
            Locale::En,
            UpdateUserPreferences {
                theme_mode: Some(ThemeMode::Dark),
                ..Default::default()
            },
        )
        .await
        .expect("theme should update");
    assert_eq!(theme.locale, Locale::En);
    assert_eq!(theme.theme_mode, ThemeMode::Dark);
    assert_eq!(theme.layout_density, LayoutDensity::Balanced);
    assert_eq!(theme.reading_font_scale, 100);

    let locale = fixture
        .repository
        .update(
            USER_A_ID,
            Locale::En,
            UpdateUserPreferences {
                locale: Some(Locale::ZhCn),
                ..Default::default()
            },
        )
        .await
        .expect("locale should update");
    assert_eq!(locale.locale, Locale::ZhCn);
    assert_eq!(locale.theme_mode, ThemeMode::Dark);

    let complete = fixture
        .repository
        .update(
            USER_A_ID,
            Locale::En,
            UpdateUserPreferences {
                locale: Some(Locale::En),
                theme_mode: Some(ThemeMode::Light),
                layout_density: Some(LayoutDensity::Spacious),
                reading_font_scale: Some(120),
            },
        )
        .await
        .expect("complete preference patch should update");
    assert_eq!(complete.locale, Locale::En);
    assert_eq!(complete.theme_mode, ThemeMode::Light);
    assert_eq!(complete.layout_density, LayoutDensity::Spacious);
    assert_eq!(complete.reading_font_scale, 120);
}

#[tokio::test]
async fn validation_rejects_empty_out_of_range_and_invalid_user_inputs() {
    let fixture = RepositoryFixture::new().await;

    assert_eq!(
        fixture
            .repository
            .update(USER_A_ID, Locale::En, UpdateUserPreferences::default())
            .await
            .expect_err("empty preference patch should fail"),
        PreferenceError::InvalidPatch
    );
    for scale in [84, 131] {
        assert_eq!(
            fixture
                .repository
                .update(
                    USER_A_ID,
                    Locale::En,
                    UpdateUserPreferences {
                        reading_font_scale: Some(scale),
                        ..Default::default()
                    },
                )
                .await
                .expect_err("out-of-range preference scale should fail"),
            PreferenceError::InvalidFontScale
        );
    }
    assert_eq!(
        fixture
            .repository
            .get("not-a-user", Locale::En)
            .await
            .expect_err("invalid user ID should fail"),
        PreferenceError::InvalidUserId
    );
}

#[tokio::test]
async fn users_are_isolated_and_owner_deletion_cascades() {
    let fixture = RepositoryFixture::new().await;
    fixture
        .repository
        .update(
            USER_A_ID,
            Locale::En,
            UpdateUserPreferences {
                theme_mode: Some(ThemeMode::Dark),
                ..Default::default()
            },
        )
        .await
        .expect("user A preferences should update");

    let user_b = fixture
        .repository
        .get(USER_B_ID, Locale::ZhCn)
        .await
        .expect("user B defaults should remain isolated");
    assert_eq!(user_b.locale, Locale::ZhCn);
    assert_eq!(user_b.theme_mode, ThemeMode::System);

    raindrop::db::entities::user::Entity::delete_by_id(USER_A_ID)
        .exec(&fixture.database)
        .await
        .expect("preference owner should delete");
    assert!(
        user_preference::Entity::find_by_id(USER_A_ID)
            .one(&fixture.database)
            .await
            .expect("preference cascade should query")
            .is_none()
    );
    assert_eq!(
        fixture
            .repository
            .get(USER_A_ID, Locale::En)
            .await
            .expect_err("deleted user preferences should be unavailable"),
        PreferenceError::UserUnavailable
    );
}

#[tokio::test]
async fn concurrent_disjoint_patches_preserve_both_changes() {
    let fixture = RepositoryFixture::new().await;
    let theme_repository = fixture.repository.clone();
    let density_repository = fixture.repository.clone();

    let (theme, density) = tokio::join!(
        theme_repository.update(
            USER_A_ID,
            Locale::En,
            UpdateUserPreferences {
                theme_mode: Some(ThemeMode::Dark),
                ..Default::default()
            },
        ),
        density_repository.update(
            USER_A_ID,
            Locale::En,
            UpdateUserPreferences {
                layout_density: Some(LayoutDensity::Spacious),
                ..Default::default()
            },
        ),
    );
    theme.expect("theme patch should commit");
    density.expect("density patch should commit");

    let stored = fixture
        .repository
        .get(USER_A_ID, Locale::En)
        .await
        .expect("concurrent preferences should load");
    assert_eq!(stored.theme_mode, ThemeMode::Dark);
    assert_eq!(stored.layout_density, LayoutDensity::Spacious);
}

#[tokio::test]
async fn corrupt_storage_is_redacted_and_fails_closed() {
    let fixture = RepositoryFixture::new().await;
    let transaction = fixture
        .database
        .begin()
        .await
        .expect("corrupt fixture transaction should begin");
    transaction
        .execute_unprepared("PRAGMA ignore_check_constraints = ON")
        .await
        .expect("SQLite corrupt fixture should disable check constraints");
    user_preference::ActiveModel {
        user_id: Set(USER_A_ID.to_owned()),
        locale: Set("en".to_owned()),
        theme_mode: Set("SECRET-BROKEN-THEME".to_owned()),
        layout_density: Set("BALANCED".to_owned()),
        reading_font_scale: Set(100),
        created_at: Set(OffsetDateTime::now_utc()),
        updated_at: Set(OffsetDateTime::now_utc()),
    }
    .insert(&transaction)
    .await
    .expect("corrupt fixture should bypass the database check");
    transaction
        .commit()
        .await
        .expect("corrupt fixture should commit");

    let error = fixture
        .repository
        .get(USER_A_ID, Locale::En)
        .await
        .expect_err("corrupt preferences should fail closed");
    assert_eq!(error, PreferenceError::CorruptData);
    assert!(!format!("{error:?}").contains("SECRET-BROKEN-THEME"));
    assert!(!error.to_string().contains("SECRET-BROKEN-THEME"));
}

struct RepositoryFixture {
    _data: TempDir,
    database: DatabaseConnection,
    repository: PreferenceRepository,
}

impl RepositoryFixture {
    async fn new() -> Self {
        let data = tempfile::tempdir().expect("temporary directory should be created");
        let url = format!(
            "sqlite://{}?mode=rwc",
            data.path().join("preference-repository.db").display()
        );
        let database = connect(&DatabaseConfig::new(SecretString::from(url)))
            .await
            .expect("preference repository database should connect");
        migrate(&database)
            .await
            .expect("preference repository database should migrate");
        insert_user(&database, USER_A_ID, "preference-user-a").await;
        insert_user(&database, USER_B_ID, "preference-user-b").await;
        let repository = PreferenceRepository::new(database.clone());
        Self {
            _data: data,
            database,
            repository,
        }
    }
}
