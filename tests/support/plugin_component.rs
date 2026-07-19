use std::{
    fs,
    path::{Path, PathBuf},
    sync::OnceLock,
};

use wit_component::{ComponentEncoder, StringEncoding, dummy_module, embed_component_metadata};
use wit_parser::{ManglingAndAbi, Resolve};

const IMAGE_BASE: u32 = 1024;
const HEAP_BASE: u32 = 16 * 1024;

#[derive(Clone, Copy)]
pub enum ComponentBehavior {
    Success,
    DescriptorMismatch,
    DescriptorTrap,
    ExecuteTrap,
    FuelSplit,
    KnownPluginError,
    UnknownPluginError,
    InvalidArtifact,
    MemoryLimit,
    OutputTooLarge,
}

pub fn component_fixture() -> &'static [u8] {
    static COMPONENT: OnceLock<Vec<u8>> = OnceLock::new();
    COMPONENT
        .get_or_init(|| build_component(ComponentBehavior::Success))
        .as_slice()
}

pub fn component_with_behavior(behavior: ComponentBehavior) -> Vec<u8> {
    build_component(behavior)
}

pub fn component_with_unexpected_import() -> Vec<u8> {
    let source =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("contracts/wit/raindrop-content-plugin-v1");
    let temp = tempfile::tempdir().expect("temporary WIT directory");
    for entry in fs::read_dir(&source).expect("committed WIT directory") {
        let entry = entry.expect("WIT directory entry");
        if entry
            .path()
            .extension()
            .is_some_and(|extension| extension == "wit")
        {
            fs::copy(entry.path(), temp.path().join(entry.file_name())).expect("copy WIT file");
        }
    }
    fs::write(
        temp.path().join("ambient.wit"),
        "package raindrop:content-plugin@1.0.0;\ninterface ambient { open: func(); }\n",
    )
    .expect("write hostile interface");
    let world_path = temp.path().join("world.wit");
    let world = fs::read_to_string(&world_path)
        .expect("read world")
        .replacen(
            "  import host-mcp;",
            "  import host-mcp;\n  import ambient;",
            1,
        );
    fs::write(&world_path, world).expect("write hostile world");

    let (resolve, world) = resolve_world_at(temp.path());
    let module = dummy_module(&resolve, world, ManglingAndAbi::Standard32);
    let wat = wasmprinter::print_bytes(module).expect("hostile dummy module should print");
    let module = wat::parse_str(core_wat(&wat, ComponentBehavior::Success))
        .expect("hostile core WAT should parse");
    encode_component(module, &resolve, world)
}

fn build_component(behavior: ComponentBehavior) -> Vec<u8> {
    let (resolve, world) = resolve_world();
    let module = dummy_module(&resolve, world, ManglingAndAbi::Standard32);
    let wat = wasmprinter::print_bytes(module).expect("dummy module should print");
    let module = wat::parse_str(core_wat(&wat, behavior)).expect("hostile core WAT should parse");
    encode_component(module, &resolve, world)
}

fn resolve_world() -> (Resolve, wit_parser::WorldId) {
    let root =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("contracts/wit/raindrop-content-plugin-v1");
    resolve_world_at(&root)
}

fn resolve_world_at(root: &Path) -> (Resolve, wit_parser::WorldId) {
    let mut resolve = Resolve::default();
    let (package, _) = resolve
        .push_dir(root)
        .unwrap_or_else(|error| panic!("plugin WIT should parse at {}: {error:#}", root.display()));
    let world = resolve
        .select_world(&[package], Some("content-plugin-v1"))
        .expect("content-plugin-v1 should resolve");
    (resolve, world)
}

fn encode_component(mut module: Vec<u8>, resolve: &Resolve, world: wit_parser::WorldId) -> Vec<u8> {
    embed_component_metadata(&mut module, resolve, world, StringEncoding::UTF8)
        .expect("component metadata should embed");
    let mut encoder = ComponentEncoder::default()
        .module(&module)
        .expect("dummy module should be accepted")
        .validate(true);
    encoder.encode().expect("component fixture should encode")
}

fn core_wat(source: &str, behavior: ComponentBehavior) -> String {
    let mut image = MemoryImage::new(IMAGE_BASE);
    let descriptor_version = if matches!(behavior, ComponentBehavior::DescriptorMismatch) {
        "9.9.9"
    } else {
        "1.0.0"
    };
    let descriptor = write_descriptor(&mut image, descriptor_version);
    let artifact = write_artifact(&mut image, behavior);
    let event_outcome = write_event_outcome(&mut image);
    let image_end = usize::try_from(IMAGE_BASE).expect("image base") + image.bytes.len();
    let heap_base = usize::try_from(HEAP_BASE)
        .expect("heap base")
        .max(image_end.div_ceil(8) * 8);
    let memory_pages = match behavior {
        ComponentBehavior::MemoryLimit => 1025,
        ComponentBehavior::OutputTooLarge => {
            u32::try_from((heap_base + 2 * 64 * 1024).div_ceil(64 * 1024))
                .expect("fixture pages")
                .max(2)
        }
        _ => 2,
    };
    let memory = format!(
        "(memory (;0;) {memory_pages})\n  (global $heap (mut i32) (i32.const {heap_base}))\n  (data (i32.const {IMAGE_BASE}) \"{}\")",
        escape_wat_bytes(&image.bytes)
    );
    let mut wat = replace_once(source, "(memory (;0;) 0)", &memory);

    if !matches!(behavior, ComponentBehavior::DescriptorTrap) {
        wat = replace_exported_function_body(
            &wat,
            "cm32p2|raindrop:content-plugin/content-plugin@1|descriptor",
            &format!("i32.const {descriptor}"),
        );
    }
    let execute_body = match behavior {
        ComponentBehavior::ExecuteTrap => "unreachable".to_owned(),
        ComponentBehavior::FuelSplit => format!("call $burn\n    i32.const {artifact}"),
        _ => format!("i32.const {artifact}"),
    };
    wat = replace_exported_function_body(
        &wat,
        "cm32p2|raindrop:content-plugin/content-plugin@1|execute",
        &execute_body,
    );
    let on_event_body = if matches!(behavior, ComponentBehavior::FuelSplit) {
        format!("call $burn\n    i32.const {event_outcome}")
    } else {
        format!("i32.const {event_outcome}")
    };
    wat = replace_exported_function_body(
        &wat,
        "cm32p2|raindrop:content-plugin/content-plugin@1|on-event",
        &on_event_body,
    );
    wat = replace_exported_function_body(
        &wat,
        "cm32p2_realloc",
        "(local $ptr i32)\n    global.get $heap\n    local.get 2\n    i32.const 1\n    i32.sub\n    i32.add\n    local.get 2\n    i32.const 1\n    i32.sub\n    i32.const -1\n    i32.xor\n    i32.and\n    local.tee $ptr\n    local.get 3\n    i32.add\n    global.set $heap\n    local.get $ptr",
    );
    if matches!(behavior, ComponentBehavior::FuelSplit) {
        let helper = "\n  (func $burn\n    (local $remaining i32)\n    i32.const 1100000\n    local.set $remaining\n    (loop $burn-loop\n      local.get $remaining\n      i32.const 1\n      i32.sub\n      local.tee $remaining\n      br_if $burn-loop\n    )\n  )\n";
        let end = wat
            .rfind(')')
            .expect("module should have a final delimiter");
        wat.insert_str(end, helper);
    }
    wat
}

fn write_descriptor(image: &mut MemoryImage, version: &str) -> u32 {
    let descriptor = image.reserve(56, 4);
    let plugin_key = image.string("raindrop.ai-content");
    let version = image.string(version);
    let abi = image.string("raindrop:content-plugin@1.0.0");
    let operations = image.bytes(&[0, 1], 1);
    let event = image.string("feed.refresh.persisted");
    let lifecycle = image.reserve(12, 4);
    image.write_pair(lifecycle, event);
    image.write_u32(lifecycle + 8, 1);
    let required_capability = image.string("ai.generate_structured");
    let required = image.reserve(8, 4);
    image.write_pair(required, required_capability);
    let optional_capability = image.string("mcp.call_tool");
    let optional = image.reserve(8, 4);
    image.write_pair(optional, optional_capability);

    image.write_pair(descriptor, plugin_key);
    image.write_pair(descriptor + 8, version);
    image.write_pair(descriptor + 16, abi);
    image.write_pair(descriptor + 24, (operations, 2));
    image.write_pair(descriptor + 32, (lifecycle, 1));
    image.write_pair(descriptor + 40, (required, 1));
    image.write_pair(descriptor + 48, (optional, 1));
    descriptor
}

fn write_artifact(image: &mut MemoryImage, behavior: ComponentBehavior) -> u32 {
    if matches!(
        behavior,
        ComponentBehavior::KnownPluginError | ComponentBehavior::UnknownPluginError
    ) {
        return write_plugin_error(image, behavior);
    }
    let result = image.reserve(40, 4);
    let artifact = result + 4;
    let schema = image.string("raindrop://schemas/artifacts/ai-summary/v1");
    let payload = match behavior {
        ComponentBehavior::OutputTooLarge => image.string(&"x".repeat(512 * 1024)),
        ComponentBehavior::InvalidArtifact => image.string(r#"{"schemaVersion":1}"#),
        _ => image.string(
            r#"{"bullets":[],"conclusion":null,"schemaVersion":1,"sourceLanguage":"en","summary":"Fixture summary."}"#,
        ),
    };
    let provenance = image.string(r#"{"fixture":true}"#);
    image.write_pair(artifact, schema);
    image.write_u32(artifact + 8, 0);
    image.write_pair(artifact + 20, payload);
    image.write_pair(artifact + 28, provenance);
    result
}

fn write_plugin_error(image: &mut MemoryImage, behavior: ComponentBehavior) -> u32 {
    let result = image.reserve(40, 4);
    let message_key = match behavior {
        ComponentBehavior::KnownPluginError => "raindrop.ai-content.config-invalid",
        ComponentBehavior::UnknownPluginError => "rd-secret-attacker-message-key",
        _ => unreachable!("plugin error helper requires an error behavior"),
    };
    let message_key = image.string(message_key);
    image.write_u8(result, 1);
    image.write_u8(result + 4, 3);
    image.write_u8(result + 5, 0);
    image.write_pair(result + 8, message_key);
    result
}

fn write_event_outcome(image: &mut MemoryImage) -> u32 {
    image.reserve(20, 4)
}

fn replace_once(source: &str, needle: &str, replacement: &str) -> String {
    assert_eq!(source.matches(needle).count(), 1, "fixture shape drifted");
    source.replacen(needle, replacement, 1)
}

fn replace_exported_function_body(source: &str, export_name: &str, body: &str) -> String {
    let export = format!("(export \"{export_name}\" (func ");
    let export_start = source.find(&export).expect("fixture export should exist") + export.len();
    let export_end = source[export_start..]
        .find(')')
        .map(|offset| export_start + offset)
        .expect("fixture export index should end");
    let index = &source[export_start..export_end];
    let marker = format!("(func (;{index};)");
    let function_start = source.find(&marker).expect("fixture function should exist");
    let header_end = source[function_start..]
        .find('\n')
        .map(|offset| function_start + offset)
        .expect("fixture function header should end");
    let function_end = source[header_end..]
        .find("\n  )")
        .map(|offset| header_end + offset + "\n  )".len())
        .expect("fixture function should end");
    let header = &source[function_start..header_end];
    let replacement = format!("{header}\n    {body}\n  )");
    let mut updated = String::with_capacity(source.len() + replacement.len());
    updated.push_str(&source[..function_start]);
    updated.push_str(&replacement);
    updated.push_str(&source[function_end..]);
    updated
}

fn escape_wat_bytes(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("\\{byte:02x}")).collect()
}

struct MemoryImage {
    base: u32,
    bytes: Vec<u8>,
}

impl MemoryImage {
    fn new(base: u32) -> Self {
        Self {
            base,
            bytes: Vec::new(),
        }
    }

    fn reserve(&mut self, size: usize, align: usize) -> u32 {
        let padding = (align - self.bytes.len() % align) % align;
        self.bytes.resize(self.bytes.len() + padding, 0);
        let address = self.base + u32::try_from(self.bytes.len()).expect("fixture image fits u32");
        self.bytes.resize(self.bytes.len() + size, 0);
        address
    }

    fn bytes(&mut self, value: &[u8], align: usize) -> u32 {
        let address = self.reserve(value.len(), align);
        let offset = self.offset(address);
        self.bytes[offset..offset + value.len()].copy_from_slice(value);
        address
    }

    fn string(&mut self, value: &str) -> (u32, u32) {
        (
            self.bytes(value.as_bytes(), 1),
            u32::try_from(value.len()).expect("fixture string fits u32"),
        )
    }

    fn write_pair(&mut self, address: u32, pair: (u32, u32)) {
        self.write_u32(address, pair.0);
        self.write_u32(address + 4, pair.1);
    }

    fn write_u32(&mut self, address: u32, value: u32) {
        let offset = self.offset(address);
        self.bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }

    fn write_u8(&mut self, address: u32, value: u8) {
        let offset = self.offset(address);
        self.bytes[offset] = value;
    }

    fn offset(&self, address: u32) -> usize {
        usize::try_from(address - self.base).expect("fixture address fits usize")
    }
}
