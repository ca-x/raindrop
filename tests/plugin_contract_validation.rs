use std::{fs, path::PathBuf};

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use ring::{
    digest::{SHA256, digest},
    signature::{Ed25519KeyPair, KeyPair as _},
};
use serde_json::{Value, json};

use raindrop::plugins::{
    AiContentConfig, BundledOfficialPlugin, LifecycleEvent, LifecycleEventKind, OfficialSigningKey,
    PluginRegistryErrorKind, SummaryArtifact, TranslationArtifact,
};

const TEST_KEY_ID: &str = "raindrop-test-release-2026";
const TEST_SEED: [u8; 32] = [0x42; 32];
const COMPONENT: &[u8] = b"raindrop.ai-content deterministic component fixture v1";

#[test]
fn production_manifest_requires_exact_digest_identity_and_ed25519_origin() {
    let (manifest, key) = signed_manifest(COMPONENT);
    let bundle = BundledOfficialPlugin::verify(&manifest, COMPONENT, std::slice::from_ref(&key))
        .expect("signed official bundle should verify");
    assert_eq!(bundle.plugin_key(), "raindrop.ai-content");
    assert_eq!(bundle.version(), "1.0.0");
    assert_eq!(bundle.abi_version(), "raindrop:content-plugin@1.0.0");
    assert_eq!(bundle.signature_key_id(), TEST_KEY_ID);
    assert_eq!(bundle.component_digest(), sha256_hex(COMPONENT));
    assert!(bundle.manifest_json().contains("\"value\""));
    assert!(!bundle.manifest_json().contains("valueSource"));

    let duplicate = br#"{"manifestVersion":1,"manifestVersion":1}"#;
    assert_error(
        BundledOfficialPlugin::verify(duplicate, COMPONENT, std::slice::from_ref(&key)),
        PluginRegistryErrorKind::DuplicateJsonKey,
        "manifestVersion",
    );

    let template = fs::read(
        workspace_root().join("contracts/plugins/raindrop.ai-content/manifest.template.json"),
    )
    .expect("manifest template should read");
    assert_error(
        BundledOfficialPlugin::verify(&template, COMPONENT, std::slice::from_ref(&key)),
        PluginRegistryErrorKind::InvalidManifest,
        "RELEASE_BUILD",
    );

    let mut wrong_digest: Value = serde_json::from_slice(&manifest).unwrap();
    wrong_digest["componentDigest"]["value"] = json!("0".repeat(64));
    assert_error(
        BundledOfficialPlugin::verify(
            &serde_json::to_vec(&wrong_digest).unwrap(),
            COMPONENT,
            std::slice::from_ref(&key),
        ),
        PluginRegistryErrorKind::ComponentDigestMismatch,
        &sha256_hex(COMPONENT),
    );

    let mut unknown_key: Value = serde_json::from_slice(&manifest).unwrap();
    unknown_key["signature"]["keyId"] = json!("unknown-secret-key-id");
    assert_error(
        BundledOfficialPlugin::verify(
            &serde_json::to_vec(&unknown_key).unwrap(),
            COMPONENT,
            std::slice::from_ref(&key),
        ),
        PluginRegistryErrorKind::UnknownSigningKey,
        "unknown-secret-key-id",
    );

    let mut bad_signature: Value = serde_json::from_slice(&manifest).unwrap();
    bad_signature["signature"]["value"] = json!(URL_SAFE_NO_PAD.encode([0x55; 64]));
    assert_error(
        BundledOfficialPlugin::verify(
            &serde_json::to_vec(&bad_signature).unwrap(),
            COMPONENT,
            std::slice::from_ref(&key),
        ),
        PluginRegistryErrorKind::InvalidSignature,
        &URL_SAFE_NO_PAD.encode([0x55; 64]),
    );

    let mut invalid_identity: Value = serde_json::from_slice(&manifest).unwrap();
    invalid_identity["pluginKey"] = json!("attacker.secret-plugin");
    assert_error(
        BundledOfficialPlugin::verify(
            &serde_json::to_vec(&invalid_identity).unwrap(),
            COMPONENT,
            std::slice::from_ref(&key),
        ),
        PluginRegistryErrorKind::InvalidManifest,
        "attacker.secret-plugin",
    );
}

#[test]
fn ai_config_is_strict_canonical_and_enforces_cross_field_rules() {
    let input = valid_config();
    let parsed = AiContentConfig::parse(&serde_json::to_vec(&input).unwrap())
        .expect("valid AI config should parse");
    assert_eq!(parsed.schema_version(), 1);
    assert_eq!(
        parsed.summarize_provider_id(),
        "00000000-0000-4000-8000-000000000901"
    );
    assert_eq!(parsed.default_target_locale(), "zh-CN");
    assert_eq!(parsed.config_hash().len(), 64);
    assert_eq!(
        parsed.canonical_json(),
        serde_json::to_string(&input).unwrap()
    );

    let mut unknown = valid_config();
    unknown["apiKey"] = json!("rd-secret-config-value");
    assert_error(
        AiContentConfig::parse(&serde_json::to_vec(&unknown).unwrap()),
        PluginRegistryErrorKind::InvalidConfig,
        "rd-secret-config-value",
    );

    let duplicate = serde_json::to_string(&valid_config()).unwrap().replacen(
        "\"schemaVersion\":1",
        "\"schemaVersion\":1,\"schemaVersion\":1",
        1,
    );
    assert_error(
        AiContentConfig::parse(duplicate.as_bytes()),
        PluginRegistryErrorKind::DuplicateJsonKey,
        "schemaVersion",
    );

    for invalid in invalid_configs() {
        assert_error(
            AiContentConfig::parse(&serde_json::to_vec(&invalid).unwrap()),
            PluginRegistryErrorKind::InvalidConfig,
            "rd-secret-config-value",
        );
    }

    assert_error(
        AiContentConfig::parse(&vec![b' '; 256 * 1024 + 1]),
        PluginRegistryErrorKind::PayloadTooLarge,
        "secret",
    );
}

#[test]
fn artifact_payloads_are_strict_bounded_and_safe_for_later_rendering() {
    let summary_json = json!({
        "schemaVersion": 1,
        "sourceLanguage": "en",
        "summary": "A concise summary.",
        "bullets": ["First point", "Second point"],
        "conclusion": null
    });
    let summary = SummaryArtifact::parse(&serde_json::to_vec(&summary_json).unwrap())
        .expect("summary should parse");
    assert_eq!(summary.source_language(), "en");
    assert_eq!(summary.bullets().len(), 2);

    let translation_json = json!({
        "schemaVersion": 1,
        "detectedSourceLanguage": "en",
        "targetLocale": "zh-CN",
        "title": "示例标题",
        "bodyMarkdown": "安全的 **Markdown** 内容。"
    });
    let translation = TranslationArtifact::parse(&serde_json::to_vec(&translation_json).unwrap())
        .expect("translation should parse");
    assert_eq!(translation.detected_source_language(), "en");
    assert_eq!(translation.target_locale(), "zh-CN");

    let invalid = [
        json!({
            "schemaVersion": 1,
            "sourceLanguage": "en",
            "summary": "<script>rd-secret-artifact</script>",
            "bullets": [],
            "conclusion": null
        }),
        json!({
            "schemaVersion": 1,
            "sourceLanguage": "en",
            "summary": "Safe",
            "bullets": ["javascript:rd-secret-artifact"],
            "conclusion": null,
            "unknown": true
        }),
    ];
    for payload in invalid {
        assert_error(
            SummaryArtifact::parse(&serde_json::to_vec(&payload).unwrap()),
            PluginRegistryErrorKind::InvalidArtifact,
            "rd-secret-artifact",
        );
    }

    let mut unsafe_translation = translation_json.clone();
    unsafe_translation["bodyMarkdown"] = json!("[unsafe](data:text/html,rd-secret-artifact)");
    assert_error(
        TranslationArtifact::parse(&serde_json::to_vec(&unsafe_translation).unwrap()),
        PluginRegistryErrorKind::InvalidArtifact,
        "rd-secret-artifact",
    );

    let mut invalid_locale = translation_json;
    invalid_locale["targetLocale"] = json!("zh_cn_secret");
    assert_error(
        TranslationArtifact::parse(&serde_json::to_vec(&invalid_locale).unwrap()),
        PluginRegistryErrorKind::InvalidArtifact,
        "zh_cn_secret",
    );
}

#[test]
fn lifecycle_fixtures_have_exact_event_specific_contexts() {
    let fixtures = [
        (
            "feed.refresh.before.json",
            LifecycleEventKind::FeedRefreshBefore,
        ),
        (
            "feed.refresh.fetched.json",
            LifecycleEventKind::FeedRefreshFetched,
        ),
        ("entry.process.json", LifecycleEventKind::EntryProcess),
        (
            "feed.refresh.persisted.json",
            LifecycleEventKind::FeedRefreshPersisted,
        ),
        (
            "feed.refresh.completed.json",
            LifecycleEventKind::FeedRefreshCompleted,
        ),
    ];
    let root = workspace_root().join("contracts/lifecycle/feed-refresh-v1");
    for (file, expected_kind) in fixtures {
        let bytes = fs::read(root.join(file)).expect("lifecycle fixture should read");
        let event = LifecycleEvent::parse(&bytes).expect("lifecycle fixture should validate");
        assert_eq!(event.kind(), expected_kind);
        assert_eq!(event.schema_version(), 1);
    }

    let persisted = fs::read(root.join("feed.refresh.persisted.json")).unwrap();
    let mut wrong_sequence: Value = serde_json::from_slice(&persisted).unwrap();
    wrong_sequence["sequence"] = json!(20);
    assert_error(
        LifecycleEvent::parse(&serde_json::to_vec(&wrong_sequence).unwrap()),
        PluginRegistryErrorKind::InvalidLifecycleEvent,
        "secret",
    );

    let mut leaked_body: Value = serde_json::from_slice(&persisted).unwrap();
    leaked_body["context"]["rawBody"] = json!("rd-secret-feed-body");
    assert_error(
        LifecycleEvent::parse(&serde_json::to_vec(&leaked_body).unwrap()),
        PluginRegistryErrorKind::InvalidLifecycleEvent,
        "rd-secret-feed-body",
    );
}

fn invalid_configs() -> Vec<Value> {
    let mut invalid_provider = valid_config();
    invalid_provider["operations"]["summarize"]["providerId"] = json!("rd-secret-config-value");

    let mut disabled_mcp_calls = valid_config();
    disabled_mcp_calls["operations"]["summarize"]["mcp"]["maxToolCalls"] = json!(1);

    let mut enabled_mcp_without_tools = valid_config();
    enabled_mcp_without_tools["operations"]["summarize"]["mcp"]["mode"] =
        json!("CONTEXT_ENRICHMENT");
    enabled_mcp_without_tools["operations"]["summarize"]["mcp"]["maxToolCalls"] = json!(1);

    let mut duplicate_tools = valid_config();
    let tool = json!({
        "connectionId": "00000000-0000-4000-8000-000000000801",
        "toolName": "lookup.article"
    });
    duplicate_tools["operations"]["summarize"]["mcp"] = json!({
        "mode": "CONTEXT_ENRICHMENT",
        "failurePolicy": "FAIL_OPEN",
        "maxToolCalls": 2,
        "tools": [tool.clone(), tool]
    });

    let mut automatic_without_scope = valid_config();
    automatic_without_scope["automatic"]["enabled"] = json!(true);

    let mut disabled_automatic_operation = valid_config();
    disabled_automatic_operation["automatic"]["enabled"] = json!(true);
    disabled_automatic_operation["automatic"]["allSubscribedFeeds"] = json!(true);
    disabled_automatic_operation["automatic"]["operations"] = json!(["TRANSLATE"]);
    disabled_automatic_operation["operations"]["translate"]["enabled"] = json!(false);

    vec![
        invalid_provider,
        disabled_mcp_calls,
        enabled_mcp_without_tools,
        duplicate_tools,
        automatic_without_scope,
        disabled_automatic_operation,
    ]
}

fn valid_config() -> Value {
    json!({
        "schemaVersion": 1,
        "operations": {
            "summarize": {
                "enabled": true,
                "providerId": "00000000-0000-4000-8000-000000000901",
                "style": "BALANCED",
                "maxOutputTokens": 1024,
                "mcp": {
                    "mode": "DISABLED",
                    "failurePolicy": "FAIL_OPEN",
                    "maxToolCalls": 0,
                    "tools": []
                }
            },
            "translate": {
                "enabled": true,
                "providerId": "00000000-0000-4000-8000-000000000902",
                "defaultTargetLocale": "zh-CN",
                "maxOutputTokens": 2048,
                "mcp": {
                    "mode": "DISABLED",
                    "failurePolicy": "FAIL_CLOSED",
                    "maxToolCalls": 0,
                    "tools": []
                }
            }
        },
        "automatic": {
            "enabled": false,
            "operations": ["SUMMARIZE"],
            "allSubscribedFeeds": false,
            "feedIds": [],
            "categoryIds": []
        }
    })
}

fn signed_manifest(component: &[u8]) -> (Vec<u8>, OfficialSigningKey) {
    let digest = sha256_hex(component);
    let canonical_without_signature = format!(
        "{{\"abi\":\"raindrop:content-plugin@1.0.0\",\"ambientPermissions\":[],\"artifactSchemas\":[\"raindrop://schemas/artifacts/ai-summary/v1\",\"raindrop://schemas/artifacts/ai-translation/v1\"],\"capabilities\":{{\"optional\":[\"mcp.call_tool\"],\"required\":[\"ai.generate_structured\"]}},\"componentDigest\":{{\"algorithm\":\"SHA-256\",\"value\":\"{digest}\",\"valueEncoding\":\"LOWER_HEX\"}},\"configSchema\":\"raindrop://schemas/plugins/raindrop.ai-content/config/v1\",\"distribution\":\"BUNDLED_OFFICIAL\",\"lifecycleSubscriptions\":[{{\"event\":\"feed.refresh.persisted\",\"schemaVersion\":1}}],\"manifestVersion\":1,\"operations\":[\"summarize\",\"translate\"],\"pluginKey\":\"raindrop.ai-content\",\"signature\":{{\"algorithm\":\"Ed25519\",\"keyId\":\"{TEST_KEY_ID}\",\"valueEncoding\":\"BASE64URL\"}},\"version\":\"1.0.0\"}}"
    );
    let payload = signature_payload(canonical_without_signature.as_bytes(), &digest);
    let key_pair = Ed25519KeyPair::from_seed_unchecked(&TEST_SEED).unwrap();
    let signature = URL_SAFE_NO_PAD.encode(key_pair.sign(&payload).as_ref());
    let mut manifest: Value = serde_json::from_str(&canonical_without_signature).unwrap();
    manifest["signature"]["value"] = json!(signature);
    let public_key: [u8; 32] = key_pair.public_key().as_ref().try_into().unwrap();
    (
        serde_json::to_vec(&manifest).unwrap(),
        OfficialSigningKey::new(TEST_KEY_ID, public_key).unwrap(),
    )
}

fn signature_payload(canonical_manifest: &[u8], digest: &str) -> Vec<u8> {
    let mut payload = b"raindrop.plugin-signature.v1".to_vec();
    payload.extend_from_slice(&(canonical_manifest.len() as u64).to_be_bytes());
    payload.extend_from_slice(canonical_manifest);
    payload.extend_from_slice(&(digest.len() as u64).to_be_bytes());
    payload.extend_from_slice(digest.as_bytes());
    payload
}

fn sha256_hex(bytes: &[u8]) -> String {
    digest(&SHA256, bytes)
        .as_ref()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn assert_error<T: std::fmt::Debug>(
    result: Result<T, raindrop::plugins::PluginRegistryError>,
    expected: PluginRegistryErrorKind,
    secret: &str,
) {
    let error = result.expect_err("input should fail closed");
    assert_eq!(error.kind(), expected);
    let rendered = format!("{error:?} {error}");
    assert!(!rendered.contains(secret));
}

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}
