use std::{future::Future, sync::Arc, time::Duration};

use secrecy::SecretString;
use tokio::time::timeout;

use crate::{
    content::{
        provider::{ProviderCoreErrorKind, ProviderKind, ProviderRepository},
        sanitize::rendered_translation_segments,
    },
    feeds::{FeedRepository, RepositoryError},
    plugins::{PluginRegistryErrorKind, json::normalize_locale},
};

use super::{
    ApiKeyUpdate, SaveTranslationConfig, TestTranslationInput, TranslationConfig,
    TranslationEngine, TranslationError, TranslationErrorKind, TranslationLookupResult,
    TranslationRepository, TranslationResult, TranslationSegment, TranslationTestResult,
    TranslationTextResult,
    deeplx::{
        DeepLxTranslateInput, DeepLxTransport, requires_url_api_key, validate_base_url_template,
    },
    openai::{
        OpenAiLookupInput, OpenAiSourceSegment, OpenAiTranslateBatchInput, OpenAiTranslateInput,
        OpenAiTranslationTransport, validate_custom_prompts,
    },
    repository::validate_api_key,
};

const TEST_TEXT: &str = "Raindrop connection test";
const MAX_ARTICLE_CHARACTERS: usize = 24_000;
const MAX_ARTICLE_SEGMENTS: usize = 96;
const MAX_LOOKUP_CHARACTERS: usize = 200;
const MAX_SELECTION_CHARACTERS: usize = 8_000;
const ANONYMOUS_CHUNK_CHARACTERS: usize = 1_350;
const AUTHENTICATED_CHUNK_CHARACTERS: usize = 3_500;
const MAX_OPENAI_BATCH_CHARACTERS: usize = 6_000;
const MAX_OPENAI_BATCH_SEGMENTS: usize = 24;
const MAX_DEEPLX_ARTICLE_CALLS: usize = 48;
const ENTRY_TRANSLATION_TIMEOUT: Duration = Duration::from_secs(120);

pub struct TranslationService {
    repository: TranslationRepository,
    feeds: FeedRepository,
    providers: ProviderRepository,
    deeplx: Arc<dyn DeepLxTransport>,
    openai: Arc<dyn OpenAiTranslationTransport>,
}

impl TranslationService {
    #[must_use]
    pub fn new(
        repository: TranslationRepository,
        feeds: FeedRepository,
        providers: ProviderRepository,
        deeplx: Arc<dyn DeepLxTransport>,
        openai: Arc<dyn OpenAiTranslationTransport>,
    ) -> Self {
        Self {
            repository,
            feeds,
            providers,
            deeplx,
            openai,
        }
    }

    pub async fn get_config(&self, user_id: &str) -> Result<TranslationConfig, TranslationError> {
        self.repository.get(user_id).await
    }

    pub async fn save_config(
        &self,
        user_id: &str,
        input: SaveTranslationConfig,
    ) -> Result<TranslationConfig, TranslationError> {
        if let Some(provider_id) = input.open_ai_provider_id.as_deref() {
            self.validate_openai_provider(user_id, provider_id, input.is_enabled)
                .await?;
        }
        self.repository.save(user_id, input).await
    }

    pub async fn test_connection(
        &self,
        user_id: &str,
        input: TestTranslationInput,
    ) -> Result<TranslationTestResult, TranslationError> {
        let target_locale = normalize_target_locale(&input.target_locale)?;
        match input.engine {
            TranslationEngine::OpenAi => {
                let provider_id = input
                    .open_ai_provider_id
                    .filter(|value| !value.trim().is_empty())
                    .ok_or_else(|| TranslationError::new(TranslationErrorKind::NotConfigured))?;
                self.validate_openai_provider(user_id, &provider_id, true)
                    .await?;
                if !(256..=16_384).contains(&input.open_ai_max_output_tokens) {
                    return Err(invalid_input());
                }
                let (custom_system_prompt, custom_prompt) = validate_custom_prompts(
                    input.open_ai_profile,
                    input.open_ai_custom_system_prompt.as_deref(),
                    input.open_ai_custom_prompt.as_deref(),
                )?;
                let translated = self
                    .openai
                    .translate(OpenAiTranslateInput {
                        user_id: user_id.to_owned(),
                        provider_id,
                        text: TEST_TEXT.to_owned(),
                        target_locale: target_locale.clone(),
                        max_output_tokens: input.open_ai_max_output_tokens,
                        profile: input.open_ai_profile,
                        custom_system_prompt,
                        custom_prompt,
                    })
                    .await?;
                Ok(TranslationTestResult {
                    translated_text: translated.text,
                    provider_label: translated.provider_label,
                    detected_source_locale: None,
                    target_locale,
                })
            }
            TranslationEngine::DeepLx => {
                let base_url = validate_base_url_template(input.deeplx.base_url.as_deref())?;
                let api_key = self
                    .resolve_test_api_key(user_id, input.deeplx.api_key)
                    .await?;
                if requires_url_api_key(base_url.as_deref()) && api_key.is_none() {
                    return Err(TranslationError::new(TranslationErrorKind::NotConfigured));
                }
                let translated = self
                    .deeplx
                    .translate(DeepLxTranslateInput {
                        base_url,
                        api_key,
                        text: TEST_TEXT.to_owned(),
                        target_locale: target_locale.clone(),
                    })
                    .await?;
                Ok(TranslationTestResult {
                    translated_text: translated.text,
                    provider_label: "DeepLX".to_owned(),
                    detected_source_locale: translated.detected_source_locale,
                    target_locale,
                })
            }
        }
    }

    pub async fn translate_entry(
        &self,
        user_id: &str,
        entry_id: &str,
    ) -> Result<TranslationResult, TranslationError> {
        translation_with_timeout(
            ENTRY_TRANSLATION_TIMEOUT,
            self.translate_entry_inner(user_id, entry_id),
        )
        .await
    }

    async fn translate_entry_inner(
        &self,
        user_id: &str,
        entry_id: &str,
    ) -> Result<TranslationResult, TranslationError> {
        let config = self.enabled_config(user_id).await?;
        let entry = self
            .feeds
            .get_detail_for_user(user_id, entry_id)
            .await
            .map_err(map_feed_error)?
            .ok_or_else(|| TranslationError::new(TranslationErrorKind::NotFound))?;
        let original_segments = rendered_translation_segments(&entry.content_html);
        if original_segments.is_empty()
            || original_segments.len() > MAX_ARTICLE_SEGMENTS
            || original_segments
                .iter()
                .map(|segment| segment.chars().count())
                .sum::<usize>()
                > MAX_ARTICLE_CHARACTERS
        {
            return Err(TranslationError::new(TranslationErrorKind::TooLarge));
        }

        let title = entry.title.unwrap_or_else(|| "Translation".to_owned());
        if config.engine == TranslationEngine::OpenAi {
            return self
                .translate_openai_entry(user_id, &config, title, original_segments)
                .await;
        }
        self.translate_deeplx_entry(user_id, &config, title, original_segments)
            .await
    }

    async fn translate_deeplx_entry(
        &self,
        user_id: &str,
        config: &TranslationConfig,
        title: String,
        original_segments: Vec<String>,
    ) -> Result<TranslationResult, TranslationError> {
        let api_key = self.repository.deeplx_api_key(user_id).await?;
        if requires_url_api_key(config.deeplx.base_url.as_deref()) && api_key.is_none() {
            return Err(TranslationError::new(TranslationErrorKind::NotConfigured));
        }
        let maximum = if api_key.is_some() {
            AUTHENTICATED_CHUNK_CHARACTERS
        } else {
            ANONYMOUS_CHUNK_CHARACTERS
        };
        if deeplx_article_call_count(&title, &original_segments, maximum) > MAX_DEEPLX_ARTICLE_CALLS
        {
            return Err(TranslationError::new(TranslationErrorKind::TooLarge));
        }
        let translated_title = self
            .translate_deeplx_text(config, api_key.as_ref(), title, maximum)
            .await?;
        let mut segments = Vec::with_capacity(original_segments.len());
        let mut detected_source_locale = translated_title.detected_source_locale.clone();
        let provider_label = translated_title.provider_label.clone();
        for (index, original_text) in original_segments.into_iter().enumerate() {
            let translated = self
                .translate_deeplx_text(config, api_key.as_ref(), original_text.clone(), maximum)
                .await?;
            if detected_source_locale.is_none() {
                detected_source_locale = translated.detected_source_locale;
            }
            segments.push(TranslationSegment {
                index: u32::try_from(index)
                    .map_err(|_| TranslationError::new(TranslationErrorKind::TooLarge))?,
                original_text,
                translated_text: translated.text,
            });
        }
        Ok(TranslationResult {
            title: translated_title.text,
            segments,
            provider_label,
            detected_source_locale,
            target_locale: config.default_target_locale.clone(),
        })
    }

    async fn translate_openai_entry(
        &self,
        user_id: &str,
        config: &TranslationConfig,
        title: String,
        original_segments: Vec<String>,
    ) -> Result<TranslationResult, TranslationError> {
        let provider_id = config
            .open_ai
            .provider_id
            .clone()
            .ok_or_else(|| TranslationError::new(TranslationErrorKind::NotConfigured))?;
        let mut source = Vec::with_capacity(original_segments.len() + 1);
        source.push(OpenAiSourceSegment { id: 0, text: title });
        for (index, text) in original_segments.iter().cloned().enumerate() {
            source.push(OpenAiSourceSegment {
                id: u32::try_from(index + 1)
                    .map_err(|_| TranslationError::new(TranslationErrorKind::TooLarge))?,
                text,
            });
        }
        let character_limit = usize::try_from(config.open_ai.max_output_tokens)
            .unwrap_or(MAX_OPENAI_BATCH_CHARACTERS)
            .clamp(256, MAX_OPENAI_BATCH_CHARACTERS);
        let mut translated = vec![None; source.len()];
        let mut provider_label = None;
        for batch in batch_source_segments(source, character_limit, MAX_OPENAI_BATCH_SEGMENTS) {
            let output = self
                .openai
                .translate_batch(OpenAiTranslateBatchInput {
                    user_id: user_id.to_owned(),
                    provider_id: provider_id.clone(),
                    segments: batch,
                    target_locale: config.default_target_locale.clone(),
                    max_output_tokens: config.open_ai.max_output_tokens,
                    profile: config.open_ai.profile,
                    custom_system_prompt: config.open_ai.custom_system_prompt.clone(),
                    custom_prompt: config.open_ai.custom_prompt.clone(),
                })
                .await?;
            if provider_label.is_none() {
                provider_label = Some(output.provider_label);
            }
            for segment in output.segments {
                let index = usize::try_from(segment.id).map_err(|_| upstream())?;
                let slot = translated.get_mut(index).ok_or_else(upstream)?;
                if slot.replace(segment.text).is_some() {
                    return Err(upstream());
                }
            }
        }
        let mut translated = translated.into_iter();
        let translated_title = translated.next().flatten().ok_or_else(upstream)?;
        let segments = original_segments
            .into_iter()
            .zip(translated)
            .enumerate()
            .map(|(index, (original_text, translated_text))| {
                Ok(TranslationSegment {
                    index: u32::try_from(index)
                        .map_err(|_| TranslationError::new(TranslationErrorKind::TooLarge))?,
                    original_text,
                    translated_text: translated_text.ok_or_else(upstream)?,
                })
            })
            .collect::<Result<Vec<_>, TranslationError>>()?;
        Ok(TranslationResult {
            title: translated_title,
            segments,
            provider_label: provider_label.ok_or_else(upstream)?,
            detected_source_locale: None,
            target_locale: config.default_target_locale.clone(),
        })
    }

    pub async fn lookup(
        &self,
        user_id: &str,
        query: &str,
    ) -> Result<TranslationLookupResult, TranslationError> {
        let query = normalize_lookup_query(query)?;
        let config = self.enabled_config(user_id).await?;
        match config.engine {
            TranslationEngine::OpenAi => {
                let provider_id =
                    config.open_ai.provider_id.clone().ok_or_else(|| {
                        TranslationError::new(TranslationErrorKind::NotConfigured)
                    })?;
                let output = self
                    .openai
                    .lookup(OpenAiLookupInput {
                        user_id: user_id.to_owned(),
                        provider_id,
                        text: query.clone(),
                        target_locale: config.default_target_locale.clone(),
                        max_output_tokens: config.open_ai.max_output_tokens,
                    })
                    .await?;
                Ok(TranslationLookupResult {
                    query,
                    translation: output.translation,
                    definition: output.definition,
                    examples: output.examples,
                    provider_label: output.provider_label,
                    detected_source_locale: None,
                    target_locale: config.default_target_locale,
                })
            }
            TranslationEngine::DeepLx => {
                let api_key = self.repository.deeplx_api_key(user_id).await?;
                if requires_url_api_key(config.deeplx.base_url.as_deref()) && api_key.is_none() {
                    return Err(TranslationError::new(TranslationErrorKind::NotConfigured));
                }
                let translated = self
                    .deeplx
                    .translate(DeepLxTranslateInput {
                        base_url: config.deeplx.base_url.clone(),
                        api_key,
                        text: query.clone(),
                        target_locale: config.default_target_locale.clone(),
                    })
                    .await?;
                Ok(TranslationLookupResult {
                    query,
                    translation: translated.text,
                    definition: None,
                    examples: Vec::new(),
                    provider_label: config.deeplx.display_name,
                    detected_source_locale: translated.detected_source_locale,
                    target_locale: config.default_target_locale,
                })
            }
        }
    }

    pub async fn translate_text(
        &self,
        user_id: &str,
        text: &str,
    ) -> Result<TranslationTextResult, TranslationError> {
        translation_with_timeout(
            ENTRY_TRANSLATION_TIMEOUT,
            self.translate_text_inner(user_id, text),
        )
        .await
    }

    async fn translate_text_inner(
        &self,
        user_id: &str,
        text: &str,
    ) -> Result<TranslationTextResult, TranslationError> {
        let text = normalize_selection_text(text)?;
        let config = self.enabled_config(user_id).await?;
        match config.engine {
            TranslationEngine::OpenAi => {
                let provider_id =
                    config.open_ai.provider_id.clone().ok_or_else(|| {
                        TranslationError::new(TranslationErrorKind::NotConfigured)
                    })?;
                let output = self
                    .openai
                    .translate(OpenAiTranslateInput {
                        user_id: user_id.to_owned(),
                        provider_id,
                        text,
                        target_locale: config.default_target_locale.clone(),
                        max_output_tokens: config.open_ai.max_output_tokens,
                        profile: config.open_ai.profile,
                        custom_system_prompt: config.open_ai.custom_system_prompt,
                        custom_prompt: config.open_ai.custom_prompt,
                    })
                    .await?;
                Ok(TranslationTextResult {
                    translated_text: output.text,
                    provider_label: output.provider_label,
                    detected_source_locale: None,
                    target_locale: config.default_target_locale,
                })
            }
            TranslationEngine::DeepLx => {
                let api_key = self.repository.deeplx_api_key(user_id).await?;
                if requires_url_api_key(config.deeplx.base_url.as_deref()) && api_key.is_none() {
                    return Err(TranslationError::new(TranslationErrorKind::NotConfigured));
                }
                let maximum = if api_key.is_some() {
                    AUTHENTICATED_CHUNK_CHARACTERS
                } else {
                    ANONYMOUS_CHUNK_CHARACTERS
                };
                let output = self
                    .translate_deeplx_text(&config, api_key.as_ref(), text, maximum)
                    .await?;
                Ok(TranslationTextResult {
                    translated_text: output.text,
                    provider_label: output.provider_label,
                    detected_source_locale: output.detected_source_locale,
                    target_locale: config.default_target_locale,
                })
            }
        }
    }

    async fn enabled_config(&self, user_id: &str) -> Result<TranslationConfig, TranslationError> {
        let config = self.repository.get(user_id).await?;
        if config.revision.is_none() {
            return Err(TranslationError::new(TranslationErrorKind::NotConfigured));
        }
        if !config.is_enabled {
            return Err(TranslationError::new(TranslationErrorKind::Disabled));
        }
        if config.engine == TranslationEngine::OpenAi {
            let provider_id = config
                .open_ai
                .provider_id
                .as_deref()
                .ok_or_else(|| TranslationError::new(TranslationErrorKind::NotConfigured))?;
            self.validate_openai_provider(user_id, provider_id, true)
                .await?;
        }
        Ok(config)
    }

    async fn validate_openai_provider(
        &self,
        user_id: &str,
        provider_id: &str,
        require_enabled: bool,
    ) -> Result<(), TranslationError> {
        let provider = self
            .providers
            .get_visible_for_user(provider_id, user_id)
            .await
            .map_err(map_provider_error)?;
        if !matches!(
            provider.kind(),
            ProviderKind::OpenAiResponses | ProviderKind::OpenAiChatCompletions
        ) || (require_enabled && !provider.is_enabled())
        {
            return Err(TranslationError::new(
                TranslationErrorKind::ProviderUnavailable,
            ));
        }
        Ok(())
    }

    async fn resolve_test_api_key(
        &self,
        user_id: &str,
        update: ApiKeyUpdate,
    ) -> Result<Option<SecretString>, TranslationError> {
        match update {
            ApiKeyUpdate::Keep => self.repository.deeplx_api_key(user_id).await,
            ApiKeyUpdate::Set(api_key) => {
                validate_api_key(&api_key)?;
                Ok(Some(api_key))
            }
            ApiKeyUpdate::Clear => Ok(None),
        }
    }

    async fn translate_deeplx_text(
        &self,
        config: &TranslationConfig,
        api_key: Option<&SecretString>,
        text: String,
        maximum: usize,
    ) -> Result<TranslatedText, TranslationError> {
        let chunks = chunk_text(&text, maximum);
        if chunks.is_empty() {
            return Err(invalid_input());
        }
        let mut translated_chunks = Vec::with_capacity(chunks.len());
        let mut detected_source_locale = None;
        for chunk in chunks {
            let translated = self
                .deeplx
                .translate(DeepLxTranslateInput {
                    base_url: config.deeplx.base_url.clone(),
                    api_key: api_key.cloned(),
                    text: chunk,
                    target_locale: config.default_target_locale.clone(),
                })
                .await?;
            if detected_source_locale.is_none() {
                detected_source_locale = translated.detected_source_locale;
            }
            translated_chunks.push(translated.text);
        }
        Ok(TranslatedText {
            text: translated_chunks.join(" "),
            provider_label: config.deeplx.display_name.clone(),
            detected_source_locale,
        })
    }
}

struct TranslatedText {
    text: String,
    provider_label: String,
    detected_source_locale: Option<String>,
}

fn normalize_target_locale(value: &str) -> Result<String, TranslationError> {
    normalize_locale(value.trim(), PluginRegistryErrorKind::InvalidConfig)
        .map_err(|_| invalid_input())
}

fn normalize_lookup_query(value: &str) -> Result<String, TranslationError> {
    let value = value.trim();
    if value.is_empty()
        || value.chars().count() > MAX_LOOKUP_CHARACTERS
        || value
            .chars()
            .any(|character| character.is_control() && !matches!(character, '\n' | '\r' | '\t'))
    {
        return Err(invalid_input());
    }
    Ok(value.to_owned())
}

fn normalize_selection_text(value: &str) -> Result<String, TranslationError> {
    normalize_bounded_text(value, MAX_SELECTION_CHARACTERS)
}

fn normalize_bounded_text(
    value: &str,
    maximum_characters: usize,
) -> Result<String, TranslationError> {
    let value = value.trim();
    if value.is_empty()
        || value.chars().count() > maximum_characters
        || value
            .chars()
            .any(|character| character.is_control() && !matches!(character, '\n' | '\r' | '\t'))
    {
        return Err(invalid_input());
    }
    Ok(value.to_owned())
}

fn chunk_text(value: &str, maximum_characters: usize) -> Vec<String> {
    let mut chunks = Vec::new();
    let mut current = String::new();
    for word in value.split_whitespace() {
        let separator = usize::from(!current.is_empty());
        if current.chars().count() + separator + word.chars().count() > maximum_characters {
            if !current.is_empty() {
                chunks.push(std::mem::take(&mut current));
            }
            if word.chars().count() > maximum_characters {
                let mut split = String::new();
                for character in word.chars() {
                    split.push(character);
                    if split.chars().count() == maximum_characters {
                        chunks.push(std::mem::take(&mut split));
                    }
                }
                current = split;
                continue;
            }
        }
        if !current.is_empty() {
            current.push(' ');
        }
        current.push_str(word);
    }
    if !current.is_empty() {
        chunks.push(current);
    }
    chunks
}

fn deeplx_article_call_count(title: &str, segments: &[String], maximum_characters: usize) -> usize {
    std::iter::once(title)
        .chain(segments.iter().map(String::as_str))
        .map(|text| chunk_text(text, maximum_characters).len())
        .sum()
}

async fn translation_with_timeout<T, F>(
    duration: Duration,
    future: F,
) -> Result<T, TranslationError>
where
    F: Future<Output = Result<T, TranslationError>>,
{
    timeout(duration, future)
        .await
        .map_err(|_| TranslationError::new(TranslationErrorKind::Timeout))?
}

fn batch_source_segments(
    segments: Vec<OpenAiSourceSegment>,
    maximum_characters: usize,
    maximum_segments: usize,
) -> Vec<Vec<OpenAiSourceSegment>> {
    let mut batches = Vec::new();
    let mut current = Vec::new();
    let mut current_characters: usize = 0;
    for segment in segments {
        let segment_characters = segment.text.chars().count();
        if !current.is_empty()
            && (current.len() >= maximum_segments
                || current_characters.saturating_add(segment_characters) > maximum_characters)
        {
            batches.push(std::mem::take(&mut current));
            current_characters = 0;
        }
        current_characters = current_characters.saturating_add(segment_characters);
        current.push(segment);
    }
    if !current.is_empty() {
        batches.push(current);
    }
    batches
}

fn map_provider_error(error: crate::content::provider::ProviderCoreError) -> TranslationError {
    let kind = match error.kind() {
        ProviderCoreErrorKind::SecretUnavailable => TranslationErrorKind::KeyringUnavailable,
        ProviderCoreErrorKind::Database => TranslationErrorKind::Database,
        ProviderCoreErrorKind::NotFound
        | ProviderCoreErrorKind::ProviderDisabled
        | ProviderCoreErrorKind::InvalidProviderId => TranslationErrorKind::ProviderUnavailable,
        _ => TranslationErrorKind::CorruptData,
    };
    TranslationError::new(kind)
}

fn map_feed_error(error: RepositoryError) -> TranslationError {
    match error {
        RepositoryError::InvalidEntryId | RepositoryError::InvalidUserId => {
            TranslationError::new(TranslationErrorKind::NotFound)
        }
        _ => TranslationError::new(TranslationErrorKind::Database),
    }
}

const fn invalid_input() -> TranslationError {
    TranslationError::new(TranslationErrorKind::InvalidInput)
}

const fn upstream() -> TranslationError {
    TranslationError::new(TranslationErrorKind::Upstream)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunking_respects_character_limits_and_keeps_all_words() {
        let chunks = chunk_text("one two three four five", 8);
        assert_eq!(chunks, ["one two", "three", "four", "five"]);
        assert!(chunks.iter().all(|chunk| chunk.chars().count() <= 8));
    }

    #[test]
    fn lookup_input_is_bounded() {
        assert_eq!(normalize_lookup_query(" fox ").unwrap(), "fox");
        assert!(normalize_lookup_query("").is_err());
        assert!(normalize_lookup_query(&"x".repeat(201)).is_err());
    }

    #[test]
    fn selected_text_is_trimmed_and_bounded() {
        assert_eq!(
            normalize_selection_text("  Selected paragraph.  ").unwrap(),
            "Selected paragraph."
        );
        assert!(normalize_selection_text("").is_err());
        assert!(normalize_selection_text("unsafe\u{0007}text").is_err());
        assert!(normalize_selection_text(&"x".repeat(MAX_SELECTION_CHARACTERS + 1)).is_err());
    }

    #[test]
    fn openai_batches_preserve_ids_and_bound_request_size() {
        let batches = batch_source_segments(
            vec![
                OpenAiSourceSegment {
                    id: 0,
                    text: "title".to_owned(),
                },
                OpenAiSourceSegment {
                    id: 1,
                    text: "12345".to_owned(),
                },
                OpenAiSourceSegment {
                    id: 2,
                    text: "67890".to_owned(),
                },
            ],
            8,
            2,
        );

        assert_eq!(batches.len(), 3);
        assert_eq!(batches[0][0].id, 0);
        assert_eq!(batches[1][0].id, 1);
        assert_eq!(batches[2][0].id, 2);
    }

    #[test]
    fn deeplx_article_call_budget_counts_title_segments_and_chunks() {
        let segments = vec!["short".to_owned(); MAX_DEEPLX_ARTICLE_CALLS - 1];
        assert_eq!(
            deeplx_article_call_count("title", &segments, 10),
            MAX_DEEPLX_ARTICLE_CALLS
        );
        let oversized = vec!["short".to_owned(); MAX_DEEPLX_ARTICLE_CALLS];
        assert!(deeplx_article_call_count("title", &oversized, 10) > MAX_DEEPLX_ARTICLE_CALLS);
        assert_eq!(deeplx_article_call_count("123456", &[], 3), 2);
    }

    #[tokio::test(start_paused = true)]
    async fn entry_translation_deadline_returns_a_typed_timeout() {
        let future = std::future::pending::<Result<(), TranslationError>>();
        let task = tokio::spawn(translation_with_timeout(Duration::from_secs(1), future));
        tokio::time::advance(Duration::from_secs(1)).await;
        let error = task
            .await
            .expect("deadline task should join")
            .expect_err("pending translation should time out");
        assert_eq!(error.kind(), TranslationErrorKind::Timeout);
    }
}
