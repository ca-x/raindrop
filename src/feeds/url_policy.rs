use std::fmt;

use url::{Host, Url};

use super::FeedUrlError;

const MAX_URL_BYTES: usize = 4_096;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FeedUrlPolicy {
    allow_insecure_http: bool,
}

impl FeedUrlPolicy {
    #[must_use]
    pub const fn new(allow_insecure_http: bool) -> Self {
        Self {
            allow_insecure_http,
        }
    }

    pub fn normalize(&self, raw: &str) -> Result<NormalizedFeedUrl, FeedUrlError> {
        normalize(raw, self.allow_insecure_http)
    }

    pub fn normalize_redirect(
        &self,
        previous: &NormalizedFeedUrl,
        raw: &str,
    ) -> Result<NormalizedFeedUrl, FeedUrlError> {
        let next = self.normalize(raw)?;
        if previous.scheme == "https" && next.scheme == "http" {
            return Err(FeedUrlError::HttpsDowngrade);
        }
        Ok(next)
    }
}

#[derive(Clone, Eq, PartialEq)]
pub struct NormalizedFeedUrl {
    complete: String,
    hash: String,
    canonical_host: String,
    scheme: &'static str,
    effective_port: u16,
}

impl NormalizedFeedUrl {
    #[must_use]
    pub fn url_hash(&self) -> &str {
        &self.hash
    }

    #[must_use]
    pub fn canonical_host(&self) -> &str {
        &self.canonical_host
    }

    #[must_use]
    pub const fn scheme(&self) -> &str {
        self.scheme
    }

    #[must_use]
    pub const fn effective_port(&self) -> u16 {
        self.effective_port
    }

    pub(crate) fn complete(&self) -> &str {
        &self.complete
    }
}

impl fmt::Debug for NormalizedFeedUrl {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NormalizedFeedUrl")
            .field("url_hash", &self.hash)
            .field("canonical_host", &self.canonical_host)
            .field("scheme", &self.scheme)
            .field("effective_port", &self.effective_port)
            .finish()
    }
}

pub(crate) fn normalize_identity_url(raw: &str) -> Result<String, FeedUrlError> {
    normalize(raw, true).map(|url| url.complete)
}

fn normalize(raw: &str, allow_insecure_http: bool) -> Result<NormalizedFeedUrl, FeedUrlError> {
    validate_raw(raw)?;

    if authority_has_userinfo(raw) {
        return Err(FeedUrlError::CredentialsForbidden);
    }
    if raw_authority(raw).is_some_and(|authority| authority.ends_with(':')) {
        return Err(FeedUrlError::Invalid);
    }

    let mut url = Url::parse(raw).map_err(|_| FeedUrlError::Invalid)?;
    let scheme = match url.scheme() {
        "https" => "https",
        "http" if allow_insecure_http => "http",
        "http" => return Err(FeedUrlError::InsecureHttpDisabled),
        _ => return Err(FeedUrlError::UnsupportedScheme),
    };

    if !url.username().is_empty() || url.password().is_some() {
        return Err(FeedUrlError::CredentialsForbidden);
    }

    let host = url.host().ok_or(FeedUrlError::MissingHost)?.to_owned();
    let canonical_host = match host {
        Host::Domain(domain) => {
            let domain = domain.strip_suffix('.').unwrap_or(&domain);
            validate_domain(domain)?;
            url.set_host(Some(domain))
                .map_err(|_| FeedUrlError::Invalid)?;
            domain.to_owned()
        }
        Host::Ipv4(address) => address.to_string(),
        Host::Ipv6(address) => address.to_string(),
    };

    let default_port = if scheme == "https" { 443 } else { 80 };
    let effective_port = url.port().unwrap_or(default_port);
    if url.port() == Some(default_port) {
        url.set_port(None).map_err(|_| FeedUrlError::Invalid)?;
    }
    url.set_fragment(None);

    let complete = url.to_string();
    if complete.len() > MAX_URL_BYTES {
        return Err(FeedUrlError::TooLong);
    }

    let hash = blake3::hash(complete.as_bytes()).to_hex().to_string();
    Ok(NormalizedFeedUrl {
        complete,
        hash,
        canonical_host,
        scheme,
        effective_port,
    })
}

fn validate_raw(raw: &str) -> Result<(), FeedUrlError> {
    if raw.is_empty() {
        return Err(FeedUrlError::Empty);
    }
    if raw.len() > MAX_URL_BYTES {
        return Err(FeedUrlError::TooLong);
    }
    if raw
        .chars()
        .any(|character| character == ' ' || character.is_control())
    {
        return Err(FeedUrlError::ControlCharacter);
    }
    Ok(())
}

fn authority_has_userinfo(raw: &str) -> bool {
    raw_authority(raw).is_some_and(|authority| authority.contains('@'))
}

fn raw_authority(raw: &str) -> Option<&str> {
    let scheme_end = raw.find(':')?;
    let after_scheme = &raw[scheme_end + 1..];
    let authority = after_scheme.strip_prefix("//")?;
    let authority_end = authority
        .find(['/', '?', '#', '\\'])
        .unwrap_or(authority.len());
    Some(&authority[..authority_end])
}

fn validate_domain(domain: &str) -> Result<(), FeedUrlError> {
    if domain.is_empty() || domain.len() > 253 {
        return Err(FeedUrlError::Invalid);
    }

    for label in domain.split('.') {
        if label.is_empty()
            || label.len() > 63
            || !label
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
            || !label
                .as_bytes()
                .first()
                .is_some_and(u8::is_ascii_alphanumeric)
            || !label
                .as_bytes()
                .last()
                .is_some_and(u8::is_ascii_alphanumeric)
        {
            return Err(FeedUrlError::Invalid);
        }
    }

    Ok(())
}
