use raindrop::plugins::{
    EmbeddedOfficialAiPlugin,
    runtime::{NativePlugin, PluginRuntime},
};

pub fn official_ai_native() -> &'static [u8] {
    EmbeddedOfficialAiPlugin::identity()
}

pub fn registered_official_ai_plugin(runtime: &PluginRuntime) -> NativePlugin {
    EmbeddedOfficialAiPlugin::load()
        .expect("bundled native metadata")
        .register(runtime)
        .expect("native plugin registration")
}
