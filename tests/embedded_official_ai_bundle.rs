use raindrop::plugins::{EmbeddedOfficialAiPlugin, EmbeddedSignatureMode, runtime::PluginRuntime};

#[tokio::test]
async fn embedded_official_ai_native_bundle_verifies_registers_and_shuts_down() {
    let embedded = EmbeddedOfficialAiPlugin::load().expect("embedded native bundle should verify");
    let expected_mode = match env!("RAINDROP_OFFICIAL_PLUGIN_SIGNATURE_MODE") {
        "DEVELOPMENT" => EmbeddedSignatureMode::Development,
        "OFFICIAL" => EmbeddedSignatureMode::Official,
        value => panic!("unexpected embedded signature mode: {value}"),
    };
    assert_eq!(embedded.signature_mode(), expected_mode);
    assert_eq!(
        embedded.bundle().plugin_key(),
        raindrop_ai_content::PLUGIN_KEY
    );
    assert_eq!(embedded.bundle().version(), raindrop_ai_content::VERSION);
    assert_eq!(embedded.bundle().abi_version(), raindrop_ai_content::ABI);
    assert_eq!(
        embedded.bundle().signature_key_id(),
        env!("RAINDROP_OFFICIAL_PLUGIN_SIGNING_KEY_ID")
    );
    assert_eq!(embedded.bundle().component_digest().len(), 64);
    let inputs: serde_json::Value = serde_json::from_slice(EmbeddedOfficialAiPlugin::identity())
        .expect("signed native build inputs");
    assert!(
        inputs
            .get("plugins/official/raindrop-ai-content/src/native.rs")
            .is_some()
    );
    assert!(!EmbeddedOfficialAiPlugin::identity().starts_with(b"\0asm"));
    let manifest: serde_json::Value =
        serde_json::from_str(embedded.bundle().manifest_json()).expect("native manifest");
    assert_eq!(
        manifest["ambientPermissions"],
        serde_json::json!(["TRUSTED_NATIVE"])
    );
    let runtime = PluginRuntime::new().expect("Rutis runtime");
    let registered = embedded
        .register(&runtime)
        .expect("linked official registration");
    runtime
        .ready()
        .await
        .expect("official service should become active");
    assert_eq!(
        registered.component_digest(),
        embedded.bundle().component_digest()
    );
    runtime
        .shutdown()
        .await
        .expect("Rutis shutdown should finish");
    assert!(runtime.ready().await.is_err());
}
