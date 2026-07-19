use std::{collections::BTreeMap, fmt};

use serde::Deserialize;
use serde_json::{Value, json};

use super::{
    PluginRegistryError, PluginRegistryErrorKind,
    json::{canonical_json, parse_unique_json, validate_visible_ascii},
};

pub(crate) const TOOL_PLAN_SCHEMA_ID: &str =
    "raindrop://schemas/plugins/raindrop.ai-content/tool-plan/v1";
pub(crate) const TOOL_PLAN_SCHEMA_NAME: &str = "raindrop_ai_tool_plan_v1";

const JSON_SCHEMA_DRAFT: &str = "https://json-schema.org/draft/2020-12/schema";
const MAX_SCHEMA_BYTES: usize = 64 * 1024;
const MAX_OUTPUT_BYTES: usize = 512 * 1024;
const MAX_BINDING_ID_BYTES: usize = 128;
const MAX_TOOL_BINDINGS: usize = 16;
const MAX_TOOL_CALLS: usize = 4;

#[derive(Clone, Copy)]
pub(crate) struct ToolPlanBinding<'a> {
    pub binding_id: &'a str,
    pub input_schema_json: &'a str,
}

#[derive(Clone, Eq, PartialEq)]
pub(crate) struct ToolPlanSchema {
    canonical_json: String,
    value: Value,
    max_calls: usize,
    binding_schemas: BTreeMap<String, String>,
}

impl ToolPlanSchema {
    pub(crate) fn build<'a>(
        bindings: impl IntoIterator<Item = ToolPlanBinding<'a>>,
        max_calls: usize,
    ) -> Result<Self, PluginRegistryError> {
        let mut binding_schemas = BTreeMap::new();
        for binding in bindings {
            validate_visible_ascii(
                binding.binding_id,
                MAX_BINDING_ID_BYTES,
                PluginRegistryErrorKind::InvalidInput,
            )?;
            let (schema, canonical) = canonical_object_exact(binding.input_schema_json)?;
            if binding_schemas
                .insert(binding.binding_id.to_owned(), (schema, canonical))
                .is_some()
            {
                return Err(invalid());
            }
        }
        if binding_schemas.is_empty()
            || binding_schemas.len() > MAX_TOOL_BINDINGS
            || max_calls == 0
            || max_calls > MAX_TOOL_CALLS
            || max_calls > binding_schemas.len()
        {
            return Err(invalid());
        }

        let branches = binding_schemas
            .iter()
            .map(|(binding_id, (schema, _))| {
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
        let value = json!({
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
        });
        let canonical = canonical_json(value, MAX_SCHEMA_BYTES)?;
        Self::parse(&canonical)
    }

    pub(crate) fn parse(input: &str) -> Result<Self, PluginRegistryError> {
        let value = parse_unique_json(input.as_bytes(), MAX_SCHEMA_BYTES)?;
        if canonical_json(value.clone(), MAX_SCHEMA_BYTES)? != input {
            return Err(invalid());
        }
        let document =
            serde_json::from_value::<SchemaDocument>(value.clone()).map_err(|_| invalid())?;
        if document.schema != JSON_SCHEMA_DRAFT
            || document.id != TOOL_PLAN_SCHEMA_ID
            || document.kind != "object"
            || document.additional_properties
            || document.required != ["schemaVersion", "calls"]
            || document.properties.schema_version.constant != 1
            || document.properties.calls.kind != "array"
            || document.properties.calls.max_items == 0
            || document.properties.calls.max_items > MAX_TOOL_CALLS
        {
            return Err(invalid());
        }
        let branches = document.properties.calls.items.one_of;
        if branches.is_empty()
            || branches.len() > MAX_TOOL_BINDINGS
            || document.properties.calls.max_items > branches.len()
        {
            return Err(invalid());
        }

        let mut binding_schemas = BTreeMap::new();
        let mut previous_binding_id: Option<&str> = None;
        for branch in &branches {
            let binding_id = branch.properties.tool_binding_id.constant.as_str();
            if branch.kind != "object"
                || branch.additional_properties
                || branch.required != ["toolBindingId", "arguments"]
                || validate_visible_ascii(
                    binding_id,
                    MAX_BINDING_ID_BYTES,
                    PluginRegistryErrorKind::InvalidInput,
                )
                .is_err()
                || previous_binding_id.is_some_and(|previous| previous >= binding_id)
                || !branch.properties.arguments.is_object()
            {
                return Err(invalid());
            }
            let schema_json =
                canonical_json(branch.properties.arguments.clone(), MAX_SCHEMA_BYTES)?;
            if binding_schemas
                .insert(binding_id.to_owned(), schema_json)
                .is_some()
            {
                return Err(invalid());
            }
            previous_binding_id = Some(binding_id);
        }

        Ok(Self {
            canonical_json: input.to_owned(),
            value,
            max_calls: document.properties.calls.max_items,
            binding_schemas,
        })
    }

    #[must_use]
    pub(crate) fn canonical_json(&self) -> &str {
        &self.canonical_json
    }

    #[must_use]
    pub(crate) fn value(&self) -> Value {
        self.value.clone()
    }

    #[must_use]
    pub(crate) const fn max_calls(&self) -> usize {
        self.max_calls
    }

    pub(crate) fn binding_schemas(&self) -> impl Iterator<Item = (&str, &str)> {
        self.binding_schemas
            .iter()
            .map(|(binding_id, schema)| (binding_id.as_str(), schema.as_str()))
    }

    pub(crate) fn validate_output(&self, output: Value) -> Result<String, PluginRegistryError> {
        let document =
            serde_json::from_value::<ToolPlanOutput>(output.clone()).map_err(|_| invalid())?;
        if document.schema_version != 1 || document.calls.len() > self.max_calls {
            return Err(invalid());
        }
        let mut seen = BTreeMap::new();
        for call in document.calls {
            if !self.binding_schemas.contains_key(&call.tool_binding_id)
                || !call.arguments.is_object()
                || seen.insert(call.tool_binding_id, ()).is_some()
            {
                return Err(invalid());
            }
        }
        canonical_json(output, MAX_OUTPUT_BYTES)
    }
}

impl fmt::Debug for ToolPlanSchema {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ToolPlanSchema")
            .field("max_calls", &self.max_calls)
            .field("binding_count", &self.binding_schemas.len())
            .field("schema_bytes", &self.canonical_json.len())
            .finish()
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SchemaDocument {
    #[serde(rename = "$schema")]
    schema: String,
    #[serde(rename = "$id")]
    id: String,
    #[serde(rename = "type")]
    kind: String,
    #[serde(rename = "additionalProperties")]
    additional_properties: bool,
    required: Vec<String>,
    properties: SchemaProperties,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SchemaProperties {
    #[serde(rename = "schemaVersion")]
    schema_version: ConstU32,
    calls: CallsSchema,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CallsSchema {
    #[serde(rename = "type")]
    kind: String,
    #[serde(rename = "maxItems")]
    max_items: usize,
    items: CallItems,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CallItems {
    #[serde(rename = "oneOf")]
    one_of: Vec<CallBranch>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CallBranch {
    #[serde(rename = "type")]
    kind: String,
    #[serde(rename = "additionalProperties")]
    additional_properties: bool,
    required: Vec<String>,
    properties: CallProperties,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CallProperties {
    #[serde(rename = "toolBindingId")]
    tool_binding_id: ConstString,
    arguments: Value,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ConstU32 {
    #[serde(rename = "const")]
    constant: u32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ConstString {
    #[serde(rename = "const")]
    constant: String,
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

fn canonical_object_exact(input: &str) -> Result<(Value, String), PluginRegistryError> {
    let value = parse_unique_json(input.as_bytes(), MAX_SCHEMA_BYTES)?;
    if !value.is_object() {
        return Err(invalid());
    }
    let canonical = canonical_json(value.clone(), MAX_SCHEMA_BYTES)?;
    if canonical == input {
        Ok((value, canonical))
    } else {
        Err(invalid())
    }
}

const fn invalid() -> PluginRegistryError {
    PluginRegistryError::new(PluginRegistryErrorKind::InvalidInput)
}
