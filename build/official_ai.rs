use std::{
    env, fs,
    path::{Path, PathBuf},
    process::Command,
};

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use ring::{
    digest::{SHA256, digest},
    signature::{Ed25519KeyPair, KeyPair as _},
};
use serde_json::{Map, Value};
use wit_component::ComponentEncoder;
use zeroize::Zeroize;

const COMPONENT_FILE: &str = "raindrop-ai-content.component.wasm";
const MANIFEST_FILE: &str = "raindrop-ai-content.manifest.json";
const PUBLIC_KEY_FILE: &str = "raindrop-ai-content.public-key.bin";
const GUEST_MANIFEST: &str = "plugins/official/raindrop-ai-content/Cargo.toml";
const MANIFEST_TEMPLATE: &str = "contracts/plugins/raindrop.ai-content/manifest.template.json";
const SIGNATURE_CONTEXT: &[u8] = b"raindrop.plugin-signature.v1";
const DEVELOPMENT_SEED_CONTEXT: &[u8] =
    b"raindrop development signing seed v1; public and not trusted for releases";
const DEVELOPMENT_KEY_ID: &str = "raindrop-development-2026";
const OFFICIAL_KEY_ID: &str = "raindrop-release-2026";
const SIGNING_SEED_ENV: &str = "RAINDROP_OFFICIAL_PLUGIN_SIGNING_SEED";
const SIGNING_KEY_ID_ENV: &str = "RAINDROP_OFFICIAL_PLUGIN_SIGNING_KEY_ID";
const REQUIRE_OFFICIAL_ENV: &str = "RAINDROP_REQUIRE_OFFICIAL_PLUGIN_SIGNATURE";

pub fn build() -> Result<(), String> {
    emit_inputs();
    let root = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").ok_or("workspace root is missing")?);
    let out_dir = PathBuf::from(env::var_os("OUT_DIR").ok_or("Cargo OUT_DIR is missing")?);
    let component = build_component(&root, &out_dir)?;
    let digest = sha256_hex(&component);
    let signing = signing_material()?;
    let (manifest, public_key) = finalize_manifest(&root, &component, &digest, signing)?;

    fs::write(out_dir.join(COMPONENT_FILE), component)
        .map_err(|_| "embedded component output could not be written".to_owned())?;
    fs::write(out_dir.join(MANIFEST_FILE), manifest)
        .map_err(|_| "embedded manifest output could not be written".to_owned())?;
    fs::write(out_dir.join(PUBLIC_KEY_FILE), public_key)
        .map_err(|_| "embedded public key output could not be written".to_owned())?;
    Ok(())
}

fn emit_inputs() {
    for path in [
        GUEST_MANIFEST,
        "plugins/official/raindrop-ai-content/src",
        "contracts/wit/raindrop-content-plugin-v1",
        MANIFEST_TEMPLATE,
        "contracts/plugins/raindrop.ai-content/config.v1.schema.json",
        "contracts/artifacts/ai-summary.v1.schema.json",
        "contracts/artifacts/ai-translation.v1.schema.json",
    ] {
        println!("cargo:rerun-if-changed={path}");
    }
    for name in [SIGNING_SEED_ENV, SIGNING_KEY_ID_ENV, REQUIRE_OFFICIAL_ENV] {
        println!("cargo:rerun-if-env-changed={name}");
    }
}

fn build_component(root: &Path, out_dir: &Path) -> Result<Vec<u8>, String> {
    let target_dir = out_dir.join("official-ai-target");
    let cargo = env::var_os("CARGO").ok_or("Cargo executable is missing")?;
    let output = Command::new(cargo)
        .args(["build", "--locked", "--manifest-path"])
        .arg(root.join(GUEST_MANIFEST))
        .args(["--target", "wasm32-unknown-unknown", "--release"])
        .current_dir(root)
        .env("CARGO_TARGET_DIR", &target_dir)
        .env_remove("CARGO_BUILD_TARGET")
        .env_remove("CARGO_ENCODED_RUSTFLAGS")
        .env_remove("RUSTFLAGS")
        .env_remove("RUSTC_WRAPPER")
        .env_remove("RUSTC_WORKSPACE_WRAPPER")
        .env_remove(SIGNING_SEED_ENV)
        .env_remove(SIGNING_KEY_ID_ENV)
        .env_remove(REQUIRE_OFFICIAL_ENV)
        .output()
        .map_err(|_| "official guest build could not start".to_owned())?;
    if !output.status.success() {
        return Err(format!(
            "official guest build failed\n{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        ));
    }

    let core_path = target_dir.join("wasm32-unknown-unknown/release/raindrop_ai_content.wasm");
    let core = fs::read(core_path).map_err(|_| "official guest output is missing".to_owned())?;
    let first = componentize(&core)?;
    let second = componentize(&core)?;
    if first != second {
        return Err("official componentization is not deterministic".to_owned());
    }
    let text = wasmprinter::print_bytes(&first)
        .map_err(|_| "official component could not be inspected".to_owned())?;
    if text.contains("wasi:")
        || !text.contains("raindrop:content-plugin/host-ai@1.0.0")
        || !text.contains("raindrop:content-plugin/host-mcp@1.0.0")
        || !text.contains("raindrop:content-plugin/content-plugin@1.0.0")
    {
        return Err("official component imports or exports are invalid".to_owned());
    }
    Ok(first)
}

fn componentize(core: &[u8]) -> Result<Vec<u8>, String> {
    ComponentEncoder::default()
        .module(core)
        .map_err(|_| "official guest metadata is invalid".to_owned())?
        .validate(true)
        .encode()
        .map_err(|_| "official guest componentization failed".to_owned())
}

struct SigningMaterial {
    key_id: &'static str,
    mode: &'static str,
    seed: [u8; 32],
}

impl Drop for SigningMaterial {
    fn drop(&mut self) {
        self.seed.zeroize();
    }
}

fn signing_material() -> Result<SigningMaterial, String> {
    let require_official = match env::var_os(REQUIRE_OFFICIAL_ENV) {
        None => false,
        Some(value) if value == "1" => true,
        Some(_) => return Err("official signature requirement is invalid".to_owned()),
    };
    match env::var_os(SIGNING_SEED_ENV) {
        Some(raw) => {
            let mut raw = raw
                .into_string()
                .map_err(|_| "official signing seed is invalid".to_owned())?;
            let mut decoded = match URL_SAFE_NO_PAD.decode(raw.as_bytes()) {
                Ok(decoded) => decoded,
                Err(_) => {
                    raw.zeroize();
                    return Err("official signing seed is invalid".to_owned());
                }
            };
            let mut canonical = URL_SAFE_NO_PAD.encode(&decoded);
            let is_valid = decoded.len() == 32 && canonical.as_bytes() == raw.as_bytes();
            canonical.zeroize();
            raw.zeroize();
            if !is_valid {
                decoded.zeroize();
                return Err("official signing seed is invalid".to_owned());
            }
            let key_id = env::var(SIGNING_KEY_ID_ENV)
                .map_err(|_| "official signing key ID is missing".to_owned())?;
            if key_id != OFFICIAL_KEY_ID {
                decoded.zeroize();
                return Err("official signing key ID is invalid".to_owned());
            }
            let mut seed = [0_u8; 32];
            seed.copy_from_slice(&decoded);
            decoded.zeroize();
            Ok(SigningMaterial {
                key_id: OFFICIAL_KEY_ID,
                mode: "OFFICIAL",
                seed,
            })
        }
        None if require_official => Err("official signing seed is required".to_owned()),
        None => {
            if env::var_os(SIGNING_KEY_ID_ENV).is_some() {
                return Err("official signing key ID requires a signing seed".to_owned());
            }
            let value = digest(&SHA256, DEVELOPMENT_SEED_CONTEXT);
            let mut seed = [0_u8; 32];
            seed.copy_from_slice(value.as_ref());
            Ok(SigningMaterial {
                key_id: DEVELOPMENT_KEY_ID,
                mode: "DEVELOPMENT",
                seed,
            })
        }
    }
}

fn finalize_manifest(
    root: &Path,
    component: &[u8],
    component_digest: &str,
    mut signing: SigningMaterial,
) -> Result<(Vec<u8>, [u8; 32]), String> {
    if component.is_empty() {
        signing.seed.zeroize();
        return Err("official component is empty".to_owned());
    }
    let template = fs::read(root.join(MANIFEST_TEMPLATE))
        .map_err(|_| "official manifest template is missing".to_owned())?;
    let mut manifest: Value = serde_json::from_slice(&template)
        .map_err(|_| "official manifest template is invalid".to_owned())?;
    let document = manifest
        .as_object_mut()
        .ok_or("official manifest template is invalid")?;
    let digest_field = document
        .get_mut("componentDigest")
        .and_then(Value::as_object_mut)
        .ok_or("official manifest digest field is invalid")?;
    if digest_field.remove("valueSource") != Some(Value::String("RELEASE_BUILD".to_owned())) {
        signing.seed.zeroize();
        return Err("official manifest digest source is invalid".to_owned());
    }
    digest_field.insert(
        "value".to_owned(),
        Value::String(component_digest.to_owned()),
    );

    let signature_field = document
        .get_mut("signature")
        .and_then(Value::as_object_mut)
        .ok_or("official manifest signature field is invalid")?;
    if signature_field.remove("valueSource") != Some(Value::String("RELEASE_BUILD".to_owned())) {
        signing.seed.zeroize();
        return Err("official manifest signature source is invalid".to_owned());
    }
    signature_field.insert("keyId".to_owned(), Value::String(signing.key_id.to_owned()));
    signature_field.remove("value");

    let unsigned = canonical_json(manifest.clone())?;
    let payload = signature_payload(&unsigned, component_digest);
    let key_pair = Ed25519KeyPair::from_seed_unchecked(&signing.seed)
        .map_err(|_| "official signing key could not be created".to_owned())?;
    signing.seed.zeroize();
    let signature = URL_SAFE_NO_PAD.encode(key_pair.sign(&payload).as_ref());
    let public_key: [u8; 32] = key_pair
        .public_key()
        .as_ref()
        .try_into()
        .map_err(|_| "official signing public key is invalid".to_owned())?;
    manifest
        .get_mut("signature")
        .and_then(Value::as_object_mut)
        .ok_or("official manifest signature field is invalid")?
        .insert("value".to_owned(), Value::String(signature));
    let canonical = canonical_json(manifest)?;
    println!(
        "cargo:rustc-env=RAINDROP_OFFICIAL_PLUGIN_SIGNING_KEY_ID={}",
        signing.key_id
    );
    println!(
        "cargo:rustc-env=RAINDROP_OFFICIAL_PLUGIN_SIGNATURE_MODE={}",
        signing.mode
    );
    Ok((canonical, public_key))
}

fn canonical_json(value: Value) -> Result<Vec<u8>, String> {
    serde_json::to_vec(&sort_value(value))
        .map_err(|_| "official manifest canonicalization failed".to_owned())
}

fn sort_value(value: Value) -> Value {
    match value {
        Value::Object(object) => {
            let mut entries = object.into_iter().collect::<Vec<_>>();
            entries.sort_by(|left, right| left.0.cmp(&right.0));
            let mut sorted = Map::new();
            for (key, value) in entries {
                sorted.insert(key, sort_value(value));
            }
            Value::Object(sorted)
        }
        Value::Array(values) => Value::Array(values.into_iter().map(sort_value).collect()),
        value => value,
    }
}

fn signature_payload(canonical_manifest: &[u8], component_digest: &str) -> Vec<u8> {
    let mut payload = Vec::with_capacity(
        SIGNATURE_CONTEXT.len() + canonical_manifest.len() + component_digest.len() + 16,
    );
    payload.extend_from_slice(SIGNATURE_CONTEXT);
    payload.extend_from_slice(&(canonical_manifest.len() as u64).to_be_bytes());
    payload.extend_from_slice(canonical_manifest);
    payload.extend_from_slice(&(component_digest.len() as u64).to_be_bytes());
    payload.extend_from_slice(component_digest.as_bytes());
    payload
}

fn sha256_hex(bytes: &[u8]) -> String {
    digest(&SHA256, bytes)
        .as_ref()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
