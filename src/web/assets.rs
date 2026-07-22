use std::borrow::Cow;

use rust_embed::RustEmbed;

#[derive(RustEmbed)]
#[folder = "web/dist/"]
struct EmbeddedAssets;

pub(super) fn get(path: &str) -> Option<Cow<'static, [u8]>> {
    // The build-script digest makes changes under web/dist a Rust compilation
    // input, so release binaries cannot silently retain an older embedded UI.
    let _bundle_digest = env!("RAINDROP_WEB_BUNDLE_DIGEST");
    EmbeddedAssets::get(path).map(|asset| asset.data)
}
