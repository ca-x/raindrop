use super::{
    BundledOfficialPlugin, OfficialSigningKey, PluginRegistryError, PluginRegistryErrorKind,
    runtime::{CompiledPlugin, PluginRuntime, PluginRuntimeError},
};

const COMPONENT: &[u8] = include_bytes!(concat!(
    env!("OUT_DIR"),
    "/raindrop-ai-content.component.wasm"
));
const MANIFEST: &[u8] = include_bytes!(concat!(
    env!("OUT_DIR"),
    "/raindrop-ai-content.manifest.json"
));
const PUBLIC_KEY: &[u8] = include_bytes!(concat!(
    env!("OUT_DIR"),
    "/raindrop-ai-content.public-key.bin"
));
const SIGNING_KEY_ID: &str = env!("RAINDROP_OFFICIAL_PLUGIN_SIGNING_KEY_ID");
const SIGNATURE_MODE: &str = env!("RAINDROP_OFFICIAL_PLUGIN_SIGNATURE_MODE");

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EmbeddedSignatureMode {
    Development,
    Official,
}

pub struct EmbeddedOfficialAiPlugin {
    bundle: BundledOfficialPlugin,
    signature_mode: EmbeddedSignatureMode,
}

impl EmbeddedOfficialAiPlugin {
    pub fn load() -> Result<Self, PluginRegistryError> {
        let signature_mode = match SIGNATURE_MODE {
            "DEVELOPMENT" if SIGNING_KEY_ID == "raindrop-development-2026" => {
                EmbeddedSignatureMode::Development
            }
            "OFFICIAL" if SIGNING_KEY_ID == "raindrop-release-2026" => {
                EmbeddedSignatureMode::Official
            }
            _ => return Err(invalid_manifest()),
        };
        let public_key: [u8; 32] = PUBLIC_KEY.try_into().map_err(|_| invalid_manifest())?;
        let key =
            OfficialSigningKey::new(SIGNING_KEY_ID, public_key).map_err(|_| invalid_manifest())?;
        let bundle = BundledOfficialPlugin::verify(MANIFEST, COMPONENT, &[key])?;
        if bundle.plugin_key() != "raindrop.ai-content"
            || bundle.version() != "1.0.0"
            || bundle.abi_version() != "raindrop:content-plugin@1.0.0"
            || bundle.signature_key_id() != SIGNING_KEY_ID
        {
            return Err(invalid_manifest());
        }
        Ok(Self {
            bundle,
            signature_mode,
        })
    }

    #[must_use]
    pub const fn bundle(&self) -> &BundledOfficialPlugin {
        &self.bundle
    }

    #[must_use]
    pub const fn component(&self) -> &'static [u8] {
        COMPONENT
    }

    #[must_use]
    pub const fn signature_mode(&self) -> EmbeddedSignatureMode {
        self.signature_mode
    }

    pub fn compile(&self, runtime: &PluginRuntime) -> Result<CompiledPlugin, PluginRuntimeError> {
        CompiledPlugin::compile(runtime, &self.bundle, COMPONENT)
    }
}

fn invalid_manifest() -> PluginRegistryError {
    PluginRegistryError::new(PluginRegistryErrorKind::InvalidManifest)
}
