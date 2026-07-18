use std::fmt;

use blake3::Hasher;
use serde::de::{DeserializeSeed, Error as _, MapAccess, SeqAccess, Visitor};
use serde_json::{Map, Number, Value};
use uuid::Uuid;

use super::{PluginRegistryError, PluginRegistryErrorKind};

const DUPLICATE_KEY_MARKER: &str = "raindrop_duplicate_json_key";

pub(crate) fn parse_unique_json(
    input: &[u8],
    max_bytes: usize,
) -> Result<Value, PluginRegistryError> {
    if input.len() > max_bytes {
        return Err(PluginRegistryError::new(
            PluginRegistryErrorKind::PayloadTooLarge,
        ));
    }
    let mut deserializer = serde_json::Deserializer::from_slice(input);
    let value = UniqueValue
        .deserialize(&mut deserializer)
        .map_err(json_error)?;
    deserializer.end().map_err(json_error)?;
    Ok(value)
}

pub(crate) fn canonical_json(
    value: Value,
    max_bytes: usize,
) -> Result<String, PluginRegistryError> {
    let normalized = normalize(value);
    let encoded = serde_json::to_string(&normalized)
        .map_err(|_| PluginRegistryError::new(PluginRegistryErrorKind::InvalidJson))?;
    if encoded.len() > max_bytes {
        return Err(PluginRegistryError::new(
            PluginRegistryErrorKind::PayloadTooLarge,
        ));
    }
    Ok(encoded)
}

pub(crate) fn contextual_hash(context: &str, value: &[u8]) -> String {
    let mut hasher = Hasher::new_derive_key(context);
    hasher.update(&(value.len() as u64).to_be_bytes());
    hasher.update(value);
    hasher.finalize().to_hex().to_string()
}

pub(crate) fn validate_uuid(
    value: &str,
    kind: PluginRegistryErrorKind,
) -> Result<(), PluginRegistryError> {
    let parsed = Uuid::parse_str(value).map_err(|_| PluginRegistryError::new(kind))?;
    if parsed.to_string() == value {
        Ok(())
    } else {
        Err(PluginRegistryError::new(kind))
    }
}

pub(crate) fn validate_lower_hex_hash(
    value: &str,
    kind: PluginRegistryErrorKind,
) -> Result<(), PluginRegistryError> {
    if value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        Ok(())
    } else {
        Err(PluginRegistryError::new(kind))
    }
}

pub(crate) fn validate_visible_ascii(
    value: &str,
    max_bytes: usize,
    kind: PluginRegistryErrorKind,
) -> Result<(), PluginRegistryError> {
    if !value.is_empty()
        && value.len() <= max_bytes
        && value.bytes().all(|byte| (0x21..=0x7e).contains(&byte))
    {
        Ok(())
    } else {
        Err(PluginRegistryError::new(kind))
    }
}

pub(crate) fn validate_text(
    value: &str,
    max_bytes: usize,
    kind: PluginRegistryErrorKind,
) -> Result<(), PluginRegistryError> {
    if value.trim().is_empty()
        || value.len() > max_bytes
        || value
            .chars()
            .any(|character| character.is_control() && !matches!(character, '\n' | '\r' | '\t'))
    {
        Err(PluginRegistryError::new(kind))
    } else {
        Ok(())
    }
}

pub(crate) fn normalize_locale(
    value: &str,
    kind: PluginRegistryErrorKind,
) -> Result<String, PluginRegistryError> {
    if !(2..=35).contains(&value.len()) || !value.is_ascii() {
        return Err(PluginRegistryError::new(kind));
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
        return Err(PluginRegistryError::new(kind));
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
    Ok(normalized.join("-"))
}

fn json_error(error: serde_json::Error) -> PluginRegistryError {
    let kind = if error.to_string().contains(DUPLICATE_KEY_MARKER) {
        PluginRegistryErrorKind::DuplicateJsonKey
    } else {
        PluginRegistryErrorKind::InvalidJson
    };
    PluginRegistryError::new(kind)
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
        formatter.write_str("a JSON value without duplicate object keys")
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
