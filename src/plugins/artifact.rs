use serde::{Deserialize, Serialize};

use super::{
    PluginRegistryError, PluginRegistryErrorKind,
    json::{canonical_json, normalize_locale, parse_unique_json, validate_text},
};

const MAX_ARTIFACT_BYTES: usize = 512 * 1024;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SummaryArtifact {
    document: SummaryDocument,
    canonical_json: String,
}

impl SummaryArtifact {
    pub fn parse(input: &[u8]) -> Result<Self, PluginRegistryError> {
        let value = parse_unique_json(input, MAX_ARTIFACT_BYTES)?;
        let mut document = serde_json::from_value::<SummaryDocument>(value)
            .map_err(|_| PluginRegistryError::new(PluginRegistryErrorKind::InvalidArtifact))?;
        document.validate()?;
        document.source_language = normalize_locale(
            &document.source_language,
            PluginRegistryErrorKind::InvalidArtifact,
        )?;
        let normalized = serde_json::to_value(&document)
            .map_err(|_| PluginRegistryError::new(PluginRegistryErrorKind::InvalidArtifact))?;
        let canonical_json = canonical_json(normalized, MAX_ARTIFACT_BYTES)?;
        Ok(Self {
            document,
            canonical_json,
        })
    }

    #[must_use]
    pub fn source_language(&self) -> &str {
        &self.document.source_language
    }

    #[must_use]
    pub fn summary(&self) -> &str {
        &self.document.summary
    }

    #[must_use]
    pub fn bullets(&self) -> &[String] {
        &self.document.bullets
    }

    #[must_use]
    pub fn conclusion(&self) -> Option<&str> {
        self.document.conclusion.as_deref()
    }

    #[must_use]
    pub fn canonical_json(&self) -> &str {
        &self.canonical_json
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct SummaryDocument {
    schema_version: u32,
    source_language: String,
    summary: String,
    bullets: Vec<String>,
    conclusion: Option<String>,
}

impl SummaryDocument {
    fn validate(&self) -> Result<(), PluginRegistryError> {
        if self.schema_version != 1 || self.bullets.len() > 8 {
            return invalid();
        }
        normalize_locale(
            &self.source_language,
            PluginRegistryErrorKind::InvalidArtifact,
        )?;
        validate_safe_markdown(&self.summary, 64 * 1024)?;
        for bullet in &self.bullets {
            validate_safe_markdown(bullet, 8 * 1024)?;
        }
        if let Some(conclusion) = &self.conclusion {
            validate_safe_markdown(conclusion, 16 * 1024)?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TranslationArtifact {
    document: TranslationDocument,
    canonical_json: String,
}

impl TranslationArtifact {
    pub fn parse(input: &[u8]) -> Result<Self, PluginRegistryError> {
        let value = parse_unique_json(input, MAX_ARTIFACT_BYTES)?;
        let mut document = serde_json::from_value::<TranslationDocument>(value)
            .map_err(|_| PluginRegistryError::new(PluginRegistryErrorKind::InvalidArtifact))?;
        document.validate()?;
        document.detected_source_language = normalize_locale(
            &document.detected_source_language,
            PluginRegistryErrorKind::InvalidArtifact,
        )?;
        document.target_locale = normalize_locale(
            &document.target_locale,
            PluginRegistryErrorKind::InvalidArtifact,
        )?;
        let normalized = serde_json::to_value(&document)
            .map_err(|_| PluginRegistryError::new(PluginRegistryErrorKind::InvalidArtifact))?;
        let canonical_json = canonical_json(normalized, MAX_ARTIFACT_BYTES)?;
        Ok(Self {
            document,
            canonical_json,
        })
    }

    #[must_use]
    pub fn detected_source_language(&self) -> &str {
        &self.document.detected_source_language
    }

    #[must_use]
    pub fn target_locale(&self) -> &str {
        &self.document.target_locale
    }

    #[must_use]
    pub fn title(&self) -> &str {
        &self.document.title
    }

    #[must_use]
    pub fn body_markdown(&self) -> &str {
        &self.document.body_markdown
    }

    #[must_use]
    pub fn canonical_json(&self) -> &str {
        &self.canonical_json
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct TranslationDocument {
    schema_version: u32,
    detected_source_language: String,
    target_locale: String,
    title: String,
    body_markdown: String,
}

impl TranslationDocument {
    fn validate(&self) -> Result<(), PluginRegistryError> {
        if self.schema_version != 1 {
            return invalid();
        }
        normalize_locale(
            &self.detected_source_language,
            PluginRegistryErrorKind::InvalidArtifact,
        )?;
        normalize_locale(
            &self.target_locale,
            PluginRegistryErrorKind::InvalidArtifact,
        )?;
        validate_safe_markdown(&self.title, 16 * 1024)?;
        validate_safe_markdown(&self.body_markdown, 256 * 1024)
    }
}

fn validate_safe_markdown(value: &str, max_bytes: usize) -> Result<(), PluginRegistryError> {
    validate_text(value, max_bytes, PluginRegistryErrorKind::InvalidArtifact)?;
    let lower = value.to_ascii_lowercase();
    if value.contains(['<', '>'])
        || ["javascript:", "data:", "vbscript:"]
            .iter()
            .any(|scheme| lower.contains(scheme))
    {
        return invalid();
    }
    Ok(())
}

fn invalid<T>() -> Result<T, PluginRegistryError> {
    Err(PluginRegistryError::new(
        PluginRegistryErrorKind::InvalidArtifact,
    ))
}
