use std::collections::{BTreeMap, BTreeSet};

use serde::Deserialize;
use serde_json::{Value, json};

use crate::json::{canonical_json, parse_canonical_object};

pub(crate) const TOOL_PLAN_SCHEMA_ID: &str =
    "raindrop://schemas/plugins/raindrop.ai-content/tool-plan/v1";
const JSON_SCHEMA_DRAFT: &str = "https://json-schema.org/draft/2020-12/schema";
const MAX_SCHEMA_BYTES: usize = 64 * 1024;
const MAX_OUTPUT_BYTES: usize = 512 * 1024;
const MAX_BINDINGS: usize = 16;
const MAX_CALLS: usize = 4;

#[derive(Clone, Copy)]
pub(crate) struct ToolDescriptor<'a> {
    pub(crate) binding_id: &'a str,
    pub(crate) input_schema_json: &'a str,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ToolCall {
    pub(crate) binding_id: String,
    pub(crate) arguments_json: String,
}

pub(crate) fn build_schema<'a>(
    bindings: impl IntoIterator<Item = ToolDescriptor<'a>>,
    max_calls: usize,
) -> Result<String, ()> {
    let mut schemas = BTreeMap::new();
    for binding in bindings {
        if !valid_binding_id(binding.binding_id) {
            return Err(());
        }
        let schema =
            parse_canonical_object(binding.input_schema_json, MAX_SCHEMA_BYTES).map_err(|_| ())?;
        if schemas
            .insert(binding.binding_id.to_owned(), schema)
            .is_some()
        {
            return Err(());
        }
    }
    if schemas.is_empty()
        || schemas.len() > MAX_BINDINGS
        || max_calls == 0
        || max_calls > MAX_CALLS
        || max_calls > schemas.len()
    {
        return Err(());
    }
    let branches = schemas
        .into_iter()
        .map(|(binding_id, schema)| {
            json!({
                "additionalProperties": false,
                "properties": {
                    "arguments": schema,
                    "toolBindingId": {"const": binding_id},
                },
                "required": ["toolBindingId", "arguments"],
                "type": "object",
            })
        })
        .collect::<Vec<_>>();
    canonical_json(
        json!({
            "$id": TOOL_PLAN_SCHEMA_ID,
            "$schema": JSON_SCHEMA_DRAFT,
            "additionalProperties": false,
            "properties": {
                "calls": {
                    "items": {"oneOf": branches},
                    "maxItems": max_calls,
                    "type": "array",
                },
                "schemaVersion": {"const": 1},
            },
            "required": ["schemaVersion", "calls"],
            "type": "object",
        }),
        MAX_SCHEMA_BYTES,
    )
    .map_err(|_| ())
}

pub(crate) fn parse_plan(
    input: &str,
    allowed_binding_ids: &BTreeSet<String>,
    max_calls: usize,
) -> Result<Vec<ToolCall>, ()> {
    let value = parse_canonical_object(input, MAX_OUTPUT_BYTES).map_err(|_| ())?;
    let output = serde_json::from_value::<ToolPlanOutput>(value).map_err(|_| ())?;
    if output.schema_version != 1 || output.calls.len() > max_calls {
        return Err(());
    }
    let mut seen = BTreeSet::new();
    output
        .calls
        .into_iter()
        .map(|call| {
            if !allowed_binding_ids.contains(&call.tool_binding_id)
                || !seen.insert(call.tool_binding_id.clone())
                || !call.arguments.is_object()
            {
                return Err(());
            }
            let arguments_json =
                canonical_json(call.arguments, MAX_SCHEMA_BYTES).map_err(|_| ())?;
            Ok(ToolCall {
                binding_id: call.tool_binding_id,
                arguments_json,
            })
        })
        .collect()
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct ToolPlanOutput {
    schema_version: u32,
    calls: Vec<ToolPlanCall>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct ToolPlanCall {
    tool_binding_id: String,
    arguments: Value,
}

fn valid_binding_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value.bytes().all(|byte| (0x21..=0x7e).contains(&byte))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_is_canonical_sorted_and_plan_is_bounded() {
        let schema = build_schema(
            [
                ToolDescriptor {
                    binding_id: "binding-b",
                    input_schema_json: r#"{"additionalProperties":false,"properties":{"limit":{"type":"integer"}},"type":"object"}"#,
                },
                ToolDescriptor {
                    binding_id: "binding-a",
                    input_schema_json: r#"{"additionalProperties":false,"properties":{"query":{"type":"string"}},"type":"object"}"#,
                },
            ],
            2,
        )
        .expect("tool plan schema");
        assert!(
            schema.find("binding-a").expect("binding a")
                < schema.find("binding-b").expect("binding b")
        );
        assert!(schema.contains(r#""maxItems":2"#));

        let allowed = ["binding-a".to_owned(), "binding-b".to_owned()]
            .into_iter()
            .collect();
        let calls = parse_plan(
            r#"{"calls":[{"arguments":{"query":"rust"},"toolBindingId":"binding-a"}],"schemaVersion":1}"#,
            &allowed,
            2,
        )
        .expect("valid plan");
        assert_eq!(calls[0].arguments_json, r#"{"query":"rust"}"#);
        for invalid in [
            r#"{"calls":[{"arguments":[],"toolBindingId":"binding-a"}],"schemaVersion":1}"#,
            r#"{"calls":[{"arguments":{},"toolBindingId":"unknown"}],"schemaVersion":1}"#,
            r#"{"calls":[{"arguments":{},"toolBindingId":"binding-a"},{"arguments":{},"toolBindingId":"binding-a"}],"schemaVersion":1}"#,
        ] {
            assert!(parse_plan(invalid, &allowed, 2).is_err());
        }
    }
}
