use std::{fs, path::PathBuf};

use serde_json::Value;
use wit_parser::Resolve;

const MANIFEST_TEMPLATE: &str = "contracts/plugins/raindrop.ai-content/manifest.template.json";
const CONFIG_SCHEMA: &str = "contracts/plugins/raindrop.ai-content/config.v1.schema.json";
const SUMMARY_SCHEMA: &str = "contracts/artifacts/ai-summary.v1.schema.json";
const TRANSLATION_SCHEMA: &str = "contracts/artifacts/ai-translation.v1.schema.json";

const LIFECYCLE_FIXTURES: &[(&str, &str, i64)] = &[
    ("feed.refresh.before.json", "feed.refresh.before", 1),
    ("feed.refresh.fetched.json", "feed.refresh.fetched", 5),
    ("entry.process.json", "entry.process", 8),
    ("feed.refresh.persisted.json", "feed.refresh.persisted", 10),
    ("feed.refresh.completed.json", "feed.refresh.completed", 20),
];

#[test]
fn content_plugin_wit_package_resolves_the_frozen_world() {
    let root = workspace_root().join("contracts/wit/raindrop-content-plugin-v1");
    let mut resolve = Resolve::default();
    let (package_id, _) = resolve.push_dir(&root).unwrap_or_else(|error| {
        panic!("WIT package should parse at {}: {error:#}", root.display())
    });

    assert_eq!(
        resolve.packages[package_id].name.to_string(),
        "raindrop:content-plugin@1.0.0"
    );
    let world_id = resolve
        .select_world(&[package_id], Some("content-plugin-v1"))
        .expect("content-plugin-v1 should resolve");
    let world = &resolve.worlds[world_id];
    assert_eq!(world.name, "content-plugin-v1");
    assert_eq!(world.imports.len(), 3, "types is a transitive import");
    assert_eq!(world.exports.len(), 1);
}

#[test]
fn official_manifest_template_is_not_a_production_installation() {
    let manifest = read_json(MANIFEST_TEMPLATE);
    assert_eq!(manifest["manifestVersion"], 1);
    assert_eq!(manifest["pluginKey"], "raindrop.ai-content");
    assert_eq!(manifest["version"], "1.0.0");
    assert_eq!(manifest["abi"], "raindrop:content-plugin@1.0.0");
    assert_eq!(manifest["distribution"], "BUNDLED_OFFICIAL");
    assert_eq!(manifest["componentDigest"]["valueSource"], "RELEASE_BUILD");
    assert!(manifest["componentDigest"].get("value").is_none());
    assert_eq!(manifest["signature"]["valueSource"], "RELEASE_BUILD");
    assert!(manifest["signature"].get("value").is_none());
}

#[test]
fn config_and_artifact_schema_ids_are_frozen() {
    let fixtures = [
        (
            CONFIG_SCHEMA,
            "raindrop://schemas/plugins/raindrop.ai-content/config/v1",
        ),
        (SUMMARY_SCHEMA, "raindrop://schemas/artifacts/ai-summary/v1"),
        (
            TRANSLATION_SCHEMA,
            "raindrop://schemas/artifacts/ai-translation/v1",
        ),
    ];

    for (path, expected_id) in fixtures {
        let schema = read_json(path);
        assert_eq!(
            schema["$schema"],
            "https://json-schema.org/draft/2020-12/schema"
        );
        assert_eq!(schema["$id"], expected_id);
        assert_eq!(schema["type"], "object");
        assert_eq!(schema["additionalProperties"], false);
    }
}

#[test]
fn all_five_lifecycle_fixtures_share_the_versioned_envelope() {
    let root = workspace_root().join("contracts/lifecycle/feed-refresh-v1");
    for (file, event_type, sequence) in LIFECYCLE_FIXTURES {
        let bytes = fs::read(root.join(file))
            .unwrap_or_else(|error| panic!("lifecycle fixture {file} should read: {error}"));
        let event: Value = serde_json::from_slice(&bytes)
            .unwrap_or_else(|error| panic!("lifecycle fixture {file} should parse: {error}"));
        assert_eq!(event["schemaVersion"], 1, "fixture {file}");
        assert_eq!(event["eventType"], *event_type, "fixture {file}");
        assert_eq!(event["sequence"], *sequence, "fixture {file}");
        assert!(event["eventId"].as_str().is_some(), "fixture {file}");
        assert!(event["refreshId"].as_str().is_some(), "fixture {file}");
        assert!(event["occurredAt"].as_str().is_some(), "fixture {file}");
        assert!(event["idempotencyKey"].as_str().is_some(), "fixture {file}");
        assert!(event["context"].is_object(), "fixture {file}");
    }
}

fn read_json(path: &str) -> Value {
    let full_path = workspace_root().join(path);
    let bytes = fs::read(&full_path).unwrap_or_else(|error| {
        panic!("JSON asset should read at {}: {error}", full_path.display())
    });
    serde_json::from_slice(&bytes).unwrap_or_else(|error| {
        panic!(
            "JSON asset should parse at {}: {error}",
            full_path.display()
        )
    })
}

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}
