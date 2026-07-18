use std::{path::PathBuf, sync::OnceLock};

use wit_component::{ComponentEncoder, StringEncoding, dummy_module, embed_component_metadata};
use wit_parser::{ManglingAndAbi, Resolve};

pub fn component_fixture() -> &'static [u8] {
    static COMPONENT: OnceLock<Vec<u8>> = OnceLock::new();
    COMPONENT.get_or_init(build_component).as_slice()
}

fn build_component() -> Vec<u8> {
    let root =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("contracts/wit/raindrop-content-plugin-v1");
    let mut resolve = Resolve::default();
    let (package, _) = resolve
        .push_dir(&root)
        .unwrap_or_else(|error| panic!("plugin WIT should parse at {}: {error:#}", root.display()));
    let world = resolve
        .select_world(&[package], Some("content-plugin-v1"))
        .expect("content-plugin-v1 should resolve");
    let mut module = dummy_module(&resolve, world, ManglingAndAbi::Standard32);
    embed_component_metadata(&mut module, &resolve, world, StringEncoding::UTF8)
        .expect("component metadata should embed");
    let mut encoder = ComponentEncoder::default()
        .module(&module)
        .expect("dummy module should be accepted")
        .validate(true);
    encoder.encode().expect("component fixture should encode")
}
