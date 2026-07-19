use crate::content::jobs::ContentJobOperation;

pub const OFFICIAL_AI_PLUGIN_KEY: &str = "raindrop.ai-content";
pub const OFFICIAL_AI_ABI_VERSION: &str = "raindrop:content-plugin@1.0.0";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct OfficialAiOperationContract {
    pub plugin_key: &'static str,
    pub prompt_version: &'static str,
    pub schema_id: &'static str,
}

#[must_use]
pub const fn official_ai_contract(operation: ContentJobOperation) -> OfficialAiOperationContract {
    match operation {
        ContentJobOperation::Summarize => OfficialAiOperationContract {
            plugin_key: OFFICIAL_AI_PLUGIN_KEY,
            prompt_version: "raindrop-summary-v1",
            schema_id: "raindrop://schemas/artifacts/ai-summary/v1",
        },
        ContentJobOperation::Translate => OfficialAiOperationContract {
            plugin_key: OFFICIAL_AI_PLUGIN_KEY,
            prompt_version: "raindrop-translation-v1",
            schema_id: "raindrop://schemas/artifacts/ai-translation/v1",
        },
    }
}

#[cfg(test)]
mod tests {
    use serde_json::Value;

    use super::*;

    #[test]
    fn official_contract_matches_committed_artifact_schema_ids() {
        let summary: Value = serde_json::from_str(include_str!(
            "../../../contracts/artifacts/ai-summary.v1.schema.json"
        ))
        .expect("summary schema should parse");
        let translation: Value = serde_json::from_str(include_str!(
            "../../../contracts/artifacts/ai-translation.v1.schema.json"
        ))
        .expect("translation schema should parse");

        let summary_contract = official_ai_contract(ContentJobOperation::Summarize);
        let translation_contract = official_ai_contract(ContentJobOperation::Translate);
        assert_eq!(summary_contract.plugin_key, OFFICIAL_AI_PLUGIN_KEY);
        assert_eq!(translation_contract.plugin_key, OFFICIAL_AI_PLUGIN_KEY);
        assert_eq!(summary_contract.prompt_version, "raindrop-summary-v1");
        assert_eq!(
            translation_contract.prompt_version,
            "raindrop-translation-v1"
        );
        assert_eq!(summary["$id"], summary_contract.schema_id);
        assert_eq!(translation["$id"], translation_contract.schema_id);
    }
}
