use std::fmt;

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use ring::{
    digest::{SHA256, digest},
    signature::{ED25519, UnparsedPublicKey},
};
use serde::Deserialize;
use serde_json::Value;

use super::{
    PluginRegistryError, PluginRegistryErrorKind,
    json::{canonical_json, parse_unique_json, validate_lower_hex_hash, validate_visible_ascii},
};

const MAX_MANIFEST_BYTES: usize = 64 * 1024;
const SIGNATURE_CONTEXT: &[u8] = b"raindrop.plugin-signature.v1";
const OFFICIAL_PLUGIN_KEY: &str = "raindrop.ai-content";
const OFFICIAL_ABI: &str = "raindrop:content-plugin@1.0.0";
const OFFICIAL_DISTRIBUTION: &str = "BUNDLED_OFFICIAL";
const CONFIG_SCHEMA: &str = "raindrop://schemas/plugins/raindrop.ai-content/config/v1";
const SUMMARY_SCHEMA: &str = "raindrop://schemas/artifacts/ai-summary/v1";
const TRANSLATION_SCHEMA: &str = "raindrop://schemas/artifacts/ai-translation/v1";

#[derive(Clone, Eq, PartialEq)]
pub struct OfficialSigningKey {
    key_id: String,
    public_key: [u8; 32],
}

impl OfficialSigningKey {
    pub fn new(
        key_id: impl Into<String>,
        public_key: [u8; 32],
    ) -> Result<Self, PluginRegistryError> {
        let key_id = key_id.into();
        validate_visible_ascii(&key_id, 128, PluginRegistryErrorKind::InvalidInput)?;
        Ok(Self { key_id, public_key })
    }

    #[must_use]
    pub fn key_id(&self) -> &str {
        &self.key_id
    }
}

impl fmt::Debug for OfficialSigningKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("OfficialSigningKey")
            .field("key_id", &self.key_id)
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Eq, PartialEq)]
pub struct BundledOfficialPlugin {
    plugin_key: String,
    version: String,
    abi_version: String,
    component_digest: String,
    manifest_json: String,
    signature_key_id: String,
    signature: String,
}

impl BundledOfficialPlugin {
    pub fn verify(
        manifest_json: &[u8],
        component: &[u8],
        keys: &[OfficialSigningKey],
    ) -> Result<Self, PluginRegistryError> {
        if component.is_empty() {
            return Err(PluginRegistryError::new(
                PluginRegistryErrorKind::InvalidManifest,
            ));
        }
        let mut manifest_value = parse_unique_json(manifest_json, MAX_MANIFEST_BYTES)?;
        let manifest = serde_json::from_value::<ManifestDocument>(manifest_value.clone())
            .map_err(|_| PluginRegistryError::new(PluginRegistryErrorKind::InvalidManifest))?;
        manifest.validate()?;

        let component_digest = sha256_hex(component);
        if manifest.component_digest.value.as_deref() != Some(&component_digest) {
            return Err(PluginRegistryError::new(
                PluginRegistryErrorKind::ComponentDigestMismatch,
            ));
        }

        let signature_value =
            manifest.signature.value.as_deref().ok_or_else(|| {
                PluginRegistryError::new(PluginRegistryErrorKind::InvalidManifest)
            })?;
        let signature = decode_canonical_signature(signature_value)?;
        let key = unique_signing_key(keys, &manifest.signature.key_id)?;

        manifest_value
            .get_mut("signature")
            .and_then(Value::as_object_mut)
            .and_then(|value| value.remove("value"))
            .ok_or_else(|| PluginRegistryError::new(PluginRegistryErrorKind::InvalidManifest))?;
        let canonical_unsigned = canonical_json(manifest_value, MAX_MANIFEST_BYTES)?;
        let payload = signature_payload(canonical_unsigned.as_bytes(), &component_digest);
        UnparsedPublicKey::new(&ED25519, key.public_key)
            .verify(&payload, &signature)
            .map_err(|_| PluginRegistryError::new(PluginRegistryErrorKind::InvalidSignature))?;

        let full_value = parse_unique_json(manifest_json, MAX_MANIFEST_BYTES)?;
        let canonical_manifest = canonical_json(full_value, MAX_MANIFEST_BYTES)?;
        Ok(Self {
            plugin_key: manifest.plugin_key,
            version: manifest.version,
            abi_version: manifest.abi,
            component_digest,
            manifest_json: canonical_manifest,
            signature_key_id: manifest.signature.key_id,
            signature: signature_value.to_owned(),
        })
    }

    #[must_use]
    pub fn plugin_key(&self) -> &str {
        &self.plugin_key
    }

    #[must_use]
    pub fn version(&self) -> &str {
        &self.version
    }

    #[must_use]
    pub fn abi_version(&self) -> &str {
        &self.abi_version
    }

    #[must_use]
    pub fn component_digest(&self) -> &str {
        &self.component_digest
    }

    #[must_use]
    pub fn manifest_json(&self) -> &str {
        &self.manifest_json
    }

    #[must_use]
    pub fn signature_key_id(&self) -> &str {
        &self.signature_key_id
    }

    #[must_use]
    pub fn signature(&self) -> &str {
        &self.signature
    }
}

impl fmt::Debug for BundledOfficialPlugin {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("BundledOfficialPlugin")
            .field("plugin_key", &self.plugin_key)
            .field("version", &self.version)
            .field("abi_version", &self.abi_version)
            .field("component_digest", &self.component_digest)
            .field("signature_key_id", &self.signature_key_id)
            .finish_non_exhaustive()
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ManifestDocument {
    manifest_version: u32,
    plugin_key: String,
    version: String,
    abi: String,
    distribution: String,
    operations: Vec<String>,
    lifecycle_subscriptions: Vec<LifecycleSubscription>,
    capabilities: Capabilities,
    ambient_permissions: Vec<Value>,
    config_schema: String,
    artifact_schemas: Vec<String>,
    component_digest: DigestField,
    signature: SignatureField,
}

impl ManifestDocument {
    fn validate(&self) -> Result<(), PluginRegistryError> {
        let valid = self.manifest_version == 1
            && self.plugin_key == OFFICIAL_PLUGIN_KEY
            && valid_plugin_version(&self.version)
            && self.abi == OFFICIAL_ABI
            && self.distribution == OFFICIAL_DISTRIBUTION
            && self.operations == ["summarize", "translate"]
            && self.lifecycle_subscriptions.len() == 1
            && self.lifecycle_subscriptions[0].event == "feed.refresh.persisted"
            && self.lifecycle_subscriptions[0].schema_version == 1
            && self.capabilities.required == ["ai.generate_structured"]
            && self.capabilities.optional == ["mcp.call_tool"]
            && self.ambient_permissions.is_empty()
            && self.config_schema == CONFIG_SCHEMA
            && self.artifact_schemas == [SUMMARY_SCHEMA, TRANSLATION_SCHEMA]
            && self.component_digest.algorithm == "SHA-256"
            && self.component_digest.value_encoding == "LOWER_HEX"
            && self.component_digest.value_source.is_none()
            && self.signature.algorithm == "Ed25519"
            && self.signature.value_encoding == "BASE64URL"
            && self.signature.value_source.is_none();
        if !valid {
            return Err(PluginRegistryError::new(
                PluginRegistryErrorKind::InvalidManifest,
            ));
        }
        let digest =
            self.component_digest.value.as_deref().ok_or_else(|| {
                PluginRegistryError::new(PluginRegistryErrorKind::InvalidManifest)
            })?;
        validate_lower_hex_hash(digest, PluginRegistryErrorKind::InvalidManifest)?;
        validate_visible_ascii(
            &self.signature.key_id,
            128,
            PluginRegistryErrorKind::InvalidManifest,
        )?;
        let value =
            self.signature.value.as_deref().ok_or_else(|| {
                PluginRegistryError::new(PluginRegistryErrorKind::InvalidManifest)
            })?;
        validate_visible_ascii(value, 128, PluginRegistryErrorKind::InvalidManifest)
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct LifecycleSubscription {
    event: String,
    schema_version: u32,
}

#[derive(Deserialize)]
struct Capabilities {
    required: Vec<String>,
    optional: Vec<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct DigestField {
    algorithm: String,
    value_encoding: String,
    value: Option<String>,
    value_source: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SignatureField {
    algorithm: String,
    value_encoding: String,
    key_id: String,
    value: Option<String>,
    value_source: Option<String>,
}

fn unique_signing_key<'a>(
    keys: &'a [OfficialSigningKey],
    key_id: &str,
) -> Result<&'a OfficialSigningKey, PluginRegistryError> {
    let mut matches = keys.iter().filter(|key| key.key_id == key_id);
    let key = matches
        .next()
        .ok_or_else(|| PluginRegistryError::new(PluginRegistryErrorKind::UnknownSigningKey))?;
    if matches.next().is_some() {
        return Err(PluginRegistryError::new(
            PluginRegistryErrorKind::InvalidInput,
        ));
    }
    Ok(key)
}

fn decode_canonical_signature(value: &str) -> Result<Vec<u8>, PluginRegistryError> {
    let decoded = URL_SAFE_NO_PAD
        .decode(value)
        .map_err(|_| PluginRegistryError::new(PluginRegistryErrorKind::InvalidSignature))?;
    if decoded.len() == 64 && URL_SAFE_NO_PAD.encode(&decoded) == value {
        Ok(decoded)
    } else {
        Err(PluginRegistryError::new(
            PluginRegistryErrorKind::InvalidSignature,
        ))
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

fn sha256_hex(component: &[u8]) -> String {
    let digest = digest(&SHA256, component);
    let mut encoded = String::with_capacity(64);
    for byte in digest.as_ref() {
        use fmt::Write as _;
        write!(&mut encoded, "{byte:02x}").expect("writing to a String cannot fail");
    }
    encoded
}

pub(crate) fn validate_persisted_manifest(
    manifest_json: &str,
    plugin_key: &str,
    version: &str,
    abi_version: &str,
    component_digest: &str,
    signature_key_id: &str,
    signature: &str,
) -> Result<(), PluginRegistryError> {
    let value = parse_unique_json(manifest_json.as_bytes(), MAX_MANIFEST_BYTES)
        .map_err(|_| PluginRegistryError::new(PluginRegistryErrorKind::CorruptData))?;
    let manifest = serde_json::from_value::<ManifestDocument>(value.clone())
        .map_err(|_| PluginRegistryError::new(PluginRegistryErrorKind::CorruptData))?;
    manifest
        .validate()
        .map_err(|_| PluginRegistryError::new(PluginRegistryErrorKind::CorruptData))?;
    let canonical = canonical_json(value, MAX_MANIFEST_BYTES)
        .map_err(|_| PluginRegistryError::new(PluginRegistryErrorKind::CorruptData))?;
    if canonical != manifest_json
        || manifest.plugin_key != plugin_key
        || manifest.version != version
        || manifest.abi != abi_version
        || manifest.component_digest.value.as_deref() != Some(component_digest)
        || manifest.signature.key_id != signature_key_id
        || manifest.signature.value.as_deref() != Some(signature)
    {
        return Err(PluginRegistryError::new(
            PluginRegistryErrorKind::CorruptData,
        ));
    }
    Ok(())
}

fn valid_plugin_version(value: &str) -> bool {
    let parts = value.split('.').collect::<Vec<_>>();
    parts.len() == 3
        && parts[0] == "1"
        && parts[1..].iter().all(|part| {
            !part.is_empty()
                && part.bytes().all(|byte| byte.is_ascii_digit())
                && (part == &"0" || !part.starts_with('0'))
                && part.parse::<u32>().is_ok()
        })
}
