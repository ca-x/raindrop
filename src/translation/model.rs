use std::fmt;

use secrecy::SecretString;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TranslationEngine {
    OpenAi,
    DeepLx,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TranslationDisplayMode {
    TranslationOnly,
    Bilingual,
    Hover,
    SideBySide,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AiTranslationProfile {
    General,
    Technical,
    Literary,
    Academic,
    Business,
    SocialNews,
    Custom,
}

impl AiTranslationProfile {
    #[must_use]
    pub const fn as_storage(self) -> &'static str {
        match self {
            Self::General => "GENERAL",
            Self::Technical => "TECHNICAL",
            Self::Literary => "LITERARY",
            Self::Academic => "ACADEMIC",
            Self::Business => "BUSINESS",
            Self::SocialNews => "SOCIAL_NEWS",
            Self::Custom => "CUSTOM",
        }
    }

    pub(crate) fn parse(value: &str) -> Result<Self, TranslationError> {
        match value {
            "GENERAL" => Ok(Self::General),
            "TECHNICAL" => Ok(Self::Technical),
            "LITERARY" => Ok(Self::Literary),
            "ACADEMIC" => Ok(Self::Academic),
            "BUSINESS" => Ok(Self::Business),
            "SOCIAL_NEWS" => Ok(Self::SocialNews),
            "CUSTOM" => Ok(Self::Custom),
            _ => Err(TranslationError::new(TranslationErrorKind::InvalidInput)),
        }
    }
}

impl TranslationDisplayMode {
    #[must_use]
    pub const fn as_storage(self) -> &'static str {
        match self {
            Self::TranslationOnly => "TRANSLATION_ONLY",
            Self::Bilingual => "BILINGUAL",
            Self::Hover => "HOVER",
            Self::SideBySide => "SIDE_BY_SIDE",
        }
    }

    pub(crate) fn parse(value: &str) -> Result<Self, TranslationError> {
        match value {
            "TRANSLATION_ONLY" => Ok(Self::TranslationOnly),
            "BILINGUAL" => Ok(Self::Bilingual),
            "HOVER" => Ok(Self::Hover),
            "SIDE_BY_SIDE" => Ok(Self::SideBySide),
            _ => Err(TranslationError::new(TranslationErrorKind::InvalidInput)),
        }
    }
}

impl TranslationEngine {
    #[must_use]
    pub const fn as_storage(self) -> &'static str {
        match self {
            Self::OpenAi => "OPENAI",
            Self::DeepLx => "DEEPLX",
        }
    }

    pub(crate) fn parse(value: &str) -> Result<Self, TranslationError> {
        match value {
            "OPENAI" => Ok(Self::OpenAi),
            "DEEPLX" => Ok(Self::DeepLx),
            _ => Err(TranslationError::new(TranslationErrorKind::InvalidInput)),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OpenAiSettings {
    pub provider_id: Option<String>,
    pub max_output_tokens: u32,
    pub profile: AiTranslationProfile,
    pub custom_system_prompt: Option<String>,
    pub custom_prompt: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DeepLxSettings {
    pub display_name: String,
    pub description: Option<String>,
    pub base_url: Option<String>,
    pub has_api_key: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TranslationConfig {
    pub user_id: String,
    pub engine: TranslationEngine,
    pub display_mode: TranslationDisplayMode,
    pub is_enabled: bool,
    pub default_target_locale: String,
    pub open_ai: OpenAiSettings,
    pub deeplx: DeepLxSettings,
    pub revision: Option<u64>,
}

pub enum ApiKeyUpdate {
    Keep,
    Set(SecretString),
    Clear,
}

impl fmt::Debug for ApiKeyUpdate {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Keep => "Keep",
            Self::Set(_) => "Set([REDACTED])",
            Self::Clear => "Clear",
        })
    }
}

#[derive(Debug)]
pub struct SaveTranslationConfig {
    pub expected_revision: Option<u64>,
    pub engine: TranslationEngine,
    pub display_mode: TranslationDisplayMode,
    pub is_enabled: bool,
    pub default_target_locale: String,
    pub open_ai_provider_id: Option<String>,
    pub open_ai_max_output_tokens: u32,
    pub open_ai_profile: AiTranslationProfile,
    pub open_ai_custom_system_prompt: Option<String>,
    pub open_ai_custom_prompt: Option<String>,
    pub deeplx_display_name: String,
    pub deeplx_description: Option<String>,
    pub deeplx_base_url: Option<String>,
    pub deeplx_api_key: ApiKeyUpdate,
}

#[derive(Debug)]
pub struct DeepLxDraft {
    pub base_url: Option<String>,
    pub api_key: ApiKeyUpdate,
}

#[derive(Debug)]
pub struct TestTranslationInput {
    pub engine: TranslationEngine,
    pub open_ai_provider_id: Option<String>,
    pub open_ai_max_output_tokens: u32,
    pub open_ai_profile: AiTranslationProfile,
    pub open_ai_custom_system_prompt: Option<String>,
    pub open_ai_custom_prompt: Option<String>,
    pub deeplx: DeepLxDraft,
    pub target_locale: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TranslationTestResult {
    pub translated_text: String,
    pub provider_label: String,
    pub detected_source_locale: Option<String>,
    pub target_locale: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TranslationResult {
    pub title: String,
    pub segments: Vec<TranslationSegment>,
    pub provider_label: String,
    pub detected_source_locale: Option<String>,
    pub target_locale: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TranslationSegment {
    pub index: u32,
    pub original_text: String,
    pub translated_text: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LookupExample {
    pub source: String,
    pub target: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TranslationLookupResult {
    pub query: String,
    pub translation: String,
    pub definition: Option<String>,
    pub examples: Vec<LookupExample>,
    pub provider_label: String,
    pub detected_source_locale: Option<String>,
    pub target_locale: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TranslationErrorKind {
    InvalidInput,
    NotConfigured,
    Disabled,
    ProviderUnavailable,
    KeyringUnavailable,
    RevisionConflict,
    NotFound,
    TooLarge,
    RateLimited,
    Timeout,
    Upstream,
    CorruptData,
    Database,
}

pub struct TranslationError {
    kind: TranslationErrorKind,
}

impl TranslationError {
    #[must_use]
    pub const fn new(kind: TranslationErrorKind) -> Self {
        Self { kind }
    }

    #[must_use]
    pub const fn kind(&self) -> TranslationErrorKind {
        self.kind
    }
}

impl fmt::Debug for TranslationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TranslationError")
            .field("kind", &self.kind)
            .finish()
    }
}

impl fmt::Display for TranslationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self.kind {
            TranslationErrorKind::InvalidInput => "translation configuration is invalid",
            TranslationErrorKind::NotConfigured => "translation is not configured",
            TranslationErrorKind::Disabled => "translation is disabled",
            TranslationErrorKind::ProviderUnavailable => "translation provider is unavailable",
            TranslationErrorKind::KeyringUnavailable => "translation credential is unavailable",
            TranslationErrorKind::RevisionConflict => "translation settings changed",
            TranslationErrorKind::NotFound => "translation source entry was not found",
            TranslationErrorKind::TooLarge => "translation input is too large",
            TranslationErrorKind::RateLimited => "translation provider is rate limiting requests",
            TranslationErrorKind::Timeout => "translation request timed out",
            TranslationErrorKind::Upstream => "translation request failed",
            TranslationErrorKind::CorruptData => "translation stored data is invalid",
            TranslationErrorKind::Database => "translation database operation failed",
        })
    }
}

impl std::error::Error for TranslationError {}
