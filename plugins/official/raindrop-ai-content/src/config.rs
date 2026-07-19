use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use crate::{
    Failure,
    json::{canonical_json, parse_canonical_object},
};

const MAX_CONFIG_BYTES: usize = 256 * 1024;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Config {
    pub(crate) operations: Operations,
    pub(crate) automatic: AutomaticConfig,
}

impl Config {
    pub(crate) fn parse(input: &str) -> Result<Self, Failure> {
        let value =
            parse_canonical_object(input, MAX_CONFIG_BYTES).map_err(|_| Failure::ConfigInvalid)?;
        let mut document =
            serde_json::from_value::<ConfigDocument>(value).map_err(|_| Failure::ConfigInvalid)?;
        document.validate()?;
        document.operations.translate.default_target_locale =
            normalize_locale(&document.operations.translate.default_target_locale)
                .ok_or(Failure::ConfigInvalid)?;
        let normalized = serde_json::to_value(&document).map_err(|_| Failure::ConfigInvalid)?;
        if canonical_json(normalized, MAX_CONFIG_BYTES).map_err(|_| Failure::ConfigInvalid)?
            != input
        {
            return Err(Failure::ConfigInvalid);
        }
        Ok(Self {
            operations: document.operations,
            automatic: document.automatic,
        })
    }

    pub(crate) fn operation(&self, operation: OperationKind) -> OperationConfig<'_> {
        match operation {
            OperationKind::Summarize => OperationConfig {
                enabled: self.operations.summarize.enabled,
                provider_id: &self.operations.summarize.provider_id,
                max_output_tokens: self.operations.summarize.max_output_tokens,
                mcp: &self.operations.summarize.mcp,
            },
            OperationKind::Translate => OperationConfig {
                enabled: self.operations.translate.enabled,
                provider_id: &self.operations.translate.provider_id,
                max_output_tokens: self.operations.translate.max_output_tokens,
                mcp: &self.operations.translate.mcp,
            },
        }
    }
}

pub(crate) struct OperationConfig<'a> {
    pub(crate) enabled: bool,
    pub(crate) provider_id: &'a str,
    pub(crate) max_output_tokens: u32,
    pub(crate) mcp: &'a McpConfig,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct ConfigDocument {
    schema_version: u32,
    operations: Operations,
    automatic: AutomaticConfig,
}

impl ConfigDocument {
    fn validate(&self) -> Result<(), Failure> {
        if self.schema_version != 1 {
            return Err(Failure::ConfigInvalid);
        }
        self.operations.summarize.validate(128, 4_096)?;
        self.operations.translate.validate()?;
        self.automatic.validate(&self.operations)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Operations {
    pub(crate) summarize: SummarizeConfig,
    pub(crate) translate: TranslateConfig,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct SummarizeConfig {
    pub(crate) enabled: bool,
    pub(crate) provider_id: String,
    pub(crate) style: SummaryStyle,
    pub(crate) max_output_tokens: u32,
    pub(crate) mcp: McpConfig,
}

impl SummarizeConfig {
    fn validate(&self, minimum: u32, maximum: u32) -> Result<(), Failure> {
        if !valid_uuid(&self.provider_id) || !(minimum..=maximum).contains(&self.max_output_tokens)
        {
            return Err(Failure::ConfigInvalid);
        }
        self.mcp.validate()
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub(crate) enum SummaryStyle {
    Concise,
    Balanced,
    Detailed,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct TranslateConfig {
    pub(crate) enabled: bool,
    pub(crate) provider_id: String,
    pub(crate) default_target_locale: String,
    pub(crate) max_output_tokens: u32,
    pub(crate) mcp: McpConfig,
}

impl TranslateConfig {
    fn validate(&self) -> Result<(), Failure> {
        if !valid_uuid(&self.provider_id)
            || normalize_locale(&self.default_target_locale).is_none()
            || !(256..=16_384).contains(&self.max_output_tokens)
        {
            return Err(Failure::ConfigInvalid);
        }
        self.mcp.validate()
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct McpConfig {
    pub(crate) mode: McpMode,
    pub(crate) failure_policy: McpFailurePolicy,
    pub(crate) max_tool_calls: u8,
    pub(crate) tools: Vec<McpToolSelection>,
}

impl McpConfig {
    fn validate(&self) -> Result<(), Failure> {
        if self.tools.len() > 16 || self.max_tool_calls > 4 {
            return Err(Failure::ConfigInvalid);
        }
        match self.mode {
            McpMode::Disabled if self.max_tool_calls != 0 || !self.tools.is_empty() => {
                return Err(Failure::ConfigInvalid);
            }
            McpMode::ContextEnrichment if self.max_tool_calls == 0 || self.tools.is_empty() => {
                return Err(Failure::ConfigInvalid);
            }
            McpMode::Disabled | McpMode::ContextEnrichment => {}
        }
        let mut seen = HashSet::new();
        for tool in &self.tools {
            if !valid_uuid(&tool.connection_id)
                || !valid_tool_name(&tool.tool_name)
                || !seen.insert((tool.connection_id.as_str(), tool.tool_name.as_str()))
            {
                return Err(Failure::ConfigInvalid);
            }
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub(crate) enum McpMode {
    Disabled,
    ContextEnrichment,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub(crate) enum McpFailurePolicy {
    FailOpen,
    FailClosed,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct McpToolSelection {
    pub(crate) connection_id: String,
    pub(crate) tool_name: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct AutomaticConfig {
    pub(crate) enabled: bool,
    pub(crate) operations: Vec<OperationKind>,
    pub(crate) all_subscribed_feeds: bool,
    pub(crate) feed_ids: Vec<String>,
    pub(crate) category_ids: Vec<String>,
}

impl AutomaticConfig {
    fn validate(&self, operations: &Operations) -> Result<(), Failure> {
        if self.operations.is_empty()
            || self.operations.len() > 2
            || self.feed_ids.len() > 1_000
            || self.category_ids.len() > 250
            || !unique(self.operations.iter().copied())
            || !unique_valid_uuids(&self.feed_ids)
            || !unique_valid_uuids(&self.category_ids)
        {
            return Err(Failure::ConfigInvalid);
        }
        if self.enabled {
            if !self.all_subscribed_feeds
                && self.feed_ids.is_empty()
                && self.category_ids.is_empty()
            {
                return Err(Failure::ConfigInvalid);
            }
            for operation in &self.operations {
                let enabled = match operation {
                    OperationKind::Summarize => operations.summarize.enabled,
                    OperationKind::Translate => operations.translate.enabled,
                };
                if !enabled {
                    return Err(Failure::ConfigInvalid);
                }
            }
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub(crate) enum OperationKind {
    Summarize,
    Translate,
}

impl OperationKind {
    pub(crate) const fn as_key(self) -> &'static str {
        match self {
            Self::Summarize => "summarize",
            Self::Translate => "translate",
        }
    }
}

pub(crate) fn normalize_locale(value: &str) -> Option<String> {
    if !(2..=35).contains(&value.len()) || !value.is_ascii() {
        return None;
    }
    let parts = value.split('-').collect::<Vec<_>>();
    if !(2..=8).contains(&parts[0].len())
        || !parts[0].bytes().all(|byte| byte.is_ascii_alphabetic())
        || parts.iter().skip(1).any(|part| {
            part.is_empty()
                || part.len() > 8
                || !part.bytes().all(|byte| byte.is_ascii_alphanumeric())
        })
    {
        return None;
    }
    let mut normalized = Vec::with_capacity(parts.len());
    normalized.push(parts[0].to_ascii_lowercase());
    for part in &parts[1..] {
        let segment = if part.len() == 4 && part.bytes().all(|byte| byte.is_ascii_alphabetic()) {
            let mut chars = part.to_ascii_lowercase().chars().collect::<Vec<_>>();
            chars[0] = chars[0].to_ascii_uppercase();
            chars.into_iter().collect()
        } else if (part.len() == 2 && part.bytes().all(|byte| byte.is_ascii_alphabetic()))
            || (part.len() == 3 && part.bytes().all(|byte| byte.is_ascii_digit()))
        {
            part.to_ascii_uppercase()
        } else {
            part.to_ascii_lowercase()
        };
        normalized.push(segment);
    }
    Some(normalized.join("-"))
}

fn valid_uuid(value: &str) -> bool {
    value.len() == 36
        && value.bytes().enumerate().all(|(index, byte)| {
            if matches!(index, 8 | 13 | 18 | 23) {
                byte == b'-'
            } else {
                byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)
            }
        })
}

fn valid_tool_name(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value.as_bytes()[0].is_ascii_alphanumeric()
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b':' | b'/' | b'-')
        })
}

fn unique<T: Eq + std::hash::Hash>(values: impl Iterator<Item = T>) -> bool {
    let mut seen = HashSet::new();
    values.into_iter().all(|value| seen.insert(value))
}

fn unique_valid_uuids(values: &[String]) -> bool {
    let mut seen = HashSet::new();
    values
        .iter()
        .all(|value| valid_uuid(value) && seen.insert(value))
}

#[cfg(test)]
pub(crate) mod tests {
    use serde_json::json;

    use super::*;
    use crate::json::canonical_json;

    #[test]
    fn config_parses_exact_modes_and_normalizes_no_hidden_values() {
        let config = fixture_config();
        let parsed = Config::parse(&config).expect("valid config");
        assert_eq!(parsed.operations.summarize.style, SummaryStyle::Balanced);
        assert_eq!(parsed.operations.translate.default_target_locale, "zh-CN");
        assert_eq!(
            parsed.operation(OperationKind::Summarize).max_output_tokens,
            512
        );
        assert_eq!(parsed.automatic.operations.len(), 2);

        let mut noncanonical: serde_json::Value =
            serde_json::from_str(&config).expect("config JSON");
        noncanonical["operations"]["translate"]["defaultTargetLocale"] = json!("zh-cn");
        let noncanonical = canonical_json(noncanonical, MAX_CONFIG_BYTES).expect("config JSON");
        assert_eq!(Config::parse(&noncanonical), Err(Failure::ConfigInvalid));
    }

    #[test]
    fn config_rejects_disabled_mcp_with_tools_and_automatic_disabled_operation() {
        let mut value: serde_json::Value =
            serde_json::from_str(&fixture_config()).expect("config JSON");
        value["operations"]["summarize"]["mcp"]["mode"] = json!("DISABLED");
        assert_invalid(value);

        let mut value: serde_json::Value =
            serde_json::from_str(&fixture_config()).expect("config JSON");
        value["operations"]["translate"]["enabled"] = json!(false);
        assert_invalid(value);
    }

    pub(crate) fn fixture_config() -> String {
        canonical_json(
            json!({
                "schemaVersion": 1,
                "operations": {
                    "summarize": {
                        "enabled": true,
                        "providerId": "00000000-0000-4000-8000-000000000101",
                        "style": "BALANCED",
                        "maxOutputTokens": 512,
                        "mcp": {
                            "mode": "CONTEXT_ENRICHMENT",
                            "failurePolicy": "FAIL_OPEN",
                            "maxToolCalls": 2,
                            "tools": [{
                                "connectionId": "00000000-0000-4000-8000-000000000201",
                                "toolName": "search.read"
                            }]
                        }
                    },
                    "translate": {
                        "enabled": true,
                        "providerId": "00000000-0000-4000-8000-000000000102",
                        "defaultTargetLocale": "zh-CN",
                        "maxOutputTokens": 1024,
                        "mcp": {
                            "mode": "DISABLED",
                            "failurePolicy": "FAIL_CLOSED",
                            "maxToolCalls": 0,
                            "tools": []
                        }
                    }
                },
                "automatic": {
                    "enabled": true,
                    "operations": ["SUMMARIZE", "TRANSLATE"],
                    "allSubscribedFeeds": true,
                    "feedIds": [],
                    "categoryIds": []
                }
            }),
            MAX_CONFIG_BYTES,
        )
        .expect("fixture config")
    }

    fn assert_invalid(value: serde_json::Value) {
        let input = canonical_json(value, MAX_CONFIG_BYTES).expect("config JSON");
        assert_eq!(Config::parse(&input), Err(Failure::ConfigInvalid));
    }
}
