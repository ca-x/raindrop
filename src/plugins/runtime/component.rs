use std::fmt::Write as _;

use ring::digest::{SHA256, digest};
use wasmtime::component::Component;

use crate::plugins::BundledOfficialPlugin;

use super::{PluginRuntime, PluginRuntimeError, PluginRuntimeErrorKind};

const MAX_COMPONENT_BYTES: usize = 16 * 1024 * 1024;

#[derive(Clone)]
pub struct CompiledPlugin {
    _component: Component,
    plugin_key: String,
    version: String,
    abi_version: String,
    component_digest: String,
}

impl CompiledPlugin {
    pub fn compile(
        runtime: &PluginRuntime,
        bundle: &BundledOfficialPlugin,
        component_bytes: &[u8],
    ) -> Result<Self, PluginRuntimeError> {
        if component_bytes.is_empty() || component_bytes.len() > MAX_COMPONENT_BYTES {
            return Err(PluginRuntimeError::new(
                PluginRuntimeErrorKind::InvalidComponent,
            ));
        }
        let component_digest = sha256_hex(component_bytes);
        if component_digest != bundle.component_digest() {
            return Err(PluginRuntimeError::new(
                PluginRuntimeErrorKind::ComponentDigestMismatch,
            ));
        }
        let component = Component::from_binary(runtime.engine(), component_bytes)
            .map_err(|_| PluginRuntimeError::new(PluginRuntimeErrorKind::InvalidComponent))?;

        Ok(Self {
            _component: component,
            plugin_key: bundle.plugin_key().to_owned(),
            version: bundle.version().to_owned(),
            abi_version: bundle.abi_version().to_owned(),
            component_digest,
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

    pub(crate) fn component(&self) -> &Component {
        &self._component
    }
}

fn sha256_hex(bytes: &[u8]) -> String {
    let value = digest(&SHA256, bytes);
    let mut encoded = String::with_capacity(64);
    for byte in value.as_ref() {
        write!(&mut encoded, "{byte:02x}").expect("writing to a String cannot fail");
    }
    encoded
}
