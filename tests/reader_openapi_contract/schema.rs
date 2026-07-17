use serde_json::Value;

use super::document::resolve_ref;

pub(crate) fn validate_schema(
    document: &Value,
    schema: &Value,
    value: &Value,
    path: &str,
) -> Result<(), String> {
    let schema = resolve_ref(document, schema);
    if let Some(any_of) = schema.get("anyOf").and_then(Value::as_array)
        && !any_of
            .iter()
            .any(|candidate| validate_schema(document, candidate, value, path).is_ok())
    {
        return Err(format!("{path} did not match anyOf"));
    }
    if let Some(enum_values) = schema.get("enum").and_then(Value::as_array)
        && !enum_values.contains(value)
    {
        return Err(format!("{path} is not an allowed enum value"));
    }

    let types = schema.get("type").map_or_else(Vec::new, |kind| match kind {
        Value::String(kind) => vec![kind.as_str()],
        Value::Array(kinds) => kinds.iter().filter_map(Value::as_str).collect(),
        _ => Vec::new(),
    });
    if !types.is_empty() && !types.iter().any(|kind| matches_type(kind, value)) {
        return Err(format!("{path} has the wrong type"));
    }

    validate_format(schema, value, path)?;
    validate_numeric_bounds(schema, value, path)?;
    validate_object_keywords(document, schema, value, path)?;
    if let Some(items) = schema.get("items")
        && let Some(values) = value.as_array()
    {
        for (index, item) in values.iter().enumerate() {
            validate_schema(document, items, item, &format!("{path}[{index}]"))?;
        }
    }
    Ok(())
}

fn validate_format(schema: &Value, value: &Value, path: &str) -> Result<(), String> {
    let Some(format) = schema.get("format").and_then(Value::as_str) else {
        return Ok(());
    };
    let Some(value) = value.as_str() else {
        return Ok(());
    };
    let valid = match format {
        "uuid" => is_browser_uuid(value),
        "uri" => url::Url::parse(value).is_ok(),
        _ => return Err(format!("{path} uses unsupported format {format}")),
    };
    valid
        .then_some(())
        .ok_or_else(|| format!("{path} does not match format {format}"))
}

fn validate_numeric_bounds(schema: &Value, value: &Value, path: &str) -> Result<(), String> {
    let Some(value) = value.as_f64() else {
        return Ok(());
    };
    if let Some(minimum) = schema.get("minimum").and_then(Value::as_f64)
        && value < minimum
    {
        return Err(format!("{path} is below minimum {minimum}"));
    }
    if let Some(maximum) = schema.get("maximum").and_then(Value::as_f64)
        && value > maximum
    {
        return Err(format!("{path} is above maximum {maximum}"));
    }
    Ok(())
}

fn validate_object_keywords(
    document: &Value,
    schema: &Value,
    value: &Value,
    path: &str,
) -> Result<(), String> {
    let Some(object) = value.as_object() else {
        return Ok(());
    };
    let properties = schema.get("properties").and_then(Value::as_object);
    if let Some(required) = schema.get("required").and_then(Value::as_array) {
        for field in required.iter().filter_map(Value::as_str) {
            if !object.contains_key(field) {
                return Err(format!("{path}.{field} is required"));
            }
        }
    }
    if schema.get("additionalProperties") == Some(&Value::Bool(false)) {
        let properties = properties.ok_or_else(|| format!("{path} has no properties"))?;
        for field in object.keys() {
            if !properties.contains_key(field) {
                return Err(format!("{path}.{field} is not allowed"));
            }
        }
    }
    if let Some(properties) = properties {
        for (field, field_schema) in properties {
            if let Some(field_value) = object.get(field) {
                validate_schema(
                    document,
                    field_schema,
                    field_value,
                    &format!("{path}.{field}"),
                )?;
            }
        }
    }
    if let Some(additional_schema) = schema
        .get("additionalProperties")
        .filter(|value| value.is_object())
    {
        for (field, field_value) in object {
            if properties.is_none_or(|properties| !properties.contains_key(field)) {
                validate_schema(
                    document,
                    additional_schema,
                    field_value,
                    &format!("{path}.{field}"),
                )?;
            }
        }
    }
    Ok(())
}

fn is_browser_uuid(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() == 36
        && [8, 13, 18, 23]
            .into_iter()
            .all(|index| bytes[index] == b'-')
        && bytes
            .iter()
            .enumerate()
            .all(|(index, byte)| [8, 13, 18, 23].contains(&index) || byte.is_ascii_hexdigit())
        && matches!(bytes[14], b'1'..=b'5')
        && matches!(bytes[19].to_ascii_lowercase(), b'8' | b'9' | b'a' | b'b')
}

fn matches_type(kind: &str, value: &Value) -> bool {
    match kind {
        "null" => value.is_null(),
        "object" => value.is_object(),
        "array" => value.is_array(),
        "string" => value.is_string(),
        "integer" => value.as_i64().is_some() || value.as_u64().is_some(),
        "number" => value.is_number(),
        "boolean" => value.is_boolean(),
        _ => false,
    }
}
