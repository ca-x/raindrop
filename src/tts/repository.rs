use std::sync::Arc;

use raindrop_tts::SpeechParameters;
use sea_orm::{
    ActiveValue::Set, ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, sea_query::Expr,
};
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Deserializer, Serialize};

use super::SpeechError;
use crate::{
    content::provider::{ProviderEndpoint, ProviderKind, ProviderSecretKeyring},
    db::entities::tts_config,
    feeds::{AddressDecision, AddressPolicy},
};

const SECRET_PURPOSE: &str = "raindrop.tts.api-key.v1";

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SpeechConfig {
    pub is_enabled: bool,
    pub base_url: String,
    pub model: String,
    pub voice: String,
    pub speed: f64,
    pub has_api_key: bool,
    pub keyring_available: bool,
    pub revision: i64,
}

#[derive(Default)]
pub(super) enum KeyUpdate {
    #[default]
    Keep,
    Clear,
    Replace(SecretString),
}

fn deserialize_key<'de, D: Deserializer<'de>>(deserializer: D) -> Result<KeyUpdate, D::Error> {
    Ok(Option::<String>::deserialize(deserializer)?
        .map_or(KeyUpdate::Clear, |key| KeyUpdate::Replace(key.into())))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SpeechDraft {
    #[serde(default)]
    pub is_enabled: bool,
    pub base_url: String,
    pub model: String,
    pub voice: String,
    pub speed: f64,
    #[serde(default)]
    pub revision: i64,
    #[serde(default, deserialize_with = "deserialize_key")]
    pub(super) api_key: KeyUpdate,
    #[serde(default)]
    pub text: Option<String>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct Settings {
    pub is_enabled: bool,
    pub base_url: String,
    pub model: String,
    pub voice: String,
    pub speed: f64,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            is_enabled: false,
            base_url: "https://api.openai.com/v1".into(),
            model: "tts-1".into(),
            voice: "alloy".into(),
            speed: 1.0,
        }
    }
}

impl Settings {
    pub fn parameters(&self) -> SpeechParameters {
        SpeechParameters {
            model: self.model.clone(),
            voice: self.voice.clone(),
            speed: self.speed,
        }
    }

    fn normalize(mut self) -> Result<Self, SpeechError> {
        if !self.parameters().is_valid() {
            return Err(SpeechError::InvalidInput);
        }
        let endpoint = speech_endpoint(&self.base_url)?;
        self.base_url = endpoint.as_str().trim_end_matches('/').into();
        Ok(self)
    }
}

pub(super) fn speech_endpoint(raw: &str) -> Result<ProviderEndpoint, SpeechError> {
    let endpoint = ProviderEndpoint::new(ProviderKind::OpenAiChatCompletions, Some(raw))
        .map_err(|_| SpeechError::InvalidInput)?;
    let mut url = url::Url::parse(endpoint.as_str()).map_err(|_| SpeechError::InvalidInput)?;
    if url.scheme() != "https"
        || endpoint.canonical_host() == "localhost"
        || endpoint.canonical_host().ends_with(".localhost")
        || endpoint.ip_address().is_some_and(|address| {
            AddressPolicy::public_only().classify(address) != AddressDecision::Allowed
        })
    {
        return Err(SpeechError::InvalidInput);
    }
    if url.path() == "/" {
        url.set_path("/v1/");
    }
    if url.path().trim_end_matches('/').ends_with("/audio/speech") {
        return Err(SpeechError::InvalidInput);
    }
    ProviderEndpoint::new(ProviderKind::OpenAiChatCompletions, Some(url.as_str()))
        .map_err(|_| SpeechError::InvalidInput)
}

#[derive(Clone)]
pub struct SpeechRepository {
    database: DatabaseConnection,
    keyring: Option<Arc<ProviderSecretKeyring>>,
}

impl SpeechRepository {
    #[must_use]
    pub fn new(database: DatabaseConnection, keyring: Option<Arc<ProviderSecretKeyring>>) -> Self {
        Self { database, keyring }
    }

    async fn stored(&self, user_id: &str) -> Result<Option<tts_config::Model>, SpeechError> {
        tts_config::Entity::find_by_id(user_id)
            .one(&self.database)
            .await
            .map_err(|_| SpeechError::Storage)
    }

    pub async fn get(&self, user_id: &str) -> Result<SpeechConfig, SpeechError> {
        let stored = self.stored(user_id).await?;
        self.public_config(stored.as_ref())
    }

    fn public_config(
        &self,
        stored: Option<&tts_config::Model>,
    ) -> Result<SpeechConfig, SpeechError> {
        let settings = stored_settings(stored)?;
        Ok(SpeechConfig {
            is_enabled: settings.is_enabled,
            base_url: settings.base_url,
            model: settings.model,
            voice: settings.voice,
            speed: settings.speed,
            has_api_key: stored.is_some_and(|row| row.encrypted_api_key.is_some()),
            keyring_available: self.keyring.is_some(),
            revision: stored.map_or(0, |row| row.revision),
        })
    }

    pub async fn save(
        &self,
        user_id: &str,
        draft: SpeechDraft,
    ) -> Result<SpeechConfig, SpeechError> {
        if draft.text.is_some() || draft.revision < 0 || draft.revision == i64::MAX {
            return Err(SpeechError::InvalidInput);
        }
        let stored = self.stored(user_id).await?;
        if stored.as_ref().map_or(0, |row| row.revision) != draft.revision {
            return Err(SpeechError::Conflict);
        }
        let revision = draft.revision + 1;
        let settings = draft_settings(&draft)?;
        let key = match draft.api_key {
            KeyUpdate::Keep => stored
                .as_ref()
                .and_then(|row| row.encrypted_api_key.clone()),
            KeyUpdate::Clear => None,
            KeyUpdate::Replace(key) => {
                validate_key(&key)?;
                Some(
                    self.keyring
                        .as_ref()
                        .ok_or(SpeechError::KeyringUnavailable)?
                        .encrypt_scoped(user_id, SECRET_PURPOSE, &key)
                        .map_err(|_| SpeechError::KeyringUnavailable)?,
                )
            }
        };
        let document = serde_json::to_string(&settings).map_err(|_| SpeechError::Storage)?;
        let committed = tts_config::Model {
            user_id: user_id.into(),
            config_json: document.clone(),
            encrypted_api_key: key.clone(),
            revision,
        };
        let affected = if stored.is_none() {
            tts_config::Entity::insert(tts_config::ActiveModel {
                user_id: Set(user_id.to_owned()),
                config_json: Set(document),
                encrypted_api_key: Set(key),
                revision: Set(revision),
            })
            .exec_without_returning(&self.database)
            .await
            .map_err(|error| match error.sql_err() {
                Some(sea_orm::SqlErr::UniqueConstraintViolation(_)) => SpeechError::Conflict,
                _ => SpeechError::Storage,
            })?
        } else {
            tts_config::Entity::update_many()
                .col_expr(tts_config::Column::ConfigJson, Expr::value(document))
                .col_expr(tts_config::Column::EncryptedApiKey, Expr::value(key))
                .col_expr(tts_config::Column::Revision, Expr::value(revision))
                .filter(tts_config::Column::UserId.eq(user_id))
                .filter(tts_config::Column::Revision.eq(draft.revision))
                .exec(&self.database)
                .await
                .map_err(|_| SpeechError::Storage)?
                .rows_affected
        };
        if affected != 1 {
            return Err(SpeechError::Conflict);
        }
        self.public_config(Some(&committed))
    }

    pub(super) async fn snapshot(
        &self,
        user_id: &str,
    ) -> Result<(Settings, i64, Option<SecretString>), SpeechError> {
        let stored = self.stored(user_id).await?;
        let settings = stored_settings(stored.as_ref())?;
        let key = self.decrypt(user_id, stored.as_ref())?;
        Ok((settings, stored.map_or(0, |row| row.revision), key))
    }

    pub(super) async fn preview(
        &self,
        user_id: &str,
        draft: &SpeechDraft,
    ) -> Result<(Settings, Option<SecretString>), SpeechError> {
        let settings = draft_settings(draft)?;
        let key = match &draft.api_key {
            KeyUpdate::Clear => None,
            KeyUpdate::Replace(key) => {
                validate_key(key)?;
                Some(key.clone())
            }
            KeyUpdate::Keep => self.decrypt(user_id, self.stored(user_id).await?.as_ref())?,
        };
        Ok((settings, key))
    }

    fn decrypt(
        &self,
        user_id: &str,
        stored: Option<&tts_config::Model>,
    ) -> Result<Option<SecretString>, SpeechError> {
        stored
            .and_then(|row| row.encrypted_api_key.as_deref())
            .map(|ciphertext| {
                self.keyring
                    .as_ref()
                    .ok_or(SpeechError::KeyringUnavailable)?
                    .decrypt_scoped(user_id, SECRET_PURPOSE, ciphertext)
                    .map_err(|_| SpeechError::KeyringUnavailable)
            })
            .transpose()
    }
}

fn draft_settings(draft: &SpeechDraft) -> Result<Settings, SpeechError> {
    Settings {
        is_enabled: draft.is_enabled,
        base_url: draft.base_url.clone(),
        model: draft.model.clone(),
        voice: draft.voice.clone(),
        speed: draft.speed,
    }
    .normalize()
}

fn stored_settings(stored: Option<&tts_config::Model>) -> Result<Settings, SpeechError> {
    match stored {
        None => Ok(Settings::default()),
        Some(row) if row.revision > 0 && row.config_json.len() <= 4096 => {
            serde_json::from_str::<Settings>(&row.config_json)
                .map_err(|_| SpeechError::Storage)?
                .normalize()
                .map_err(|_| SpeechError::Storage)
        }
        Some(_) => Err(SpeechError::Storage),
    }
}

fn validate_key(key: &SecretString) -> Result<(), SpeechError> {
    let key = key.expose_secret();
    if key.is_empty()
        || key.len() > 8192
        || key.trim() != key
        || !key.bytes().all(|byte| byte.is_ascii_graphic())
    {
        Err(SpeechError::InvalidInput)
    } else {
        Ok(())
    }
}
