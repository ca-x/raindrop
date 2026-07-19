use std::{error::Error, fmt};

use serde_json::{Value, json};
use url::Url;

use crate::{
    content::jobs::{ContentExecutionEntry, ContentJobOperation},
    plugins::{
        PluginRegistryErrorKind,
        json::{
            canonical_json, contextual_hash, normalize_locale, validate_lower_hex_hash,
            validate_text, validate_uuid,
        },
        runtime::bindings::types,
    },
};

const INPUT_HASH_CONTEXT: &str = "raindrop.content-invocation-input.v1";
const MCP_PROVENANCE_HASH_CONTEXT: &str = "raindrop.content-mcp-provenance.v1";
const MAX_CANONICAL_INPUT_BYTES: usize = 1024 * 1024;
const MAX_TITLE_BYTES: usize = 64 * 1024;
const MAX_TEXT_BYTES: usize = 512 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ContentInvocationError {
    InvalidInput,
    InputTooLarge,
}

impl fmt::Display for ContentInvocationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidInput => "content invocation input is invalid",
            Self::InputTooLarge => "content invocation input is too large",
        })
    }
}

impl Error for ContentInvocationError {}

#[derive(Clone)]
pub struct ContentInvocationInput {
    entry: types::EntryReference,
    operation: ContentJobOperation,
    target_locale: Option<String>,
    canonical_json: String,
    hash: String,
}

impl ContentInvocationInput {
    pub fn new(
        entry: &ContentExecutionEntry,
        operation: ContentJobOperation,
        target_locale: Option<&str>,
    ) -> Result<Self, ContentInvocationError> {
        Self::from_parts(
            entry.entry_id(),
            entry.feed_id(),
            entry.content_hash(),
            entry.title().unwrap_or_default(),
            entry.text(),
            entry.canonical_url(),
            None,
            operation,
            target_locale,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn from_parts(
        entry_id: &str,
        feed_id: &str,
        content_hash: &str,
        title: &str,
        text: &str,
        canonical_url: Option<&str>,
        source_locale: Option<&str>,
        operation: ContentJobOperation,
        target_locale: Option<&str>,
    ) -> Result<Self, ContentInvocationError> {
        validate_uuid(entry_id, PluginRegistryErrorKind::InvalidInput)
            .map_err(|_| ContentInvocationError::InvalidInput)?;
        validate_uuid(feed_id, PluginRegistryErrorKind::InvalidInput)
            .map_err(|_| ContentInvocationError::InvalidInput)?;
        validate_lower_hex_hash(content_hash, PluginRegistryErrorKind::InvalidInput)
            .map_err(|_| ContentInvocationError::InvalidInput)?;
        if !valid_title(title)
            || validate_text(text, MAX_TEXT_BYTES, PluginRegistryErrorKind::InvalidInput).is_err()
            || !valid_url(canonical_url)
        {
            return Err(
                if text.len() > MAX_TEXT_BYTES || title.len() > MAX_TITLE_BYTES {
                    ContentInvocationError::InputTooLarge
                } else {
                    ContentInvocationError::InvalidInput
                },
            );
        }
        let source_locale = normalize_optional_locale(source_locale)?;
        let target_locale = normalize_target_locale(operation, target_locale)?;
        let entry = types::EntryReference {
            entry_id: entry_id.to_owned(),
            feed_id: feed_id.to_owned(),
            content_hash: content_hash.to_owned(),
            title: title.to_owned(),
            text: text.to_owned(),
            canonical_url: canonical_url.map(str::to_owned),
            source_locale,
        };
        let value = json!({
            "schemaVersion": 1,
            "operation": operation_name(operation),
            "targetLocale": target_locale.clone(),
            "entry": {
                "entryId": &entry.entry_id,
                "feedId": &entry.feed_id,
                "contentHash": &entry.content_hash,
                "title": &entry.title,
                "text": &entry.text,
                "canonicalUrl": &entry.canonical_url,
                "sourceLocale": &entry.source_locale,
            },
        });
        let canonical_json = canonical_json(value, MAX_CANONICAL_INPUT_BYTES).map_err(|error| {
            if error.kind() == PluginRegistryErrorKind::PayloadTooLarge {
                ContentInvocationError::InputTooLarge
            } else {
                ContentInvocationError::InvalidInput
            }
        })?;
        let hash = contextual_hash(INPUT_HASH_CONTEXT, canonical_json.as_bytes());
        Ok(Self {
            entry,
            operation,
            target_locale,
            canonical_json,
            hash,
        })
    }

    #[must_use]
    pub fn entry(&self) -> &types::EntryReference {
        &self.entry
    }

    #[must_use]
    pub const fn operation(&self) -> ContentJobOperation {
        self.operation
    }

    #[must_use]
    pub fn target_locale(&self) -> Option<&str> {
        self.target_locale.as_deref()
    }

    #[must_use]
    pub fn canonical_json(&self) -> &str {
        &self.canonical_json
    }

    #[must_use]
    pub fn hash(&self) -> &str {
        &self.hash
    }

    #[must_use]
    pub fn to_wit_entry(&self) -> types::EntryReference {
        self.entry.clone()
    }
}

impl fmt::Debug for ContentInvocationInput {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ContentInvocationInput")
            .field("operation", &self.operation)
            .field("target_locale", &self.target_locale)
            .field("canonical_bytes", &self.canonical_json.len())
            .finish_non_exhaustive()
    }
}

#[must_use]
pub fn disabled_mcp_provenance_hash() -> String {
    let value: Value = json!({"schemaVersion": 1, "mode": "DISABLED"});
    let canonical = canonical_json(value, 128).expect("fixed MCP provenance must remain canonical");
    contextual_hash(MCP_PROVENANCE_HASH_CONTEXT, canonical.as_bytes())
}

const fn operation_name(operation: ContentJobOperation) -> &'static str {
    match operation {
        ContentJobOperation::Summarize => "SUMMARIZE",
        ContentJobOperation::Translate => "TRANSLATE",
    }
}

fn normalize_target_locale(
    operation: ContentJobOperation,
    target_locale: Option<&str>,
) -> Result<Option<String>, ContentInvocationError> {
    match (operation, target_locale) {
        (ContentJobOperation::Summarize, None) => Ok(None),
        (ContentJobOperation::Translate, Some(locale)) => {
            normalize_locale(locale, PluginRegistryErrorKind::InvalidInput)
                .map(Some)
                .map_err(|_| ContentInvocationError::InvalidInput)
        }
        (ContentJobOperation::Summarize, Some(_)) | (ContentJobOperation::Translate, None) => {
            Err(ContentInvocationError::InvalidInput)
        }
    }
}

fn normalize_optional_locale(
    value: Option<&str>,
) -> Result<Option<String>, ContentInvocationError> {
    value
        .map(|locale| normalize_locale(locale, PluginRegistryErrorKind::InvalidInput))
        .transpose()
        .map_err(|_| ContentInvocationError::InvalidInput)
}

fn valid_title(value: &str) -> bool {
    value.len() <= MAX_TITLE_BYTES
        && !value
            .chars()
            .any(|character| character.is_control() && !matches!(character, '\n' | '\r' | '\t'))
}

fn valid_url(value: Option<&str>) -> bool {
    value.is_none_or(|value| {
        Url::parse(value).is_ok_and(|url| {
            matches!(url.scheme(), "http" | "https")
                && url.username().is_empty()
                && url.password().is_none()
                && url.host_str().is_some()
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const ENTRY_ID: &str = "00000000-0000-4000-8000-000000000101";
    const FEED_ID: &str = "00000000-0000-4000-8000-000000000201";
    const HASH: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

    #[test]
    fn canonical_input_and_disabled_mcp_hash_are_frozen() {
        let input = ContentInvocationInput::from_parts(
            ENTRY_ID,
            FEED_ID,
            HASH,
            "A title",
            "Untrusted text",
            Some("https://example.test/article"),
            Some("en-us"),
            ContentJobOperation::Translate,
            Some("zh-cn"),
        )
        .expect("valid invocation input");
        assert_eq!(
            input.canonical_json(),
            r#"{"entry":{"canonicalUrl":"https://example.test/article","contentHash":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","entryId":"00000000-0000-4000-8000-000000000101","feedId":"00000000-0000-4000-8000-000000000201","sourceLocale":"en-US","text":"Untrusted text","title":"A title"},"operation":"TRANSLATE","schemaVersion":1,"targetLocale":"zh-CN"}"#,
        );
        assert_eq!(
            input.hash(),
            "6d6fa9b5398cc9cf1e96e256b03fd153e443ed6c936ea95b59897fedab66929e"
        );
        assert_eq!(
            disabled_mcp_provenance_hash(),
            "11c3288cf0a19350a655b8789dd653bbf1bcecdb171660e8409e5163615ec71e"
        );
    }

    #[test]
    fn operation_locale_and_size_contracts_fail_closed() {
        assert!(
            ContentInvocationInput::from_parts(
                ENTRY_ID,
                FEED_ID,
                HASH,
                "",
                "text",
                None,
                None,
                ContentJobOperation::Summarize,
                None,
            )
            .is_ok()
        );
        assert!(
            ContentInvocationInput::from_parts(
                ENTRY_ID,
                FEED_ID,
                HASH,
                "title",
                "text",
                None,
                None,
                ContentJobOperation::Translate,
                None,
            )
            .is_err()
        );
        assert_eq!(
            ContentInvocationInput::from_parts(
                ENTRY_ID,
                FEED_ID,
                HASH,
                "title",
                &"x".repeat(MAX_TEXT_BYTES + 1),
                None,
                None,
                ContentJobOperation::Summarize,
                None,
            )
            .unwrap_err(),
            ContentInvocationError::InputTooLarge
        );
    }
}
