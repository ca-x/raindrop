use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::OnceLock,
};

use wit_component::ComponentEncoder;

use raindrop::plugins::runtime::{CompiledPlugin, PluginRuntime};

use super::plugin::signed_bundle;

pub fn official_ai_component() -> &'static [u8] {
    static COMPONENT: OnceLock<Vec<u8>> = OnceLock::new();
    COMPONENT.get_or_init(build_component).as_slice()
}

pub fn compiled_official_ai_plugin(runtime: &PluginRuntime) -> CompiledPlugin {
    let component = official_ai_component();
    let bundle = signed_bundle("1.0.0", component);
    CompiledPlugin::compile(runtime, &bundle, component).expect("official component should compile")
}

fn build_component() -> Vec<u8> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let manifest = root.join("plugins/official/raindrop-ai-content/Cargo.toml");
    let target_dir = root.join("target/official-ai-component-test");
    let output = Command::new("rustup")
        .args([
            "run",
            "1.94.0",
            "cargo",
            "build",
            "--locked",
            "--manifest-path",
        ])
        .arg(&manifest)
        .args(["--target", "wasm32-unknown-unknown", "--release"])
        .current_dir(&root)
        .env_remove("RUSTUP_TOOLCHAIN")
        .env("CARGO_TARGET_DIR", &target_dir)
        .output()
        .expect("official guest build command should start");
    assert!(
        output.status.success(),
        "official guest build failed:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );

    let core_path = target_dir.join("wasm32-unknown-unknown/release/raindrop_ai_content.wasm");
    let core = fs::read(&core_path)
        .unwrap_or_else(|error| panic!("read {}: {error}", core_path.display()));
    let first = componentize(&core);
    let second = componentize(&core);
    assert_eq!(first, second, "componentization must be deterministic");

    let text = wasmprinter::print_bytes(&first).expect("component should print");
    assert!(text.contains("raindrop:content-plugin/host-ai@1.0.0"));
    assert!(text.contains("raindrop:content-plugin/host-mcp@1.0.0"));
    assert!(text.contains("raindrop:content-plugin/content-plugin@1.0.0"));
    assert!(
        !text.contains("wasi:"),
        "official component must import no WASI"
    );
    assert_guest_source_is_confined(&root);
    first
}

fn componentize(core: &[u8]) -> Vec<u8> {
    ComponentEncoder::default()
        .module(core)
        .expect("guest core module should contain component metadata")
        .validate(true)
        .encode()
        .expect("guest core module should componentize")
}

fn assert_guest_source_is_confined(root: &Path) {
    let source_root = root.join("plugins/official/raindrop-ai-content/src");
    let mut source = String::new();
    for file in [
        "component.rs",
        "config.rs",
        "json.rs",
        "lib.rs",
        "lifecycle.rs",
        "operation.rs",
        "prompt.rs",
        "tool_plan.rs",
    ] {
        source.push_str(
            &fs::read_to_string(source_root.join(file))
                .unwrap_or_else(|error| panic!("read guest source {file}: {error}")),
        );
    }
    for forbidden in [
        "std::fs",
        "std::env",
        "std::net",
        "std::process",
        "reqwest",
        "tokio",
        "sea_orm",
        "DatabaseConnection",
        "ProviderClient",
        "McpCapabilityBroker",
        "Command::new",
    ] {
        assert!(
            !source.contains(forbidden),
            "forbidden guest source path: {forbidden}",
        );
    }
}
