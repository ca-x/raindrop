use std::fmt;

use super::FeedUrlError;
use super::IdentityError;
use super::url_policy::{has_http_scheme, normalize_identity_url};

const MAX_IDENTITY_BYTES: usize = 65_536;
const FINGERPRINT_CONTEXT: &str = "raindrop.entry-fingerprint.v1";
const INDEX_CONTEXT: &str = "raindrop.entry-identity-index.v1";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IdentityKind {
    Guid,
    Url,
    Fingerprint,
}

impl IdentityKind {
    #[must_use]
    pub const fn as_database_str(self) -> &'static str {
        match self {
            Self::Guid => "GUID",
            Self::Url => "URL",
            Self::Fingerprint => "FINGERPRINT",
        }
    }

    const fn tag(self) -> u8 {
        match self {
            Self::Guid => 1,
            Self::Url => 2,
            Self::Fingerprint => 3,
        }
    }
}

#[derive(Clone, Eq, PartialEq)]
pub struct StableEntryFields {
    title: Option<String>,
    author: Option<String>,
    published_at_us: Option<i64>,
    first_enclosure_url: Option<String>,
    sanitized_content_hash: Option<[u8; 32]>,
}

impl fmt::Debug for StableEntryFields {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("StableEntryFields")
            .field("title", &self.title.as_ref().map(|_| "[REDACTED]"))
            .field("author", &self.author.as_ref().map(|_| "[REDACTED]"))
            .field("published_at_us", &self.published_at_us)
            .field(
                "first_enclosure_url",
                &self.first_enclosure_url.as_ref().map(|_| "[REDACTED]"),
            )
            .field(
                "sanitized_content_hash",
                &self.sanitized_content_hash.as_ref().map(|_| "[REDACTED]"),
            )
            .finish()
    }
}

impl StableEntryFields {
    pub fn new(
        title: Option<&str>,
        author: Option<&str>,
        published_at_us: Option<i64>,
        first_enclosure_url: Option<&str>,
        sanitized_content_hash: Option<[u8; 32]>,
    ) -> Result<Self, IdentityError> {
        let title = normalize_stable_text(title)?;
        let author = normalize_stable_text(author)?;
        let first_enclosure_url = first_enclosure_url
            .map(normalize_required_identity_url)
            .transpose()?;
        let sanitized_content_hash = if title.is_none()
            && author.is_none()
            && published_at_us.is_none()
            && first_enclosure_url.is_none()
        {
            sanitized_content_hash
        } else {
            None
        };

        Ok(Self {
            title,
            author,
            published_at_us,
            first_enclosure_url,
            sanitized_content_hash,
        })
    }

    #[must_use]
    pub fn encode_v1(&self) -> Vec<u8> {
        let mut encoded = Vec::with_capacity(128);
        encoded.extend_from_slice(b"RDFP\0\x01");
        append_optional(&mut encoded, 1, self.title.as_deref().map(str::as_bytes));
        append_optional(&mut encoded, 2, self.author.as_deref().map(str::as_bytes));
        let published_bytes = self.published_at_us.map(i64::to_be_bytes);
        append_optional(
            &mut encoded,
            3,
            published_bytes.as_ref().map(<[u8; 8]>::as_slice),
        );
        append_optional(
            &mut encoded,
            4,
            self.first_enclosure_url.as_deref().map(str::as_bytes),
        );
        append_optional(
            &mut encoded,
            5,
            self.sanitized_content_hash
                .as_ref()
                .map(<[u8; 32]>::as_slice),
        );
        encoded
    }
}

#[derive(Clone, Eq, PartialEq)]
pub struct EntryIdentity {
    kind: IdentityKind,
    identity: String,
    index_hash: String,
}

impl EntryIdentity {
    pub fn from_parts(
        guid: Option<&str>,
        canonical_url: Option<&str>,
        stable_fields: StableEntryFields,
    ) -> Result<Self, IdentityError> {
        if let Some(guid) = normalize_guid(guid)? {
            if has_http_scheme(&guid) {
                match normalize_identity_url(&guid) {
                    Ok(url) => return Ok(Self::new(IdentityKind::Url, url)),
                    Err(FeedUrlError::CredentialsForbidden) => {
                        return Err(IdentityError::CredentialsForbidden);
                    }
                    Err(FeedUrlError::TooLong) => return Err(IdentityError::TooLong),
                    Err(_) => {}
                }
            }
            return Ok(Self::new(IdentityKind::Guid, guid));
        }

        if let Some(canonical_url) = canonical_url {
            let url = normalize_required_identity_url(canonical_url)?;
            return Ok(Self::new(IdentityKind::Url, url));
        }

        let frame = stable_fields.encode_v1();
        let fingerprint = derive_hex(FINGERPRINT_CONTEXT, &frame);
        Ok(Self::new(IdentityKind::Fingerprint, fingerprint))
    }

    #[must_use]
    pub const fn kind(&self) -> IdentityKind {
        self.kind
    }

    #[must_use]
    pub fn identity(&self) -> &str {
        &self.identity
    }

    #[must_use]
    pub fn index_hash(&self) -> &str {
        &self.index_hash
    }

    #[must_use]
    pub fn index_bytes_v1(&self) -> Vec<u8> {
        index_bytes(self.kind, &self.identity)
    }

    fn new(kind: IdentityKind, identity: String) -> Self {
        let index_hash = derive_hex(INDEX_CONTEXT, &index_bytes(kind, &identity));
        Self {
            kind,
            identity,
            index_hash,
        }
    }
}

impl fmt::Debug for EntryIdentity {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("EntryIdentity")
            .field("kind", &self.kind)
            .field("index_hash", &self.index_hash)
            .finish_non_exhaustive()
    }
}

fn normalize_guid(guid: Option<&str>) -> Result<Option<String>, IdentityError> {
    let Some(raw) = guid else {
        return Ok(None);
    };
    if raw.len() > MAX_IDENTITY_BYTES {
        return Err(IdentityError::TooLong);
    }
    let normalized = raw.trim_matches(char::is_whitespace);
    if normalized.len() > MAX_IDENTITY_BYTES {
        return Err(IdentityError::TooLong);
    }
    if normalized.is_empty() {
        Ok(None)
    } else {
        Ok(Some(normalized.to_owned()))
    }
}

fn normalize_stable_text(raw: Option<&str>) -> Result<Option<String>, IdentityError> {
    let Some(raw) = raw else {
        return Ok(None);
    };
    if raw.len() > MAX_IDENTITY_BYTES {
        return Err(IdentityError::TooLong);
    }

    let trimmed = raw.trim_matches(char::is_whitespace);
    if trimmed.is_empty() {
        return Ok(None);
    }

    let mut normalized = String::with_capacity(trimmed.len());
    let mut in_whitespace = false;
    for character in trimmed.chars() {
        if character.is_whitespace() {
            in_whitespace = true;
        } else {
            if in_whitespace && !normalized.is_empty() {
                normalized.push(' ');
            }
            normalized.push(character);
            in_whitespace = false;
        }
    }

    if normalized.len() > MAX_IDENTITY_BYTES {
        return Err(IdentityError::TooLong);
    }
    Ok(Some(normalized))
}

fn normalize_required_identity_url(raw: &str) -> Result<String, IdentityError> {
    normalize_identity_url(raw).map_err(|error| match error {
        FeedUrlError::CredentialsForbidden => IdentityError::CredentialsForbidden,
        FeedUrlError::TooLong => IdentityError::TooLong,
        _ => IdentityError::InvalidUrl,
    })
}

fn append_optional(encoded: &mut Vec<u8>, tag: u8, value: Option<&[u8]>) {
    encoded.push(tag);
    match value {
        Some(value) => {
            encoded.push(1);
            encoded.extend_from_slice(&(value.len() as u32).to_be_bytes());
            encoded.extend_from_slice(value);
        }
        None => {
            encoded.push(0);
            encoded.extend_from_slice(&0_u32.to_be_bytes());
        }
    }
}

fn index_bytes(kind: IdentityKind, identity: &str) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(11 + identity.len());
    bytes.extend_from_slice(b"RDIX\0\x01");
    bytes.push(kind.tag());
    bytes.extend_from_slice(&(identity.len() as u32).to_be_bytes());
    bytes.extend_from_slice(identity.as_bytes());
    bytes
}

fn derive_hex(context: &str, bytes: &[u8]) -> String {
    let mut hasher = blake3::Hasher::new_derive_key(context);
    hasher.update(bytes);
    hasher.finalize().to_hex().to_string()
}
