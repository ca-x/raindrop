#[allow(dead_code)]
mod support;

use raindrop::plugins::runtime::{
    CompiledPlugin, PluginRuntime, PluginRuntimeError, PluginRuntimeErrorKind,
};
use support::{plugin::signed_bundle, plugin_component::component_fixture};

const MAX_COMPONENT_BYTES: usize = 16 * 1024 * 1024;

#[tokio::test]
async fn verified_binary_component_compiles_with_signed_identity() {
    let runtime = PluginRuntime::new().expect("Tokio runtime should host the epoch ticker");
    let component = component_fixture();
    let bundle = signed_bundle("1.0.0", component);

    let compiled = CompiledPlugin::compile(&runtime, &bundle, component)
        .expect("matching signed component should compile");

    assert_eq!(compiled.plugin_key(), "raindrop.ai-content");
    assert_eq!(compiled.version(), "1.0.0");
    assert_eq!(compiled.abi_version(), "raindrop:content-plugin@1.0.0");
    assert_eq!(compiled.component_digest(), bundle.component_digest());
}

#[tokio::test]
async fn verified_compile_rejects_digest_tampering_before_wasmtime() {
    let runtime = PluginRuntime::new().expect("runtime should construct");
    let component = component_fixture();
    let bundle = signed_bundle("1.0.0", component);
    let mut tampered = component.to_vec();
    let last = tampered.last_mut().expect("fixture should not be empty");
    *last ^= 0x01;

    assert_error(
        CompiledPlugin::compile(&runtime, &bundle, &tampered),
        PluginRuntimeErrorKind::ComponentDigestMismatch,
        "component bytes",
    );
}

#[tokio::test]
async fn verified_compile_rejects_empty_oversized_malformed_and_wat_inputs() {
    let runtime = PluginRuntime::new().expect("runtime should construct");
    let valid_component = component_fixture();
    let valid_bundle = signed_bundle("1.0.0", valid_component);
    assert_error(
        CompiledPlugin::compile(&runtime, &valid_bundle, &[]),
        PluginRuntimeErrorKind::InvalidComponent,
        "empty component",
    );

    let oversized = vec![0x5a; MAX_COMPONENT_BYTES + 1];
    let oversized_bundle = signed_bundle("1.0.0", &oversized);
    assert_error(
        CompiledPlugin::compile(&runtime, &oversized_bundle, &oversized),
        PluginRuntimeErrorKind::InvalidComponent,
        "oversized component",
    );

    let malformed = b"rd-secret-malformed-component";
    let malformed_bundle = signed_bundle("1.0.0", malformed);
    assert_error(
        CompiledPlugin::compile(&runtime, &malformed_bundle, malformed),
        PluginRuntimeErrorKind::InvalidComponent,
        "rd-secret-malformed-component",
    );

    let wat = b"(component ;; rd-secret-wat-component )";
    let wat_bundle = signed_bundle("1.0.0", wat);
    assert_error(
        CompiledPlugin::compile(&runtime, &wat_bundle, wat),
        PluginRuntimeErrorKind::InvalidComponent,
        "rd-secret-wat-component",
    );
}

#[test]
fn runtime_constructor_fails_closed_without_tokio() {
    assert_error(
        PluginRuntime::new(),
        PluginRuntimeErrorKind::RuntimeUnavailable,
        "runtime internals",
    );
}

fn assert_error<T>(
    result: Result<T, PluginRuntimeError>,
    expected: PluginRuntimeErrorKind,
    secret: &str,
) {
    let error = match result {
        Ok(_) => panic!("expected {expected:?}"),
        Err(error) => error,
    };
    assert_eq!(error.kind(), expected);
    let rendered = format!("{error:?} {error}");
    assert!(!rendered.contains(secret));
}
