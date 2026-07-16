use std::fmt;

use base64::Engine;
use http::HeaderValue;

use super::{NormalizedFeedUrl, ValidatorError};

const MAX_VALIDATOR_BYTES: usize = 8_192;
const STORAGE_PREFIX: &str = "v1:";

#[derive(Eq, PartialEq)]
pub struct OpaqueValidator {
    header: HeaderValue,
    storage: String,
}

impl OpaqueValidator {
    pub fn from_header(mut header: HeaderValue) -> Result<Self, ValidatorError> {
        validate_length(header.as_bytes().len())?;
        header.set_sensitive(true);
        let storage = encode_storage(header.as_bytes());
        Ok(Self { header, storage })
    }

    pub fn from_storage(storage: &str) -> Result<Self, ValidatorError> {
        let Some(encoded) = storage.strip_prefix(STORAGE_PREFIX) else {
            return Err(ValidatorError::UnsupportedVersion);
        };
        if encoded.is_empty() {
            return Err(ValidatorError::Empty);
        }
        if !encoded
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
        {
            return Err(ValidatorError::InvalidEncoding);
        }

        let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(encoded)
            .map_err(|_| ValidatorError::InvalidEncoding)?;
        validate_length(bytes.len())?;
        if base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(&bytes) != encoded {
            return Err(ValidatorError::InvalidEncoding);
        }

        let mut header =
            HeaderValue::from_bytes(&bytes).map_err(|_| ValidatorError::InvalidHeaderValue)?;
        header.set_sensitive(true);
        Ok(Self {
            header,
            storage: storage.to_owned(),
        })
    }

    #[must_use]
    pub fn storage_value(&self) -> &str {
        &self.storage
    }

    #[must_use]
    pub fn header_value(&self) -> HeaderValue {
        let mut header = self.header.clone();
        header.set_sensitive(true);
        header
    }
}

impl Clone for OpaqueValidator {
    fn clone(&self) -> Self {
        Self {
            header: self.header_value(),
            storage: self.storage.clone(),
        }
    }
}

impl fmt::Debug for OpaqueValidator {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("OpaqueValidator([REDACTED])")
    }
}

pub struct ValidatorSet {
    validator_url: String,
    etag: Option<OpaqueValidator>,
    last_modified: Option<OpaqueValidator>,
}

impl ValidatorSet {
    #[must_use]
    pub fn new(
        validator_url: &NormalizedFeedUrl,
        etag: Option<OpaqueValidator>,
        last_modified: Option<OpaqueValidator>,
    ) -> Self {
        Self {
            validator_url: validator_url.complete().to_owned(),
            etag,
            last_modified,
        }
    }

    #[must_use]
    pub fn for_request<'a>(
        &'a self,
        request_url: &NormalizedFeedUrl,
    ) -> Option<ReusableValidators<'a>> {
        (self.validator_url == request_url.complete()).then_some(ReusableValidators { set: self })
    }
}

impl fmt::Debug for ValidatorSet {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ValidatorSet")
            .field("validator_url", &"[REDACTED]")
            .field("etag", &self.etag.as_ref().map(|_| "[REDACTED]"))
            .field(
                "last_modified",
                &self.last_modified.as_ref().map(|_| "[REDACTED]"),
            )
            .finish()
    }
}

#[derive(Clone, Copy)]
pub struct ReusableValidators<'a> {
    set: &'a ValidatorSet,
}

impl ReusableValidators<'_> {
    #[must_use]
    pub fn etag(self) -> Option<HeaderValue> {
        self.set.etag.as_ref().map(OpaqueValidator::header_value)
    }

    #[must_use]
    pub fn last_modified(self) -> Option<HeaderValue> {
        self.set
            .last_modified
            .as_ref()
            .map(OpaqueValidator::header_value)
    }
}

impl fmt::Debug for ReusableValidators<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ReusableValidators([REDACTED])")
    }
}

fn validate_length(length: usize) -> Result<(), ValidatorError> {
    match length {
        0 => Err(ValidatorError::Empty),
        1..=MAX_VALIDATOR_BYTES => Ok(()),
        _ => Err(ValidatorError::TooLong),
    }
}

fn encode_storage(bytes: &[u8]) -> String {
    let encoded = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes);
    let mut storage = String::with_capacity(STORAGE_PREFIX.len() + encoded.len());
    storage.push_str(STORAGE_PREFIX);
    storage.push_str(&encoded);
    storage
}
