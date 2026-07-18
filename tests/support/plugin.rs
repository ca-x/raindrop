use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use raindrop::plugins::{BundledOfficialPlugin, OfficialSigningKey};
use ring::{
    digest::{SHA256, digest},
    signature::{Ed25519KeyPair, KeyPair as _},
};
use serde_json::{Value, json};

const TEST_KEY_ID: &str = "raindrop-test-release-2026";
const TEST_SEED: [u8; 32] = [0x24; 32];

pub fn signed_bundle(version: &str, component: &[u8]) -> BundledOfficialPlugin {
    let digest = sha256_hex(component);
    let canonical_without_signature = format!(
        "{{\"abi\":\"raindrop:content-plugin@1.0.0\",\"ambientPermissions\":[],\"artifactSchemas\":[\"raindrop://schemas/artifacts/ai-summary/v1\",\"raindrop://schemas/artifacts/ai-translation/v1\"],\"capabilities\":{{\"optional\":[\"mcp.call_tool\"],\"required\":[\"ai.generate_structured\"]}},\"componentDigest\":{{\"algorithm\":\"SHA-256\",\"value\":\"{digest}\",\"valueEncoding\":\"LOWER_HEX\"}},\"configSchema\":\"raindrop://schemas/plugins/raindrop.ai-content/config/v1\",\"distribution\":\"BUNDLED_OFFICIAL\",\"lifecycleSubscriptions\":[{{\"event\":\"feed.refresh.persisted\",\"schemaVersion\":1}}],\"manifestVersion\":1,\"operations\":[\"summarize\",\"translate\"],\"pluginKey\":\"raindrop.ai-content\",\"signature\":{{\"algorithm\":\"Ed25519\",\"keyId\":\"{TEST_KEY_ID}\",\"valueEncoding\":\"BASE64URL\"}},\"version\":\"{version}\"}}"
    );
    let payload = signature_payload(canonical_without_signature.as_bytes(), &digest);
    let key_pair = Ed25519KeyPair::from_seed_unchecked(&TEST_SEED).unwrap();
    let signature = URL_SAFE_NO_PAD.encode(key_pair.sign(&payload).as_ref());
    let mut manifest: Value = serde_json::from_str(&canonical_without_signature).unwrap();
    manifest["signature"]["value"] = json!(signature);
    let public_key: [u8; 32] = key_pair.public_key().as_ref().try_into().unwrap();
    let key = OfficialSigningKey::new(TEST_KEY_ID, public_key).unwrap();
    BundledOfficialPlugin::verify(&serde_json::to_vec(&manifest).unwrap(), component, &[key])
        .expect("test official bundle should verify")
}

fn signature_payload(canonical_manifest: &[u8], digest: &str) -> Vec<u8> {
    let mut payload = b"raindrop.plugin-signature.v1".to_vec();
    payload.extend_from_slice(&(canonical_manifest.len() as u64).to_be_bytes());
    payload.extend_from_slice(canonical_manifest);
    payload.extend_from_slice(&(digest.len() as u64).to_be_bytes());
    payload.extend_from_slice(digest.as_bytes());
    payload
}

fn sha256_hex(bytes: &[u8]) -> String {
    digest(&SHA256, bytes)
        .as_ref()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
