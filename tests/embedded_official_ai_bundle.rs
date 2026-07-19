use raindrop::plugins::{EmbeddedOfficialAiPlugin, EmbeddedSignatureMode, runtime::PluginRuntime};

#[tokio::test]
async fn embedded_official_ai_bundle_verifies_compiles_and_has_no_ambient_wasi() {
    let embedded =
        EmbeddedOfficialAiPlugin::load().expect("embedded official bundle should verify");

    let expected_mode = match env!("RAINDROP_OFFICIAL_PLUGIN_SIGNATURE_MODE") {
        "DEVELOPMENT" => EmbeddedSignatureMode::Development,
        "OFFICIAL" => EmbeddedSignatureMode::Official,
        value => panic!("unexpected embedded signature mode: {value}"),
    };
    assert_eq!(embedded.signature_mode(), expected_mode);
    assert_eq!(embedded.bundle().plugin_key(), "raindrop.ai-content");
    assert_eq!(embedded.bundle().version(), "1.0.0");
    assert_eq!(
        embedded.bundle().abi_version(),
        "raindrop:content-plugin@1.0.0"
    );
    assert_eq!(
        embedded.bundle().signature_key_id(),
        env!("RAINDROP_OFFICIAL_PLUGIN_SIGNING_KEY_ID")
    );
    assert!(matches!(
        (expected_mode, embedded.bundle().signature_key_id()),
        (
            EmbeddedSignatureMode::Development,
            "raindrop-development-2026"
        ) | (EmbeddedSignatureMode::Official, "raindrop-release-2026")
    ));
    assert_eq!(embedded.bundle().component_digest().len(), 64);
    assert!(!embedded.component().is_empty());

    let component_text =
        wasmprinter::print_bytes(embedded.component()).expect("embedded component should print");
    assert!(component_text.contains("raindrop:content-plugin/content-plugin@1.0.0"));
    assert!(!component_text.contains("wasi:"));

    let runtime = PluginRuntime::new().expect("plugin runtime should initialize");
    let compiled = embedded
        .compile(&runtime)
        .expect("embedded official component should compile");
    assert_eq!(compiled.plugin_key(), embedded.bundle().plugin_key());
    assert_eq!(compiled.version(), embedded.bundle().version());
    assert_eq!(
        compiled.component_digest(),
        embedded.bundle().component_digest()
    );
}
