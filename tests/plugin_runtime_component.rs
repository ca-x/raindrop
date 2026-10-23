#[allow(dead_code)]
mod support;

use raindrop::plugins::{
    EmbeddedOfficialAiPlugin,
    runtime::{NativePlugin, PluginRuntime, PluginRuntimeErrorKind},
};
use support::plugin::signed_bundle;

#[tokio::test]
async fn native_registration_rejects_arbitrary_or_tampered_builds() {
    let runtime = PluginRuntime::new().expect("Rutis runtime");
    let arbitrary = b"unlinked third party native code";
    let bundle = signed_bundle("1.0.0", arbitrary);
    let error = match NativePlugin::register(&runtime, &bundle, arbitrary) {
        Ok(_) => panic!("unlinked plugin accepted"),
        Err(error) => error,
    };
    assert_eq!(error.kind(), PluginRuntimeErrorKind::DescriptorMismatch);
    let embedded = EmbeddedOfficialAiPlugin::load().expect("native bundle");
    let error = match NativePlugin::register(&runtime, embedded.bundle(), arbitrary) {
        Ok(_) => panic!("tampered identity accepted"),
        Err(error) => error,
    };
    assert_eq!(
        error.kind(),
        PluginRuntimeErrorKind::ComponentDigestMismatch
    );
    runtime.shutdown().await.expect("runtime cleanup");
}
