use blake3::Hasher;
use serde_json::{Map, Value};

use super::model::{
    ArtifactIdentity, ArtifactIdentityInput, ContentJobTrigger, ContentRepositoryError,
    ContentRepositoryErrorKind,
};

const IDEMPOTENCY_CONTEXT: &str = "raindrop.content-job.idempotency.v1";
const REQUEST_CONTEXT: &str = "raindrop.content-job.request.v1";
const ARTIFACT_CONTEXT: &str = "raindrop.content-artifact.identity.v1";

pub(super) fn idempotency_key(value: &str) -> String {
    framed_hash(IDEMPOTENCY_CONTEXT, [value.as_bytes()])
}

pub(super) fn artifact_identity(input: &ArtifactIdentityInput, locale: Option<&str>) -> String {
    let revision = input.provider_revision.to_be_bytes();
    framed_hash(
        ARTIFACT_CONTEXT,
        [
            input.user_id.as_bytes(),
            input.entry_id.as_bytes(),
            input.kind.as_storage().as_bytes(),
            locale.unwrap_or_default().as_bytes(),
            input.entry_content_hash.as_bytes(),
            input.input_hash.as_bytes(),
            input.config_hash.as_bytes(),
            input.plugin_key.as_bytes(),
            input.plugin_version.as_bytes(),
            input.component_digest.as_bytes(),
            input.provider_binding_id.as_bytes(),
            input.provider_kind.as_storage().as_bytes(),
            input.provider_model.as_bytes(),
            revision.as_slice(),
            input.prompt_version.as_bytes(),
            input.schema_id.as_bytes(),
            input.mcp_provenance_hash.as_bytes(),
        ],
    )
}

pub(super) fn request(
    identity: &ArtifactIdentity,
    trigger: ContentJobTrigger,
    call_chain_id: &str,
    remaining_depth: u8,
    max_attempts: u8,
    timeout_seconds: u16,
) -> String {
    let provider_revision = identity.provider_revision().to_be_bytes();
    let remaining_depth = remaining_depth.to_be_bytes();
    let max_attempts = max_attempts.to_be_bytes();
    let timeout_seconds = timeout_seconds.to_be_bytes();
    framed_hash(
        REQUEST_CONTEXT,
        [
            identity.user_id().as_bytes(),
            identity.entry_id().as_bytes(),
            identity.kind().as_storage().as_bytes(),
            identity.target_locale().unwrap_or_default().as_bytes(),
            identity.entry_content_hash().as_bytes(),
            identity.input_hash().as_bytes(),
            identity.config_hash().as_bytes(),
            identity.plugin_key().as_bytes(),
            identity.plugin_version().as_bytes(),
            identity.component_digest().as_bytes(),
            identity.provider_binding_id().as_bytes(),
            identity.provider_kind().as_storage().as_bytes(),
            identity.provider_model().as_bytes(),
            provider_revision.as_slice(),
            identity.prompt_version().as_bytes(),
            identity.schema_id().as_bytes(),
            identity.mcp_provenance_hash().as_bytes(),
            trigger.as_storage().as_bytes(),
            call_chain_id.as_bytes(),
            remaining_depth.as_slice(),
            max_attempts.as_slice(),
            timeout_seconds.as_slice(),
        ],
    )
}

pub(super) fn canonical_json(
    value: Value,
    max_bytes: usize,
    too_large: ContentRepositoryErrorKind,
) -> Result<String, ContentRepositoryError> {
    let normalized = normalize(value);
    let encoded = serde_json::to_string(&normalized)
        .map_err(|_| ContentRepositoryError::new(ContentRepositoryErrorKind::InvalidInput))?;
    if encoded.len() > max_bytes {
        return Err(ContentRepositoryError::new(too_large));
    }
    Ok(encoded)
}

fn normalize(value: Value) -> Value {
    match value {
        Value::Array(values) => Value::Array(values.into_iter().map(normalize).collect()),
        Value::Object(values) => {
            let mut entries = values.into_iter().collect::<Vec<_>>();
            entries.sort_by(|left, right| left.0.cmp(&right.0));
            let mut normalized = Map::new();
            for (key, value) in entries {
                normalized.insert(key, normalize(value));
            }
            Value::Object(normalized)
        }
        scalar => scalar,
    }
}

fn framed_hash<'a>(context: &str, frames: impl IntoIterator<Item = &'a [u8]>) -> String {
    let mut hasher = Hasher::new_derive_key(context);
    for frame in frames {
        hasher.update(&(frame.len() as u64).to_be_bytes());
        hasher.update(frame);
    }
    hasher.finalize().to_hex().to_string()
}

#[cfg(test)]
mod tests {
    use super::framed_hash;

    #[test]
    fn framing_prevents_concatenation_ambiguity() {
        assert_ne!(
            framed_hash("raindrop.test.frames.v1", [b"ab".as_slice(), b"c"]),
            framed_hash("raindrop.test.frames.v1", [b"a".as_slice(), b"bc"]),
        );
    }

    #[test]
    fn domain_separation_changes_the_digest() {
        assert_ne!(
            framed_hash("raindrop.test.left.v1", [b"same".as_slice()]),
            framed_hash("raindrop.test.right.v1", [b"same".as_slice()]),
        );
    }
}
