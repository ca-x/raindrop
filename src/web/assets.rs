use std::borrow::Cow;

use rust_embed::RustEmbed;

#[derive(RustEmbed)]
#[folder = "web/dist/"]
struct EmbeddedAssets;

pub(super) fn get(path: &str) -> Option<Cow<'static, [u8]>> {
    EmbeddedAssets::get(path).map(|asset| asset.data)
}
