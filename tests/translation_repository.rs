#[allow(dead_code)]
mod support;

use std::sync::Arc;

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use raindrop::{
    content::provider::ProviderSecretKeyring,
    db::{entities::translation_config, migrate},
    translation::{
        AiTranslationProfile, ApiKeyUpdate, SaveTranslationConfig, TranslationDisplayMode,
        TranslationEngine, TranslationErrorKind, TranslationRepository,
    },
};
use sea_orm::{EntityTrait, PaginatorTrait};
use secrecy::{ExposeSecret, SecretString};
use support::database::{USER_A_ID, USER_B_ID, connect_for_contract, insert_user};
use tempfile::tempdir;

#[tokio::test]
async fn deeplx_configuration_is_encrypted_revisioned_and_isolated_per_user() {
    let data = tempdir().expect("temporary directory should be created");
    let database = connect_for_contract(SecretString::from(format!(
        "sqlite://{}?mode=rwc",
        data.path().join("translation.db").display()
    )))
    .await;
    migrate(&database)
        .await
        .expect("translation migrations should apply");
    insert_user(&database, USER_A_ID, "translator-a").await;
    insert_user(&database, USER_B_ID, "translator-b").await;

    let repository =
        TranslationRepository::new(database.clone(), Some(Arc::new(keyring("primary", 41))));
    let initial = repository
        .get(USER_A_ID)
        .await
        .expect("default config should load");
    assert_eq!(initial.revision, None);
    assert!(!initial.is_enabled);
    assert!(!initial.deeplx.has_api_key);

    let saved = repository
        .save(
            USER_A_ID,
            deeplx_config(
                None,
                true,
                Some("https://api.deeplx.org/{{apiKey}}/translate"),
                ApiKeyUpdate::Set(SecretString::from("deeplx-key-sentinel")),
            ),
        )
        .await
        .expect("DeepLX config should save");
    assert_eq!(saved.revision, Some(0));
    assert!(saved.is_enabled);
    assert!(saved.deeplx.has_api_key);
    assert_eq!(saved.deeplx.display_name, "DeepLX");

    let row = translation_config::Entity::find_by_id(USER_A_ID)
        .one(&database)
        .await
        .expect("translation row should query")
        .expect("translation row should exist");
    let encrypted = row
        .encrypted_deep_lx_api_key
        .expect("encrypted API key should be stored");
    assert!(encrypted.starts_with("rdsec1.primary."));
    assert!(!encrypted.contains("deeplx-key-sentinel"));
    assert_eq!(
        repository
            .deeplx_api_key(USER_A_ID)
            .await
            .expect("saved key should decrypt")
            .expect("saved key should exist")
            .expose_secret(),
        "deeplx-key-sentinel"
    );

    let renamed = repository
        .save(
            USER_A_ID,
            SaveTranslationConfig {
                expected_revision: saved.revision,
                deeplx_display_name: "  Private DeepLX  ".to_owned(),
                deeplx_api_key: ApiKeyUpdate::Keep,
                ..deeplx_config(
                    saved.revision,
                    true,
                    Some("https://api.deeplx.org/{{apiKey}}/translate"),
                    ApiKeyUpdate::Keep,
                )
            },
        )
        .await
        .expect("metadata update should keep the key");
    assert_eq!(renamed.revision, Some(1));
    assert_eq!(renamed.deeplx.display_name, "Private DeepLX");
    assert!(renamed.deeplx.has_api_key);

    assert_eq!(
        repository
            .save(
                USER_A_ID,
                deeplx_config(
                    saved.revision,
                    true,
                    Some("https://api.deeplx.org/{{apiKey}}/translate"),
                    ApiKeyUpdate::Keep,
                ),
            )
            .await
            .expect_err("stale revision should fail")
            .kind(),
        TranslationErrorKind::RevisionConflict
    );

    let other = repository
        .get(USER_B_ID)
        .await
        .expect("other user's default should load");
    assert_eq!(other.revision, None);
    assert!(!other.deeplx.has_api_key);
    assert_eq!(
        translation_config::Entity::find_by_id(USER_B_ID)
            .count(&database)
            .await
            .expect("other user's rows should count"),
        0
    );
    assert_eq!(
        repository
            .save(
                USER_B_ID,
                deeplx_config(
                    None,
                    true,
                    Some("https://api.deeplx.org/{{apiKey}}/translate"),
                    ApiKeyUpdate::Keep,
                ),
            )
            .await
            .expect_err("URL key placeholder should require a key")
            .kind(),
        TranslationErrorKind::InvalidInput
    );
}

fn deeplx_config(
    expected_revision: Option<u64>,
    is_enabled: bool,
    base_url: Option<&str>,
    api_key: ApiKeyUpdate,
) -> SaveTranslationConfig {
    SaveTranslationConfig {
        expected_revision,
        engine: TranslationEngine::DeepLx,
        display_mode: TranslationDisplayMode::Bilingual,
        is_enabled,
        default_target_locale: "zh-CN".to_owned(),
        open_ai_provider_id: None,
        open_ai_max_output_tokens: 4096,
        open_ai_profile: AiTranslationProfile::General,
        open_ai_custom_system_prompt: None,
        open_ai_custom_prompt: None,
        deeplx_display_name: "DeepLX".to_owned(),
        deeplx_description: Some("Private translation endpoint".to_owned()),
        deeplx_base_url: base_url.map(str::to_owned),
        deeplx_api_key: api_key,
    }
}

fn keyring(id: &str, byte: u8) -> ProviderSecretKeyring {
    ProviderSecretKeyring::from_entries(&[SecretString::from(format!(
        "{id}:{}",
        URL_SAFE_NO_PAD.encode([byte; 32])
    ))])
    .expect("test keyring should construct")
}
