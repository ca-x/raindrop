use std::sync::Arc;

use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter,
    sea_query::Expr,
};
use secrecy::SecretString;
use time::OffsetDateTime;
use uuid::Uuid;

use crate::{
    content::provider::ProviderSecretKeyring,
    db::entities::translation_config,
    plugins::{PluginRegistryErrorKind, json::normalize_locale},
};

use super::{
    AiTranslationProfile, ApiKeyUpdate, DeepLxSettings, OpenAiSettings, SaveTranslationConfig,
    TranslationConfig, TranslationDisplayMode, TranslationEngine, TranslationError,
    TranslationErrorKind,
    deeplx::{requires_url_api_key, validate_base_url_template},
    openai::validate_custom_prompts,
};

const SECRET_PURPOSE: &str = "TRANSLATION_DEEPLX_API_KEY";

#[derive(Clone)]
pub struct TranslationRepository {
    database: DatabaseConnection,
    keyring: Option<Arc<ProviderSecretKeyring>>,
}

impl TranslationRepository {
    #[must_use]
    pub fn new(database: DatabaseConnection, keyring: Option<Arc<ProviderSecretKeyring>>) -> Self {
        Self { database, keyring }
    }

    pub async fn get(&self, user_id: &str) -> Result<TranslationConfig, TranslationError> {
        let stored = translation_config::Entity::find_by_id(user_id)
            .one(&self.database)
            .await
            .map_err(database_error)?;
        stored.map_or_else(|| Ok(default_config(user_id)), config_from_model)
    }

    pub async fn save(
        &self,
        user_id: &str,
        input: SaveTranslationConfig,
    ) -> Result<TranslationConfig, TranslationError> {
        let target_locale = normalize_locale(
            input.default_target_locale.trim(),
            PluginRegistryErrorKind::InvalidConfig,
        )
        .map_err(|_| invalid_input())?;
        let open_ai_provider_id = normalize_provider_id(input.open_ai_provider_id)?;
        if !(256..=16_384).contains(&input.open_ai_max_output_tokens) {
            return Err(invalid_input());
        }
        if input.is_enabled
            && input.engine == TranslationEngine::OpenAi
            && open_ai_provider_id.is_none()
        {
            return Err(invalid_input());
        }
        let (custom_system_prompt, custom_prompt) = validate_custom_prompts(
            input.open_ai_profile,
            input.open_ai_custom_system_prompt.as_deref(),
            input.open_ai_custom_prompt.as_deref(),
        )?;
        let deep_lx_display_name = normalize_display_name(&input.deeplx_display_name)?;
        let deep_lx_description = normalize_description(input.deeplx_description)?;
        let deep_lx_base_url = validate_base_url_template(input.deeplx_base_url.as_deref())?;
        let stored = translation_config::Entity::find_by_id(user_id)
            .one(&self.database)
            .await
            .map_err(database_error)?;
        let encrypted_api_key = self.updated_secret(
            user_id,
            stored
                .as_ref()
                .and_then(|model| model.encrypted_deep_lx_api_key.clone()),
            input.deeplx_api_key,
        )?;
        if input.is_enabled
            && input.engine == TranslationEngine::DeepLx
            && requires_url_api_key(deep_lx_base_url.as_deref())
            && encrypted_api_key.is_none()
        {
            return Err(invalid_input());
        }

        let now = OffsetDateTime::now_utc();
        let saved = if let Some(stored) = stored {
            let current_revision = u64::try_from(stored.revision).map_err(|_| corrupt_data())?;
            if input.expected_revision != Some(current_revision) {
                return Err(TranslationError::new(
                    TranslationErrorKind::RevisionConflict,
                ));
            }
            let next_revision = stored.revision.checked_add(1).ok_or_else(corrupt_data)?;
            let result = translation_config::Entity::update_many()
                .col_expr(
                    translation_config::Column::Engine,
                    Expr::value(input.engine.as_storage()),
                )
                .col_expr(
                    translation_config::Column::DisplayMode,
                    Expr::value(input.display_mode.as_storage()),
                )
                .col_expr(
                    translation_config::Column::IsEnabled,
                    Expr::value(input.is_enabled),
                )
                .col_expr(
                    translation_config::Column::DefaultTargetLocale,
                    Expr::value(target_locale),
                )
                .col_expr(
                    translation_config::Column::OpenAiProviderId,
                    Expr::value(open_ai_provider_id),
                )
                .col_expr(
                    translation_config::Column::OpenAiMaxOutputTokens,
                    Expr::value(
                        i32::try_from(input.open_ai_max_output_tokens)
                            .map_err(|_| invalid_input())?,
                    ),
                )
                .col_expr(
                    translation_config::Column::OpenAiProfile,
                    Expr::value(input.open_ai_profile.as_storage()),
                )
                .col_expr(
                    translation_config::Column::OpenAiCustomSystemPrompt,
                    Expr::value(custom_system_prompt),
                )
                .col_expr(
                    translation_config::Column::OpenAiCustomPrompt,
                    Expr::value(custom_prompt),
                )
                .col_expr(
                    translation_config::Column::DeepLxDisplayName,
                    Expr::value(deep_lx_display_name),
                )
                .col_expr(
                    translation_config::Column::DeepLxDescription,
                    Expr::value(deep_lx_description),
                )
                .col_expr(
                    translation_config::Column::DeepLxBaseUrl,
                    Expr::value(deep_lx_base_url),
                )
                .col_expr(
                    translation_config::Column::DeepLxIsProgressive,
                    Expr::value(input.deeplx_is_progressive),
                )
                .col_expr(
                    translation_config::Column::EncryptedDeepLxApiKey,
                    Expr::value(encrypted_api_key),
                )
                .col_expr(
                    translation_config::Column::Revision,
                    Expr::value(next_revision),
                )
                .col_expr(translation_config::Column::UpdatedAt, Expr::value(now))
                .filter(translation_config::Column::UserId.eq(user_id))
                .filter(translation_config::Column::Revision.eq(stored.revision))
                .exec(&self.database)
                .await
                .map_err(database_error)?;
            if result.rows_affected != 1 {
                return Err(TranslationError::new(
                    TranslationErrorKind::RevisionConflict,
                ));
            }
            translation_config::Entity::find_by_id(user_id)
                .one(&self.database)
                .await
                .map_err(database_error)?
                .ok_or_else(corrupt_data)?
        } else {
            if input.expected_revision.is_some() {
                return Err(TranslationError::new(
                    TranslationErrorKind::RevisionConflict,
                ));
            }
            translation_config::ActiveModel {
                user_id: Set(user_id.to_owned()),
                engine: Set(input.engine.as_storage().to_owned()),
                display_mode: Set(input.display_mode.as_storage().to_owned()),
                is_enabled: Set(input.is_enabled),
                default_target_locale: Set(target_locale),
                open_ai_provider_id: Set(open_ai_provider_id),
                open_ai_max_output_tokens: Set(
                    i32::try_from(input.open_ai_max_output_tokens).map_err(|_| invalid_input())?
                ),
                open_ai_profile: Set(input.open_ai_profile.as_storage().to_owned()),
                open_ai_custom_system_prompt: Set(custom_system_prompt),
                open_ai_custom_prompt: Set(custom_prompt),
                deep_lx_display_name: Set(deep_lx_display_name),
                deep_lx_description: Set(deep_lx_description),
                deep_lx_base_url: Set(deep_lx_base_url),
                deep_lx_is_progressive: Set(input.deeplx_is_progressive),
                encrypted_deep_lx_api_key: Set(encrypted_api_key),
                revision: Set(0),
                created_at: Set(now),
                updated_at: Set(now),
            }
            .insert(&self.database)
            .await
            .map_err(database_error)?
        };
        config_from_model(saved)
    }

    pub async fn deeplx_api_key(
        &self,
        user_id: &str,
    ) -> Result<Option<SecretString>, TranslationError> {
        let encrypted = translation_config::Entity::find_by_id(user_id)
            .one(&self.database)
            .await
            .map_err(database_error)?
            .and_then(|model| model.encrypted_deep_lx_api_key);
        let Some(encrypted) = encrypted else {
            return Ok(None);
        };
        self.decrypt_secret(user_id, &encrypted).map(Some)
    }

    fn updated_secret(
        &self,
        user_id: &str,
        current: Option<String>,
        update: ApiKeyUpdate,
    ) -> Result<Option<String>, TranslationError> {
        match update {
            ApiKeyUpdate::Keep => Ok(current),
            ApiKeyUpdate::Clear => Ok(None),
            ApiKeyUpdate::Set(api_key) => {
                validate_api_key(&api_key)?;
                let keyring = self.keyring.as_ref().ok_or_else(|| {
                    TranslationError::new(TranslationErrorKind::KeyringUnavailable)
                })?;
                keyring
                    .encrypt_scoped(user_id, SECRET_PURPOSE, &api_key)
                    .map(Some)
                    .map_err(|_| TranslationError::new(TranslationErrorKind::KeyringUnavailable))
            }
        }
    }

    fn decrypt_secret(
        &self,
        user_id: &str,
        encrypted: &str,
    ) -> Result<SecretString, TranslationError> {
        self.keyring
            .as_ref()
            .ok_or_else(|| TranslationError::new(TranslationErrorKind::KeyringUnavailable))?
            .decrypt_scoped(user_id, SECRET_PURPOSE, encrypted)
            .map_err(|_| TranslationError::new(TranslationErrorKind::KeyringUnavailable))
    }
}

fn default_config(user_id: &str) -> TranslationConfig {
    TranslationConfig {
        user_id: user_id.to_owned(),
        engine: TranslationEngine::DeepLx,
        display_mode: TranslationDisplayMode::Bilingual,
        is_enabled: false,
        default_target_locale: "zh-CN".to_owned(),
        open_ai: OpenAiSettings {
            provider_id: None,
            max_output_tokens: 4096,
            profile: AiTranslationProfile::General,
            custom_system_prompt: None,
            custom_prompt: None,
        },
        deeplx: DeepLxSettings {
            display_name: "DeepLX".to_owned(),
            description: None,
            base_url: None,
            is_progressive: true,
            has_api_key: false,
        },
        revision: None,
    }
}

fn config_from_model(
    model: translation_config::Model,
) -> Result<TranslationConfig, TranslationError> {
    let revision = u64::try_from(model.revision).map_err(|_| corrupt_data())?;
    let engine = TranslationEngine::parse(&model.engine).map_err(|_| corrupt_data())?;
    let display_mode =
        TranslationDisplayMode::parse(&model.display_mode).map_err(|_| corrupt_data())?;
    let target_locale = normalize_locale(
        &model.default_target_locale,
        PluginRegistryErrorKind::InvalidConfig,
    )
    .map_err(|_| corrupt_data())?;
    if target_locale != model.default_target_locale {
        return Err(corrupt_data());
    }
    let provider_id =
        normalize_provider_id(model.open_ai_provider_id.clone()).map_err(|_| corrupt_data())?;
    if provider_id != model.open_ai_provider_id {
        return Err(corrupt_data());
    }
    let max_output_tokens =
        u32::try_from(model.open_ai_max_output_tokens).map_err(|_| corrupt_data())?;
    if !(256..=16_384).contains(&max_output_tokens) {
        return Err(corrupt_data());
    }
    let profile =
        AiTranslationProfile::parse(&model.open_ai_profile).map_err(|_| corrupt_data())?;
    let (custom_system_prompt, custom_prompt) = validate_custom_prompts(
        profile,
        model.open_ai_custom_system_prompt.as_deref(),
        model.open_ai_custom_prompt.as_deref(),
    )
    .map_err(|_| corrupt_data())?;
    if custom_system_prompt != model.open_ai_custom_system_prompt
        || custom_prompt != model.open_ai_custom_prompt
    {
        return Err(corrupt_data());
    }
    let deep_lx_display_name =
        normalize_display_name(&model.deep_lx_display_name).map_err(|_| corrupt_data())?;
    let deep_lx_description =
        normalize_description(model.deep_lx_description.clone()).map_err(|_| corrupt_data())?;
    let deep_lx_base_url = validate_base_url_template(model.deep_lx_base_url.as_deref())
        .map_err(|_| corrupt_data())?;
    if deep_lx_display_name != model.deep_lx_display_name
        || deep_lx_description != model.deep_lx_description
        || deep_lx_base_url != model.deep_lx_base_url
    {
        return Err(corrupt_data());
    }
    Ok(TranslationConfig {
        user_id: model.user_id,
        engine,
        display_mode,
        is_enabled: model.is_enabled,
        default_target_locale: target_locale,
        open_ai: OpenAiSettings {
            provider_id,
            max_output_tokens,
            profile,
            custom_system_prompt,
            custom_prompt,
        },
        deeplx: DeepLxSettings {
            display_name: deep_lx_display_name,
            description: deep_lx_description,
            base_url: deep_lx_base_url,
            is_progressive: model.deep_lx_is_progressive,
            has_api_key: model.encrypted_deep_lx_api_key.is_some(),
        },
        revision: Some(revision),
    })
}

fn normalize_provider_id(value: Option<String>) -> Result<Option<String>, TranslationError> {
    let Some(value) = value else {
        return Ok(None);
    };
    let value = value.trim();
    if value.is_empty() {
        return Ok(None);
    }
    let parsed = Uuid::parse_str(value).map_err(|_| invalid_input())?;
    if parsed.to_string() != value {
        return Err(invalid_input());
    }
    Ok(Some(value.to_owned()))
}

fn normalize_display_name(value: &str) -> Result<String, TranslationError> {
    let value = value.trim();
    if value.is_empty() || value.chars().count() > 80 || value.chars().any(char::is_control) {
        return Err(invalid_input());
    }
    Ok(value.to_owned())
}

fn normalize_description(value: Option<String>) -> Result<Option<String>, TranslationError> {
    let Some(value) = value else {
        return Ok(None);
    };
    let value = value.trim();
    if value.is_empty() {
        return Ok(None);
    }
    if value.chars().count() > 500 || value.chars().any(char::is_control) {
        return Err(invalid_input());
    }
    Ok(Some(value.to_owned()))
}

pub(crate) fn validate_api_key(value: &SecretString) -> Result<(), TranslationError> {
    use secrecy::ExposeSecret;
    let value = value.expose_secret();
    if value.is_empty()
        || value.len() > 1024
        || !value.is_ascii()
        || value
            .chars()
            .any(|character| character.is_control() || character.is_whitespace())
    {
        return Err(invalid_input());
    }
    Ok(())
}

fn database_error(_: sea_orm::DbErr) -> TranslationError {
    TranslationError::new(TranslationErrorKind::Database)
}

const fn invalid_input() -> TranslationError {
    TranslationError::new(TranslationErrorKind::InvalidInput)
}

const fn corrupt_data() -> TranslationError {
    TranslationError::new(TranslationErrorKind::CorruptData)
}
