use std::{error::Error, fmt, str, time::UNIX_EPOCH};

use http::{HeaderValue, StatusCode};
use time::{Duration, OffsetDateTime, PrimitiveDateTime, UtcOffset};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProviderTimeoutStage {
    Dns,
    Connect,
    FirstByte,
    BodyIdle,
    Total,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProviderTransportErrorKind {
    Configuration,
    InvalidEndpoint,
    Dns,
    AddressCount,
    AddressDenied,
    InvalidHeaders,
    Network,
    Timeout,
    PeerMismatch,
    RedirectDenied,
    ResponseHeaders,
    ResponseTooLarge,
    Decode,
}

enum ProviderTransportErrorSource {
    Reqwest(reqwest::Error),
}

pub struct ProviderTransportError {
    provider_id: String,
    kind: ProviderTransportErrorKind,
    stage: Option<ProviderTimeoutStage>,
    count: Option<usize>,
    source: Option<ProviderTransportErrorSource>,
}

impl ProviderTransportError {
    pub(super) fn new(provider_id: &str, kind: ProviderTransportErrorKind) -> Self {
        Self {
            provider_id: provider_id.to_owned(),
            kind,
            stage: None,
            count: None,
            source: None,
        }
    }

    pub(super) fn timeout(provider_id: &str, stage: ProviderTimeoutStage) -> Self {
        let mut error = Self::new(provider_id, ProviderTransportErrorKind::Timeout);
        error.stage = Some(stage);
        error
    }

    pub(super) fn with_count(mut self, count: usize) -> Self {
        self.count = Some(count);
        self
    }

    pub(super) fn reqwest(provider_id: &str, error: reqwest::Error) -> Self {
        let mut result = Self::new(provider_id, ProviderTransportErrorKind::Network);
        result.source = Some(ProviderTransportErrorSource::Reqwest(error.without_url()));
        result
    }

    #[must_use]
    pub const fn kind(&self) -> ProviderTransportErrorKind {
        self.kind
    }

    #[must_use]
    pub const fn stage(&self) -> Option<ProviderTimeoutStage> {
        self.stage
    }

    #[must_use]
    pub const fn count(&self) -> Option<usize> {
        self.count
    }

    #[must_use]
    pub fn provider_id(&self) -> &str {
        &self.provider_id
    }
}

impl fmt::Debug for ProviderTransportError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProviderTransportError")
            .field("provider_id", &self.provider_id)
            .field("kind", &self.kind)
            .field("stage", &self.stage)
            .field("count", &self.count)
            .finish()
    }
}

impl fmt::Display for ProviderTransportError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self.kind {
            ProviderTransportErrorKind::Configuration => {
                "AI provider transport configuration failed"
            }
            ProviderTransportErrorKind::InvalidEndpoint => "AI provider endpoint is invalid",
            ProviderTransportErrorKind::Dns => "AI provider DNS lookup failed",
            ProviderTransportErrorKind::AddressCount => "AI provider DNS answer count is invalid",
            ProviderTransportErrorKind::AddressDenied => "AI provider address is not allowed",
            ProviderTransportErrorKind::InvalidHeaders => "AI provider request headers are invalid",
            ProviderTransportErrorKind::Network => "AI provider network request failed",
            ProviderTransportErrorKind::Timeout => "AI provider network request timed out",
            ProviderTransportErrorKind::PeerMismatch => "AI provider connected peer is invalid",
            ProviderTransportErrorKind::RedirectDenied => "AI provider redirect is not allowed",
            ProviderTransportErrorKind::ResponseHeaders => {
                "AI provider response headers are invalid"
            }
            ProviderTransportErrorKind::ResponseTooLarge => "AI provider response is too large",
            ProviderTransportErrorKind::Decode => "AI provider response encoding is invalid",
        })
    }
}

impl Error for ProviderTransportError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self.source.as_ref() {
            Some(ProviderTransportErrorSource::Reqwest(error)) => Some(error),
            None => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ProviderRetryAfter {
    at: OffsetDateTime,
}

impl ProviderRetryAfter {
    pub(super) fn parse(raw: &HeaderValue, received_at: OffsetDateTime) -> Result<Self, ()> {
        let bytes = trim_optional_whitespace(raw.as_bytes());
        if bytes.is_empty() {
            return Err(());
        }
        let received_at = received_at.to_offset(UtcOffset::UTC);
        let at = if bytes.iter().all(u8::is_ascii_digit) {
            let digits = str::from_utf8(bytes).map_err(|_| ())?;
            let seconds = digits.parse::<u64>().map_err(|_| ())?;
            if seconds > i64::MAX as u64 {
                maximum_utc()
            } else {
                received_at
                    .checked_add(Duration::seconds(seconds as i64))
                    .unwrap_or_else(maximum_utc)
            }
        } else {
            let date = str::from_utf8(bytes).map_err(|_| ())?;
            let parsed = httpdate::parse_http_date(date).map_err(|_| ())?;
            let since_epoch = parsed.duration_since(UNIX_EPOCH).map_err(|_| ())?;
            let seconds = i64::try_from(since_epoch.as_secs()).map_err(|_| ())?;
            OffsetDateTime::UNIX_EPOCH
                .checked_add(Duration::seconds(seconds))
                .ok_or(())?
        };
        Ok(Self { at })
    }

    #[must_use]
    pub const fn at(self) -> OffsetDateTime {
        self.at
    }
}

pub struct ProviderTransportResponse {
    status: StatusCode,
    body: Vec<u8>,
    retry_after: Option<ProviderRetryAfter>,
}

impl ProviderTransportResponse {
    pub(super) const fn new(
        status: StatusCode,
        body: Vec<u8>,
        retry_after: Option<ProviderRetryAfter>,
    ) -> Self {
        Self {
            status,
            body,
            retry_after,
        }
    }

    #[must_use]
    pub const fn status(&self) -> StatusCode {
        self.status
    }

    #[must_use]
    pub fn body(&self) -> &[u8] {
        &self.body
    }

    #[must_use]
    pub const fn retry_after(&self) -> Option<ProviderRetryAfter> {
        self.retry_after
    }
}

impl fmt::Debug for ProviderTransportResponse {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProviderTransportResponse")
            .field("status", &self.status)
            .field("body_bytes", &self.body.len())
            .field("retry_after", &self.retry_after)
            .finish()
    }
}

fn trim_optional_whitespace(mut bytes: &[u8]) -> &[u8] {
    while bytes
        .first()
        .is_some_and(|byte| matches!(byte, b' ' | b'\t'))
    {
        bytes = &bytes[1..];
    }
    while bytes
        .last()
        .is_some_and(|byte| matches!(byte, b' ' | b'\t'))
    {
        bytes = &bytes[..bytes.len() - 1];
    }
    bytes
}

fn maximum_utc() -> OffsetDateTime {
    PrimitiveDateTime::MAX.assume_utc()
}

#[cfg(test)]
mod tests {
    use super::*;

    const PROVIDER_ID: &str = "00000000-0000-4000-8000-000000000901";

    #[test]
    fn retry_after_accepts_delta_and_http_date_as_utc_deadlines() {
        let received_at = OffsetDateTime::UNIX_EPOCH
            .to_offset(UtcOffset::from_hms(8, 0, 0).expect("valid offset"));
        let delta = ProviderRetryAfter::parse(&HeaderValue::from_static(" 5\t"), received_at)
            .expect("delta seconds should parse");
        assert_eq!(
            delta.at(),
            OffsetDateTime::UNIX_EPOCH + Duration::seconds(5)
        );
        assert_eq!(delta.at().offset(), UtcOffset::UTC);

        let date = ProviderRetryAfter::parse(
            &HeaderValue::from_static("Wed, 21 Oct 2015 07:28:00 GMT"),
            received_at,
        )
        .expect("HTTP date should parse");
        assert_eq!(
            date.at(),
            OffsetDateTime::from_unix_timestamp(1_445_412_480).expect("valid timestamp")
        );
        assert_eq!(date.at().offset(), UtcOffset::UTC);
    }

    #[test]
    fn retry_after_rejects_empty_invalid_and_non_ascii_values() {
        for raw in [
            HeaderValue::from_static(""),
            HeaderValue::from_static("five"),
            HeaderValue::from_bytes(&[0xff]).expect("opaque header bytes are valid"),
        ] {
            assert!(ProviderRetryAfter::parse(&raw, OffsetDateTime::UNIX_EPOCH).is_err());
        }
    }

    #[test]
    fn transport_debug_contracts_do_not_expose_payload_or_sources() {
        let error = ProviderTransportError::timeout(PROVIDER_ID, ProviderTimeoutStage::FirstByte)
            .with_count(16);
        let error_debug = format!("{error:?}");
        assert!(error_debug.contains(PROVIDER_ID));
        assert!(error_debug.contains("FirstByte"));
        assert!(!error_debug.contains("credential-sentinel"));

        let response = ProviderTransportResponse::new(
            StatusCode::OK,
            b"credential-sentinel-response".to_vec(),
            None,
        );
        let response_debug = format!("{response:?}");
        assert!(response_debug.contains("body_bytes: 28"));
        assert!(!response_debug.contains("credential-sentinel-response"));
    }
}
