use std::collections::HashSet;

use serde::Deserialize;
use serde_json::Value;
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

use super::{
    PluginRegistryError, PluginRegistryErrorKind,
    json::{
        canonical_json, normalize_locale, parse_unique_json, validate_lower_hex_hash,
        validate_text, validate_uuid, validate_visible_ascii,
    },
};

const MAX_EVENT_BYTES: usize = 1024 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LifecycleEventKind {
    FeedRefreshBefore,
    FeedRefreshFetched,
    EntryProcess,
    FeedRefreshPersisted,
    FeedRefreshCompleted,
}

impl LifecycleEventKind {
    const fn expected_sequence(self) -> u32 {
        match self {
            Self::FeedRefreshBefore => 1,
            Self::FeedRefreshFetched => 5,
            Self::EntryProcess => 8,
            Self::FeedRefreshPersisted => 10,
            Self::FeedRefreshCompleted => 20,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LifecycleEvent {
    kind: LifecycleEventKind,
    schema_version: u32,
    event_id: String,
    refresh_id: String,
    sequence: u32,
    occurred_at: String,
    idempotency_key: String,
    canonical_json: String,
}

impl LifecycleEvent {
    pub fn parse(input: &[u8]) -> Result<Self, PluginRegistryError> {
        let value = parse_unique_json(input, MAX_EVENT_BYTES)?;
        let envelope = serde_json::from_value::<EventEnvelope>(value.clone()).map_err(|_| {
            PluginRegistryError::new(PluginRegistryErrorKind::InvalidLifecycleEvent)
        })?;
        let kind = event_kind(&envelope.event_type)?;
        validate_envelope(&envelope, kind)?;
        validate_context(kind, envelope.context)?;
        let canonical_json = canonical_json(value, MAX_EVENT_BYTES)?;
        Ok(Self {
            kind,
            schema_version: envelope.schema_version,
            event_id: envelope.event_id,
            refresh_id: envelope.refresh_id,
            sequence: envelope.sequence,
            occurred_at: envelope.occurred_at,
            idempotency_key: envelope.idempotency_key,
            canonical_json,
        })
    }

    #[must_use]
    pub const fn kind(&self) -> LifecycleEventKind {
        self.kind
    }

    #[must_use]
    pub const fn schema_version(&self) -> u32 {
        self.schema_version
    }

    #[must_use]
    pub fn event_id(&self) -> &str {
        &self.event_id
    }

    #[must_use]
    pub fn refresh_id(&self) -> &str {
        &self.refresh_id
    }

    #[must_use]
    pub const fn sequence(&self) -> u32 {
        self.sequence
    }

    #[must_use]
    pub fn occurred_at(&self) -> &str {
        &self.occurred_at
    }

    #[must_use]
    pub fn idempotency_key(&self) -> &str {
        &self.idempotency_key
    }

    #[must_use]
    pub fn canonical_json(&self) -> &str {
        &self.canonical_json
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct EventEnvelope {
    schema_version: u32,
    event_id: String,
    event_type: String,
    refresh_id: String,
    sequence: u32,
    occurred_at: String,
    idempotency_key: String,
    context: Value,
}

fn event_kind(value: &str) -> Result<LifecycleEventKind, PluginRegistryError> {
    match value {
        "feed.refresh.before" => Ok(LifecycleEventKind::FeedRefreshBefore),
        "feed.refresh.fetched" => Ok(LifecycleEventKind::FeedRefreshFetched),
        "entry.process" => Ok(LifecycleEventKind::EntryProcess),
        "feed.refresh.persisted" => Ok(LifecycleEventKind::FeedRefreshPersisted),
        "feed.refresh.completed" => Ok(LifecycleEventKind::FeedRefreshCompleted),
        _ => invalid(),
    }
}

fn validate_envelope(
    envelope: &EventEnvelope,
    kind: LifecycleEventKind,
) -> Result<(), PluginRegistryError> {
    if envelope.schema_version != 1 || envelope.sequence != kind.expected_sequence() {
        return invalid();
    }
    validate_uuid(
        &envelope.event_id,
        PluginRegistryErrorKind::InvalidLifecycleEvent,
    )?;
    validate_uuid(
        &envelope.refresh_id,
        PluginRegistryErrorKind::InvalidLifecycleEvent,
    )?;
    OffsetDateTime::parse(&envelope.occurred_at, &Rfc3339)
        .map_err(|_| PluginRegistryError::new(PluginRegistryErrorKind::InvalidLifecycleEvent))?;
    validate_visible_ascii(
        &envelope.idempotency_key,
        255,
        PluginRegistryErrorKind::InvalidLifecycleEvent,
    )?;
    let prefix = format!("refresh:{}:", envelope.refresh_id);
    if !envelope.idempotency_key.starts_with(&prefix) || !envelope.idempotency_key.ends_with(":v1")
    {
        return invalid();
    }
    Ok(())
}

fn validate_context(kind: LifecycleEventKind, value: Value) -> Result<(), PluginRegistryError> {
    match kind {
        LifecycleEventKind::FeedRefreshBefore => {
            let context = decode::<BeforeContext>(value)?;
            validate_uuid(
                &context.feed_id,
                PluginRegistryErrorKind::InvalidLifecycleEvent,
            )?;
            if !matches!(context.request_url.scheme.as_str(), "http" | "https")
                || context.request_url.host.is_empty()
                || context.request_url.host.len() > 253
                || !context.request_url.host.is_ascii()
                || context.request_url.host.chars().any(char::is_control)
            {
                return invalid();
            }
            validate_lower_hex_hash(
                &context.request_url.path_hash,
                PluginRegistryErrorKind::InvalidLifecycleEvent,
            )?;
            let _ = context.request_url.has_query;
            let _ = context.conditional_request.has_etag;
            let _ = context.conditional_request.has_last_modified;
            Ok(())
        }
        LifecycleEventKind::FeedRefreshFetched => {
            let context = decode::<FetchedContext>(value)?;
            validate_uuid(
                &context.feed_id,
                PluginRegistryErrorKind::InvalidLifecycleEvent,
            )?;
            if !(100..=599).contains(&context.status) || context.body_size_bytes > 2 * 1024 * 1024 {
                return invalid();
            }
            validate_visible_ascii(
                &context.media_type,
                128,
                PluginRegistryErrorKind::InvalidLifecycleEvent,
            )?;
            validate_visible_ascii(
                &context.body_handle,
                128,
                PluginRegistryErrorKind::InvalidLifecycleEvent,
            )?;
            let _ = context.has_etag;
            let _ = context.has_last_modified;
            Ok(())
        }
        LifecycleEventKind::EntryProcess => {
            let context = decode::<EntryProcessContext>(value)?;
            validate_uuid(
                &context.feed_id,
                PluginRegistryErrorKind::InvalidLifecycleEvent,
            )?;
            if context.candidate_ordinal > 9999
                || !matches!(
                    context.identity_kind.as_str(),
                    "GUID" | "CANONICAL_URL" | "FINGERPRINT"
                )
            {
                return invalid();
            }
            validate_lower_hex_hash(
                &context.identity_hash,
                PluginRegistryErrorKind::InvalidLifecycleEvent,
            )?;
            validate_text(
                &context.sanitized_entry.title,
                16 * 1024,
                PluginRegistryErrorKind::InvalidLifecycleEvent,
            )?;
            validate_optional_text(&context.sanitized_entry.summary_text, 64 * 1024)?;
            validate_optional_text(&context.sanitized_entry.content_text, 512 * 1024)?;
            if let Some(locale) = &context.sanitized_entry.source_locale {
                normalize_locale(locale, PluginRegistryErrorKind::InvalidLifecycleEvent)?;
            }
            Ok(())
        }
        LifecycleEventKind::FeedRefreshPersisted => {
            let context = decode::<PersistedContext>(value)?;
            validate_uuid(
                &context.feed_id,
                PluginRegistryErrorKind::InvalidLifecycleEvent,
            )?;
            if context.commit_generation < 0
                || context.new_count < 0
                || context.updated_count < 0
                || context.dropped_count < 0
                || usize::try_from(context.new_count).ok() != Some(context.new_entries.len())
                || usize::try_from(context.updated_count).ok()
                    != Some(context.updated_entries.len())
                || context.new_entries.len() > 10_000
                || context.updated_entries.len() > 10_000
            {
                return invalid();
            }
            validate_entry_refs(
                context
                    .new_entries
                    .iter()
                    .chain(context.updated_entries.iter()),
            )
        }
        LifecycleEventKind::FeedRefreshCompleted => {
            let context = decode::<CompletedContext>(value)?;
            validate_uuid(
                &context.feed_id,
                PluginRegistryErrorKind::InvalidLifecycleEvent,
            )?;
            if context.new_count < 0 || context.updated_count < 0 || context.dropped_count < 0 {
                return invalid();
            }
            match context.status.as_str() {
                "SUCCESS" | "NOT_MODIFIED" if context.error_code.is_none() => {}
                "PARTIAL" | "ERROR" => {
                    if let Some(code) = context.error_code.as_deref() {
                        validate_error_code(code)?;
                    }
                }
                _ => return invalid(),
            }
            let _ = context.duration_ms;
            Ok(())
        }
    }
}

fn decode<T: for<'de> Deserialize<'de>>(value: Value) -> Result<T, PluginRegistryError> {
    serde_json::from_value(value)
        .map_err(|_| PluginRegistryError::new(PluginRegistryErrorKind::InvalidLifecycleEvent))
}

fn validate_optional_text(value: &Option<String>, max: usize) -> Result<(), PluginRegistryError> {
    if let Some(value) = value {
        validate_text(value, max, PluginRegistryErrorKind::InvalidLifecycleEvent)?;
    }
    Ok(())
}

fn validate_entry_refs<'a>(
    values: impl Iterator<Item = &'a EntryReference>,
) -> Result<(), PluginRegistryError> {
    let mut seen = HashSet::new();
    for value in values {
        validate_uuid(
            &value.entry_id,
            PluginRegistryErrorKind::InvalidLifecycleEvent,
        )?;
        validate_lower_hex_hash(
            &value.content_hash,
            PluginRegistryErrorKind::InvalidLifecycleEvent,
        )?;
        if !seen.insert(&value.entry_id) {
            return invalid();
        }
    }
    Ok(())
}

fn validate_error_code(value: &str) -> Result<(), PluginRegistryError> {
    if !value.is_empty()
        && value.len() <= 64
        && value.as_bytes()[0].is_ascii_uppercase()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_')
    {
        Ok(())
    } else {
        invalid()
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct BeforeContext {
    feed_id: String,
    request_url: SafeUrlMetadata,
    conditional_request: ConditionalRequest,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct SafeUrlMetadata {
    scheme: String,
    host: String,
    path_hash: String,
    has_query: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct ConditionalRequest {
    has_etag: bool,
    has_last_modified: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct FetchedContext {
    feed_id: String,
    status: u16,
    media_type: String,
    body_handle: String,
    body_size_bytes: usize,
    has_etag: bool,
    has_last_modified: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct EntryProcessContext {
    feed_id: String,
    candidate_ordinal: u32,
    identity_kind: String,
    identity_hash: String,
    sanitized_entry: SanitizedEntry,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct SanitizedEntry {
    title: String,
    summary_text: Option<String>,
    content_text: Option<String>,
    source_locale: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct PersistedContext {
    feed_id: String,
    commit_generation: i64,
    new_count: i32,
    updated_count: i32,
    dropped_count: i32,
    new_entries: Vec<EntryReference>,
    updated_entries: Vec<EntryReference>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct EntryReference {
    entry_id: String,
    content_hash: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct CompletedContext {
    feed_id: String,
    status: String,
    new_count: i32,
    updated_count: i32,
    dropped_count: i32,
    duration_ms: u64,
    error_code: Option<String>,
}

fn invalid<T>() -> Result<T, PluginRegistryError> {
    Err(PluginRegistryError::new(
        PluginRegistryErrorKind::InvalidLifecycleEvent,
    ))
}
