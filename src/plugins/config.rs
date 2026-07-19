use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use super::{
    PluginRegistryError, PluginRegistryErrorKind,
    json::{
        canonical_json, contextual_hash, normalize_locale, parse_unique_json, validate_uuid,
        validate_visible_ascii,
    },
};

const MAX_CONFIG_BYTES: usize = 256 * 1024;
const CONFIG_HASH_CONTEXT: &str = "raindrop.plugin-config.v1";

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AiContentConfig {
    document: ConfigDocument,
    canonical_json: String,
    config_hash: String,
}

impl AiContentConfig {
    pub fn parse(input: &[u8]) -> Result<Self, PluginRegistryError> {
        let value = parse_unique_json(input, MAX_CONFIG_BYTES)?;
        let mut document = serde_json::from_value::<ConfigDocument>(value)
            .map_err(|_| PluginRegistryError::new(PluginRegistryErrorKind::InvalidConfig))?;
        document.validate()?;
        document.operations.translate.default_target_locale = normalize_locale(
            &document.operations.translate.default_target_locale,
            PluginRegistryErrorKind::InvalidConfig,
        )?;
        let normalized = serde_json::to_value(&document)
            .map_err(|_| PluginRegistryError::new(PluginRegistryErrorKind::InvalidConfig))?;
        let canonical_json = canonical_json(normalized, MAX_CONFIG_BYTES)?;
        let config_hash = contextual_hash(CONFIG_HASH_CONTEXT, canonical_json.as_bytes());
        Ok(Self {
            document,
            canonical_json,
            config_hash,
        })
    }

    #[must_use]
    pub const fn schema_version(&self) -> u32 {
        self.document.schema_version
    }

    #[must_use]
    pub fn summarize_provider_id(&self) -> &str {
        &self.document.operations.summarize.provider_id
    }

    #[must_use]
    pub const fn summarize_enabled(&self) -> bool {
        self.document.operations.summarize.enabled
    }

    #[must_use]
    pub const fn summarize_style(&self) -> AiSummaryStyle {
        self.document.operations.summarize.style
    }

    #[must_use]
    pub const fn summarize_max_output_tokens(&self) -> u32 {
        self.document.operations.summarize.max_output_tokens
    }

    #[must_use]
    pub const fn summarize_mcp_enabled(&self) -> bool {
        matches!(
            self.document.operations.summarize.mcp.mode,
            McpMode::ContextEnrichment
        )
    }

    #[must_use]
    pub fn translate_provider_id(&self) -> &str {
        &self.document.operations.translate.provider_id
    }

    #[must_use]
    pub const fn translate_enabled(&self) -> bool {
        self.document.operations.translate.enabled
    }

    #[must_use]
    pub const fn translate_max_output_tokens(&self) -> u32 {
        self.document.operations.translate.max_output_tokens
    }

    #[must_use]
    pub const fn translate_mcp_enabled(&self) -> bool {
        matches!(
            self.document.operations.translate.mcp.mode,
            McpMode::ContextEnrichment
        )
    }

    #[must_use]
    pub fn default_target_locale(&self) -> &str {
        &self.document.operations.translate.default_target_locale
    }

    #[must_use]
    pub fn canonical_json(&self) -> &str {
        &self.canonical_json
    }

    #[must_use]
    pub fn config_hash(&self) -> &str {
        &self.config_hash
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct ConfigDocument {
    schema_version: u32,
    operations: Operations,
    automatic: AutomaticConfig,
}

impl ConfigDocument {
    fn validate(&self) -> Result<(), PluginRegistryError> {
        if self.schema_version != 1 {
            return invalid();
        }
        self.operations.summarize.validate()?;
        self.operations.translate.validate()?;
        self.automatic.validate(&self.operations)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct Operations {
    summarize: SummarizeConfig,
    translate: TranslateConfig,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct SummarizeConfig {
    enabled: bool,
    provider_id: String,
    style: AiSummaryStyle,
    max_output_tokens: u32,
    mcp: McpConfig,
}

impl SummarizeConfig {
    fn validate(&self) -> Result<(), PluginRegistryError> {
        validate_uuid(&self.provider_id, PluginRegistryErrorKind::InvalidConfig)?;
        if !(128..=4096).contains(&self.max_output_tokens) {
            return invalid();
        }
        self.mcp.validate()
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum AiSummaryStyle {
    Concise,
    Balanced,
    Detailed,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct TranslateConfig {
    enabled: bool,
    provider_id: String,
    default_target_locale: String,
    max_output_tokens: u32,
    mcp: McpConfig,
}

impl TranslateConfig {
    fn validate(&self) -> Result<(), PluginRegistryError> {
        validate_uuid(&self.provider_id, PluginRegistryErrorKind::InvalidConfig)?;
        normalize_locale(
            &self.default_target_locale,
            PluginRegistryErrorKind::InvalidConfig,
        )?;
        if !(256..=16_384).contains(&self.max_output_tokens) {
            return invalid();
        }
        self.mcp.validate()
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct McpConfig {
    mode: McpMode,
    failure_policy: McpFailurePolicy,
    max_tool_calls: u8,
    tools: Vec<McpToolBinding>,
}

impl McpConfig {
    fn validate(&self) -> Result<(), PluginRegistryError> {
        if self.tools.len() > 16 || self.max_tool_calls > 4 {
            return invalid();
        }
        match self.mode {
            McpMode::Disabled if self.max_tool_calls != 0 || !self.tools.is_empty() => {
                return invalid();
            }
            McpMode::ContextEnrichment if self.max_tool_calls == 0 || self.tools.is_empty() => {
                return invalid();
            }
            McpMode::Disabled | McpMode::ContextEnrichment => {}
        }

        let mut seen = HashSet::new();
        for tool in &self.tools {
            tool.validate()?;
            if !seen.insert((tool.connection_id.as_str(), tool.tool_name.as_str())) {
                return invalid();
            }
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
enum McpMode {
    Disabled,
    ContextEnrichment,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
enum McpFailurePolicy {
    FailOpen,
    FailClosed,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct McpToolBinding {
    connection_id: String,
    tool_name: String,
}

impl McpToolBinding {
    fn validate(&self) -> Result<(), PluginRegistryError> {
        validate_uuid(&self.connection_id, PluginRegistryErrorKind::InvalidConfig)?;
        validate_visible_ascii(&self.tool_name, 128, PluginRegistryErrorKind::InvalidConfig)?;
        let first = self.tool_name.as_bytes()[0];
        if !first.is_ascii_alphanumeric()
            || self.tool_name.bytes().any(|byte| {
                !(byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b':' | b'/' | b'-'))
            })
        {
            return invalid();
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct AutomaticConfig {
    enabled: bool,
    operations: Vec<AutomaticOperation>,
    all_subscribed_feeds: bool,
    feed_ids: Vec<String>,
    category_ids: Vec<String>,
}

impl AutomaticConfig {
    fn validate(&self, operations: &Operations) -> Result<(), PluginRegistryError> {
        if self.operations.is_empty()
            || self.operations.len() > 2
            || self.feed_ids.len() > 1000
            || self.category_ids.len() > 250
        {
            return invalid();
        }
        let mut operation_set = HashSet::new();
        if !self
            .operations
            .iter()
            .all(|operation| operation_set.insert(*operation))
        {
            return invalid();
        }
        validate_unique_uuids(&self.feed_ids)?;
        validate_unique_uuids(&self.category_ids)?;

        if self.enabled {
            if !self.all_subscribed_feeds
                && self.feed_ids.is_empty()
                && self.category_ids.is_empty()
            {
                return invalid();
            }
            for operation in &self.operations {
                let enabled = match operation {
                    AutomaticOperation::Summarize => operations.summarize.enabled,
                    AutomaticOperation::Translate => operations.translate.enabled,
                };
                if !enabled {
                    return invalid();
                }
            }
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
enum AutomaticOperation {
    Summarize,
    Translate,
}

fn validate_unique_uuids(values: &[String]) -> Result<(), PluginRegistryError> {
    let mut seen = HashSet::new();
    for value in values {
        validate_uuid(value, PluginRegistryErrorKind::InvalidConfig)?;
        if !seen.insert(value) {
            return invalid();
        }
    }
    Ok(())
}

fn invalid<T>() -> Result<T, PluginRegistryError> {
    Err(PluginRegistryError::new(
        PluginRegistryErrorKind::InvalidConfig,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const SUMMARY_PROVIDER_ID: &str = "00000000-0000-4000-8000-000000000901";
    const TRANSLATION_PROVIDER_ID: &str = "00000000-0000-4000-8000-000000000902";

    #[test]
    fn public_ai_config_view_is_typed_and_round_trips_canonically() {
        let config = AiContentConfig::parse(&config_json("DETAILED"))
            .expect("AI content config should parse");

        assert_eq!(config.schema_version(), 1);
        assert!(config.summarize_enabled());
        assert_eq!(config.summarize_provider_id(), SUMMARY_PROVIDER_ID);
        assert_eq!(config.summarize_style(), AiSummaryStyle::Detailed);
        assert_eq!(config.summarize_max_output_tokens(), 1024);
        assert!(!config.summarize_mcp_enabled());
        assert!(config.translate_enabled());
        assert_eq!(config.translate_provider_id(), TRANSLATION_PROVIDER_ID);
        assert_eq!(config.default_target_locale(), "zh-CN");
        assert_eq!(config.translate_max_output_tokens(), 4096);
        assert!(!config.translate_mcp_enabled());

        let reparsed = AiContentConfig::parse(config.canonical_json().as_bytes())
            .expect("canonical AI content config should parse");
        assert_eq!(reparsed, config);
    }

    #[test]
    fn public_summary_style_accepts_only_the_three_wire_values() {
        for (wire, expected) in [
            ("CONCISE", AiSummaryStyle::Concise),
            ("BALANCED", AiSummaryStyle::Balanced),
            ("DETAILED", AiSummaryStyle::Detailed),
        ] {
            let config = AiContentConfig::parse(&config_json(wire))
                .expect("public summary style should parse");
            assert_eq!(config.summarize_style(), expected);
        }
        assert_eq!(
            AiContentConfig::parse(&config_json("EXPANSIVE"))
                .expect_err("unknown summary style should fail")
                .kind(),
            PluginRegistryErrorKind::InvalidConfig
        );
    }

    fn config_json(style: &str) -> Vec<u8> {
        serde_json::to_vec(&json!({
            "schemaVersion": 1,
            "operations": {
                "summarize": {
                    "enabled": true,
                    "providerId": SUMMARY_PROVIDER_ID,
                    "style": style,
                    "maxOutputTokens": 1024,
                    "mcp": {
                        "mode": "DISABLED",
                        "failurePolicy": "FAIL_OPEN",
                        "maxToolCalls": 0,
                        "tools": []
                    }
                },
                "translate": {
                    "enabled": true,
                    "providerId": TRANSLATION_PROVIDER_ID,
                    "defaultTargetLocale": "zh-CN",
                    "maxOutputTokens": 4096,
                    "mcp": {
                        "mode": "DISABLED",
                        "failurePolicy": "FAIL_OPEN",
                        "maxToolCalls": 0,
                        "tools": []
                    }
                }
            },
            "automatic": {
                "enabled": false,
                "operations": ["SUMMARIZE", "TRANSLATE"],
                "allSubscribedFeeds": false,
                "feedIds": [],
                "categoryIds": []
            }
        }))
        .expect("AI config fixture should serialize")
    }
}
