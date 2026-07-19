use serde_json::Value;

use crate::plugins::{
    PluginRegistryErrorKind, SummaryArtifact, TranslationArtifact,
    json::{canonical_json, normalize_locale, parse_unique_json},
    runtime::{AiBrokerError, AiBrokerErrorKind, bindings::types},
};

const MAX_SCHEMA_BYTES: usize = 64 * 1024;
const MAX_INPUT_BYTES: usize = 512 * 1024;
const SUMMARY_SCHEMA_ID: &str = "raindrop://schemas/artifacts/ai-summary/v1";
const TRANSLATION_SCHEMA_ID: &str = "raindrop://schemas/artifacts/ai-translation/v1";
const SUMMARY_SCHEMA_NAME: &str = "raindrop_ai_summary_v1";
const TRANSLATION_SCHEMA_NAME: &str = "raindrop_ai_translation_v1";
const SUMMARY_SCHEMA_DOCUMENT: &str =
    include_str!("../../../contracts/artifacts/ai-summary.v1.schema.json");
const TRANSLATION_SCHEMA_DOCUMENT: &str =
    include_str!("../../../contracts/artifacts/ai-translation.v1.schema.json");

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum OfficialSchema {
    Summary,
    Translation,
}

impl OfficialSchema {
    pub(super) fn validate_request(
        operation: types::Operation,
        schema_id: &str,
        schema_json: &str,
    ) -> Result<Self, AiBrokerError> {
        let schema = match operation {
            types::Operation::Summarize => Self::Summary,
            types::Operation::Translate => Self::Translation,
        };
        let request_schema =
            canonical_object_exact(schema_json, MAX_SCHEMA_BYTES).ok_or_else(invalid_request)?;
        if schema_id != schema.schema_id() || Ok(request_schema) != canonical_contract(schema) {
            return Err(invalid_request());
        }
        Ok(schema)
    }

    pub(super) const fn schema_id(self) -> &'static str {
        match self {
            Self::Summary => SUMMARY_SCHEMA_ID,
            Self::Translation => TRANSLATION_SCHEMA_ID,
        }
    }

    pub(super) const fn schema_name(self) -> &'static str {
        match self {
            Self::Summary => SUMMARY_SCHEMA_NAME,
            Self::Translation => TRANSLATION_SCHEMA_NAME,
        }
    }

    pub(super) fn schema_value(self) -> Result<Value, AiBrokerError> {
        parse_unique_json(self.schema_document().as_bytes(), MAX_SCHEMA_BYTES)
            .map_err(|_| invalid_request())
    }

    pub(super) fn validate_output(
        self,
        output: Value,
        untrusted_input: &Value,
    ) -> Result<String, AiBrokerError> {
        let encoded = serde_json::to_vec(&output).map_err(|_| output_invalid())?;
        match self {
            Self::Summary => SummaryArtifact::parse(&encoded)
                .map(|artifact| artifact.canonical_json().to_owned())
                .map_err(|_| output_invalid()),
            Self::Translation => {
                let expected_locale = untrusted_input
                    .get("targetLocale")
                    .and_then(Value::as_str)
                    .ok_or_else(invalid_request)
                    .and_then(|locale| {
                        normalize_locale(locale, PluginRegistryErrorKind::InvalidInput)
                            .map_err(|_| invalid_request())
                    })?;
                let artifact =
                    TranslationArtifact::parse(&encoded).map_err(|_| output_invalid())?;
                if artifact.target_locale() != expected_locale {
                    return Err(output_invalid());
                }
                Ok(artifact.canonical_json().to_owned())
            }
        }
    }

    const fn schema_document(self) -> &'static str {
        match self {
            Self::Summary => SUMMARY_SCHEMA_DOCUMENT,
            Self::Translation => TRANSLATION_SCHEMA_DOCUMENT,
        }
    }
}

pub(super) fn canonical_input(input: &str) -> Result<Value, AiBrokerError> {
    let value =
        parse_unique_json(input.as_bytes(), MAX_INPUT_BYTES).map_err(|_| invalid_request())?;
    if !value.is_object()
        || !canonical_json(value.clone(), MAX_INPUT_BYTES).is_ok_and(|encoded| encoded == input)
    {
        return Err(invalid_request());
    }
    Ok(value)
}

fn canonical_contract(schema: OfficialSchema) -> Result<String, AiBrokerError> {
    canonical_object(schema.schema_document(), MAX_SCHEMA_BYTES).ok_or_else(invalid_request)
}

fn canonical_object(input: &str, max_bytes: usize) -> Option<String> {
    let value = parse_unique_json(input.as_bytes(), max_bytes).ok()?;
    value.is_object().then_some(())?;
    canonical_json(value, max_bytes).ok()
}

fn canonical_object_exact(input: &str, max_bytes: usize) -> Option<String> {
    let canonical = canonical_object(input, max_bytes)?;
    (canonical == input).then_some(canonical)
}

const fn invalid_request() -> AiBrokerError {
    AiBrokerError::new(AiBrokerErrorKind::InvalidRequest, false, None)
}

const fn output_invalid() -> AiBrokerError {
    AiBrokerError::new(AiBrokerErrorKind::OutputSchemaInvalid, false, None)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn schema_registry_accepts_only_exact_operation_contracts() {
        let summary = canonical_contract(OfficialSchema::Summary).expect("summary schema");
        assert_eq!(
            OfficialSchema::validate_request(
                types::Operation::Summarize,
                SUMMARY_SCHEMA_ID,
                &summary
            ),
            Ok(OfficialSchema::Summary),
        );
        let pretty_summary = serde_json::to_string_pretty(
            &serde_json::from_str::<Value>(&summary).expect("summary schema value"),
        )
        .expect("pretty summary schema");
        assert!(
            OfficialSchema::validate_request(
                types::Operation::Summarize,
                SUMMARY_SCHEMA_ID,
                &pretty_summary,
            )
            .is_err()
        );
        assert!(
            OfficialSchema::validate_request(
                types::Operation::Summarize,
                TRANSLATION_SCHEMA_ID,
                &summary,
            )
            .is_err()
        );
        assert!(
            OfficialSchema::validate_request(
                types::Operation::Summarize,
                SUMMARY_SCHEMA_ID,
                r#"{"$id":"raindrop://schemas/artifacts/ai-summary/v1","type":"object"}"#,
            )
            .is_err()
        );
    }

    #[test]
    fn typed_output_validation_canonicalizes_and_rejects_locale_drift() {
        let summary = json!({
            "schemaVersion": 1,
            "sourceLanguage": "en",
            "summary": "Summary.",
            "bullets": [],
            "conclusion": null,
        });
        assert_eq!(
            OfficialSchema::Summary
                .validate_output(summary, &json!({"text":"untrusted"}))
                .expect("valid summary"),
            r#"{"bullets":[],"conclusion":null,"schemaVersion":1,"sourceLanguage":"en","summary":"Summary."}"#,
        );

        let translation = json!({
            "schemaVersion": 1,
            "detectedSourceLanguage": "en",
            "targetLocale": "ja",
            "title": "Title",
            "bodyMarkdown": "Body",
        });
        assert_eq!(
            OfficialSchema::Translation
                .validate_output(translation, &json!({"targetLocale":"zh-CN"}))
                .expect_err("target locale drift"),
            output_invalid(),
        );
    }
}
