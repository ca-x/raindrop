use serde::{Deserialize, Serialize};

use super::{PluginRegistryError, PluginRegistryErrorKind, PluginRegistryRepository};

const SETTINGS_KEY: &str = "interaction.tambo.v1";

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TamboPreferences {
    pub enabled: bool,
    pub revision: Option<u64>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct StoredPreferences {
    schema_version: u32,
    enabled: bool,
}

impl PluginRegistryRepository {
    pub async fn get_tambo_preferences(
        &self,
        user_id: &str,
    ) -> Result<TamboPreferences, PluginRegistryError> {
        let stored = self
            .get_kv(raindrop_ai_content::PLUGIN_KEY, user_id, SETTINGS_KEY)
            .await?;
        let Some(stored) = stored else {
            return Ok(TamboPreferences::default());
        };
        let document: StoredPreferences = serde_json::from_slice(stored.value())
            .map_err(|_| PluginRegistryError::new(PluginRegistryErrorKind::CorruptData))?;
        if document.schema_version != 1 {
            return Err(PluginRegistryError::new(
                PluginRegistryErrorKind::CorruptData,
            ));
        }
        Ok(TamboPreferences {
            enabled: document.enabled,
            revision: Some(stored.revision()),
        })
    }

    pub async fn replace_tambo_preferences(
        &self,
        user_id: &str,
        expected_revision: Option<u64>,
        enabled: bool,
    ) -> Result<TamboPreferences, PluginRegistryError> {
        let document = StoredPreferences {
            schema_version: 1,
            enabled,
        };
        let bytes = serde_json::to_vec(&document)
            .map_err(|_| PluginRegistryError::new(PluginRegistryErrorKind::InvalidConfig))?;
        let stored = self
            .put_kv(
                raindrop_ai_content::PLUGIN_KEY,
                user_id,
                SETTINGS_KEY,
                expected_revision,
                bytes,
            )
            .await?;
        Ok(TamboPreferences {
            enabled,
            revision: Some(stored.revision()),
        })
    }
}
