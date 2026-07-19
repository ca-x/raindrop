use std::fmt;

use serde::de::{DeserializeSeed, Error as _, MapAccess, SeqAccess, Visitor};
use serde_json::{Map, Number, Value};

const DUPLICATE_KEY_MARKER: &str = "raindrop_duplicate_json_key";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum JsonError {
    Invalid,
    TooLarge,
    NotCanonical,
    ExpectedObject,
}

pub(crate) fn parse_unique(input: &str, max_bytes: usize) -> Result<Value, JsonError> {
    if input.len() > max_bytes {
        return Err(JsonError::TooLarge);
    }
    let mut deserializer = serde_json::Deserializer::from_str(input);
    let value = UniqueValue
        .deserialize(&mut deserializer)
        .map_err(|_| JsonError::Invalid)?;
    deserializer.end().map_err(|_| JsonError::Invalid)?;
    Ok(value)
}

pub(crate) fn parse_canonical_object(input: &str, max_bytes: usize) -> Result<Value, JsonError> {
    let value = parse_unique(input, max_bytes)?;
    if !value.is_object() {
        return Err(JsonError::ExpectedObject);
    }
    if canonical_json(value.clone(), max_bytes)? != input {
        return Err(JsonError::NotCanonical);
    }
    Ok(value)
}

pub(crate) fn canonical_json(value: Value, max_bytes: usize) -> Result<String, JsonError> {
    let normalized = normalize(value);
    let encoded = serde_json::to_string(&normalized).map_err(|_| JsonError::Invalid)?;
    if encoded.len() > max_bytes {
        Err(JsonError::TooLarge)
    } else {
        Ok(encoded)
    }
}

fn normalize(value: Value) -> Value {
    match value {
        Value::Array(values) => Value::Array(values.into_iter().map(normalize).collect()),
        Value::Object(values) => {
            let mut entries = values.into_iter().collect::<Vec<_>>();
            entries.sort_by(|left, right| left.0.cmp(&right.0));
            let mut normalized = Map::new();
            for (key, value) in entries {
                normalized.insert(key, normalize(value));
            }
            Value::Object(normalized)
        }
        scalar => scalar,
    }
}

struct UniqueValue;

impl<'de> DeserializeSeed<'de> for UniqueValue {
    type Value = Value;

    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        deserializer.deserialize_any(UniqueValueVisitor)
    }
}

struct UniqueValueVisitor;

impl<'de> Visitor<'de> for UniqueValueVisitor {
    type Value = Value;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("JSON without duplicate object keys")
    }

    fn visit_bool<E>(self, value: bool) -> Result<Self::Value, E> {
        Ok(Value::Bool(value))
    }

    fn visit_i64<E>(self, value: i64) -> Result<Self::Value, E> {
        Ok(Value::Number(Number::from(value)))
    }

    fn visit_u64<E>(self, value: u64) -> Result<Self::Value, E> {
        Ok(Value::Number(Number::from(value)))
    }

    fn visit_f64<E>(self, value: f64) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        Number::from_f64(value)
            .map(Value::Number)
            .ok_or_else(|| E::custom("invalid JSON number"))
    }

    fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        Ok(Value::String(value.to_owned()))
    }

    fn visit_string<E>(self, value: String) -> Result<Self::Value, E> {
        Ok(Value::String(value))
    }

    fn visit_none<E>(self) -> Result<Self::Value, E> {
        Ok(Value::Null)
    }

    fn visit_unit<E>(self) -> Result<Self::Value, E> {
        Ok(Value::Null)
    }

    fn visit_some<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        UniqueValue.deserialize(deserializer)
    }

    fn visit_seq<A>(self, mut sequence: A) -> Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        let mut values = Vec::new();
        while let Some(value) = sequence.next_element_seed(UniqueValue)? {
            values.push(value);
        }
        Ok(Value::Array(values))
    }

    fn visit_map<A>(self, mut object: A) -> Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        let mut values = Map::new();
        while let Some(key) = object.next_key::<String>()? {
            if values.contains_key(&key) {
                return Err(A::Error::custom(DUPLICATE_KEY_MARKER));
            }
            values.insert(key, object.next_value_seed(UniqueValue)?);
        }
        Ok(Value::Object(values))
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn canonical_json_sorts_recursively_and_rejects_duplicate_or_pretty_input() {
        assert_eq!(
            canonical_json(json!({"z":{"b":2,"a":1},"a":[{"d":4,"c":3}]}), 1024)
                .expect("canonical JSON"),
            r#"{"a":[{"c":3,"d":4}],"z":{"a":1,"b":2}}"#,
        );
        assert_eq!(
            parse_canonical_object(r#"{"a":1,"a":2}"#, 1024),
            Err(JsonError::Invalid),
        );
        assert_eq!(
            parse_canonical_object("{\n  \"a\": 1\n}", 1024),
            Err(JsonError::NotCanonical),
        );
        assert_eq!(
            parse_canonical_object(r#"{"a":"12345"}"#, 8),
            Err(JsonError::TooLarge),
        );
    }
}
