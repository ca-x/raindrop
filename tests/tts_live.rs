#[allow(dead_code)]
mod support;

use std::{path::PathBuf, sync::Arc};

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use raindrop::{
    content::provider::{HttpsProviderTransport, ProviderSecretKeyring},
    db::{DatabaseConfig, connect, migrate},
    feeds::FeedRepository,
    tts::{SpeechDraft, SpeechRepository, SpeechService},
};
use secrecy::SecretString;
use serde_json::json;
use support::database::{USER_A_ID, insert_user};

#[tokio::test]
#[ignore = "requires an explicitly configured live TTS endpoint and credential file"]
async fn configured_live_provider_returns_mp3_using_the_selected_voice() {
    let endpoint = std::env::var("RAINDROP_TTS_LIVE_URL").expect("set RAINDROP_TTS_LIVE_URL");
    let key_file =
        std::env::var_os("RAINDROP_TTS_LIVE_TOKEN_FILE").expect("set RAINDROP_TTS_LIVE_TOKEN_FILE");
    let credential = zeroize::Zeroizing::new(
        std::fs::read_to_string(key_file).expect("read live credential file"),
    );
    let voice =
        std::env::var("RAINDROP_TTS_LIVE_VOICE").unwrap_or_else(|_| "zh-CN-YunyangNeural".into());
    let directory = tempfile::tempdir().unwrap();
    let database = connect(&DatabaseConfig::new(SecretString::from(format!(
        "sqlite://{}?mode=rwc",
        directory.path().join("live.sqlite3").display()
    ))))
    .await
    .unwrap();
    migrate(&database).await.unwrap();
    insert_user(&database, USER_A_ID, "live-speech-test").await;
    let keyring = ProviderSecretKeyring::from_entries(&[SecretString::from(format!(
        "test:{}",
        URL_SAFE_NO_PAD.encode([37_u8; 32])
    ))])
    .unwrap();
    let repository = SpeechRepository::new(database.clone(), Some(Arc::new(keyring)));
    let draft: SpeechDraft = serde_json::from_value(json!({"baseUrl":endpoint,"model":"tts-1","voice":voice,"speed":1.0,"isEnabled":true,"revision":0,"apiKey":credential.trim()})).unwrap();
    let saved = repository
        .save(USER_A_ID, draft)
        .await
        .expect("save encrypted live speech configuration");
    assert_eq!(saved.voice, voice);
    assert!(saved.has_api_key);
    let draft: SpeechDraft = serde_json::from_value(json!({"baseUrl":saved.base_url,"model":saved.model,"voice":saved.voice,"speed":saved.speed,"isEnabled":true,"text":"你好，这是 Raindrop 的语音测试。"})).unwrap();
    let service = SpeechService::new(
        repository,
        FeedRepository::new(database),
        Arc::new(HttpsProviderTransport::public_only().unwrap()),
    );
    let audio = service
        .test(USER_A_ID, draft)
        .await
        .expect("live provider should return MP3, not an HTML challenge or API error");
    assert!(audio.len() > 128 && audio.len() <= 2 * 1024 * 1024);
    if let Some(path) = std::env::var_os("RAINDROP_TTS_LIVE_AUDIO_FILE") {
        std::fs::write(PathBuf::from(path), audio).expect("write optional live audio artifact");
    }
}
