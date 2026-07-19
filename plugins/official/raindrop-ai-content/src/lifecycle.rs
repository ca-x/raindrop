use std::collections::HashSet;

use serde::Deserialize;

use crate::{
    Failure,
    config::{Config, OperationKind},
    json::parse_canonical_object,
};

const MAX_CONTEXT_BYTES: usize = 64 * 1024;

pub(crate) struct LifecycleInput<'a> {
    pub(crate) event_id: &'a str,
    pub(crate) subject: &'a str,
    pub(crate) config_hash: &'a str,
    pub(crate) context_json: &'a str,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct JobIntent {
    pub(crate) operation: OperationKind,
    pub(crate) entry_id: String,
    pub(crate) target_locale: Option<String>,
    pub(crate) idempotency_key: String,
}

pub(crate) fn build_intents(
    config: &Config,
    input: LifecycleInput<'_>,
) -> Result<Vec<JobIntent>, Failure> {
    if !config.automatic.enabled {
        return Ok(Vec::new());
    }
    let value = parse_canonical_object(input.context_json, MAX_CONTEXT_BYTES)
        .map_err(|_| Failure::ConfigInvalid)?;
    let context =
        serde_json::from_value::<PersistedContext>(value).map_err(|_| Failure::ConfigInvalid)?;
    if context.commit_generation < 0
        || context.new_count < 0
        || context.updated_count < 0
        || context.dropped_count < 0
        || usize::try_from(context.new_count).ok() != Some(context.new_entries.len())
        || usize::try_from(context.updated_count).ok() != Some(context.updated_entries.len())
    {
        return Err(Failure::ConfigInvalid);
    }
    if !feed_is_eligible(config, &context.feed_id) {
        return Ok(Vec::new());
    }
    let mut seen = HashSet::new();
    let mut intents = Vec::new();
    for entry in context
        .new_entries
        .into_iter()
        .chain(context.updated_entries)
    {
        if !valid_uuid(&entry.entry_id) || !valid_lower_hash(&entry.content_hash) {
            return Err(Failure::ConfigInvalid);
        }
        if !seen.insert(entry.entry_id.clone()) {
            continue;
        }
        for operation in &config.automatic.operations {
            let target_locale = match operation {
                OperationKind::Summarize => None,
                OperationKind::Translate => {
                    Some(config.operations.translate.default_target_locale.clone())
                }
            };
            intents.push(JobIntent {
                operation: *operation,
                idempotency_key: format!(
                    "event:{}:plugin:raindrop.ai-content:user:{}:entry:{}:op:{}:config:{}",
                    input.event_id,
                    input.subject,
                    entry.entry_id,
                    operation.as_key(),
                    input.config_hash,
                ),
                entry_id: entry.entry_id.clone(),
                target_locale,
            });
        }
    }
    Ok(intents)
}

fn feed_is_eligible(config: &Config, feed_id: &str) -> bool {
    config.automatic.all_subscribed_feeds
        || config
            .automatic
            .feed_ids
            .iter()
            .any(|value| value == feed_id)
        || !config.automatic.category_ids.is_empty()
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

fn valid_lower_hash(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::{config::tests::fixture_config, json::canonical_json};

    #[test]
    fn lifecycle_deduplicates_in_feed_order_and_emits_configured_operation_order() {
        let config = Config::parse(&fixture_config()).expect("config");
        let first = "00000000-0000-4000-8000-000000000401";
        let second = "00000000-0000-4000-8000-000000000402";
        let context = canonical_json(
            json!({
                "feedId": "00000000-0000-4000-8000-000000000301",
                "commitGeneration": 42,
                "newCount": 2,
                "updatedCount": 2,
                "droppedCount": 0,
                "newEntries": [
                    {"entryId": first, "contentHash": "a".repeat(64)},
                    {"entryId": second, "contentHash": "b".repeat(64)},
                ],
                "updatedEntries": [
                    {"entryId": first, "contentHash": "a".repeat(64)},
                    {"entryId": second, "contentHash": "b".repeat(64)},
                ],
            }),
            MAX_CONTEXT_BYTES,
        )
        .expect("context");
        let intents = build_intents(
            &config,
            LifecycleInput {
                event_id: "00000000-0000-4000-8000-000000000501",
                subject: "user-1",
                config_hash: &"d".repeat(64),
                context_json: &context,
            },
        )
        .expect("intents");
        assert_eq!(intents.len(), 4);
        assert_eq!(intents[0].entry_id, first);
        assert_eq!(intents[0].operation, OperationKind::Summarize);
        assert_eq!(intents[1].operation, OperationKind::Translate);
        assert_eq!(intents[1].target_locale.as_deref(), Some("zh-CN"));
        assert_eq!(intents[2].entry_id, second);
        assert_eq!(
            intents[0].idempotency_key,
            format!(
                "event:00000000-0000-4000-8000-000000000501:plugin:raindrop.ai-content:user:user-1:entry:{first}:op:summarize:config:{}",
                "d".repeat(64),
            )
        );
    }

    #[test]
    fn lifecycle_returns_no_intents_for_an_explicitly_unselected_feed() {
        let mut value: serde_json::Value =
            serde_json::from_str(&fixture_config()).expect("config JSON");
        value["automatic"]["allSubscribedFeeds"] = json!(false);
        value["automatic"]["feedIds"] = json!(["00000000-0000-4000-8000-000000000399"]);
        let config =
            Config::parse(&canonical_json(value, 256 * 1024).expect("selected-feed config"))
                .expect("config");
        let context = canonical_json(
            json!({
                "feedId": "00000000-0000-4000-8000-000000000301",
                "commitGeneration": 1,
                "newCount": 0,
                "updatedCount": 0,
                "droppedCount": 0,
                "newEntries": [],
                "updatedEntries": [],
            }),
            MAX_CONTEXT_BYTES,
        )
        .expect("context");
        assert!(
            build_intents(
                &config,
                LifecycleInput {
                    event_id: "00000000-0000-4000-8000-000000000501",
                    subject: "user-1",
                    config_hash: &"d".repeat(64),
                    context_json: &context,
                },
            )
            .expect("intents")
            .is_empty()
        );
    }
}
