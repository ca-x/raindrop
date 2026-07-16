use thiserror::Error;

#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum FeedUrlError {
    #[error("feed URL is empty")]
    Empty,
    #[error("feed URL exceeds the size limit")]
    TooLong,
    #[error("feed URL contains a forbidden control character or space")]
    ControlCharacter,
    #[error("feed URL is invalid")]
    Invalid,
    #[error("feed URL scheme is unsupported")]
    UnsupportedScheme,
    #[error("insecure HTTP feeds are disabled")]
    InsecureHttpDisabled,
    #[error("feed URL credentials are forbidden")]
    CredentialsForbidden,
    #[error("feed URL has no host")]
    MissingHost,
    #[error("HTTPS redirects may not downgrade to HTTP")]
    HttpsDowngrade,
}

#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum AddressPolicyError {
    #[error("NAT64 prefix length is not supported")]
    InvalidPrefixLength,
    #[error("NAT64 prefix contains host bits")]
    NonCanonical,
    #[error("NAT64 /96 prefix has a non-zero u octet")]
    NonZeroUOctet,
    #[error("NAT64 prefix is outside allowed global-unicast space")]
    OutsideAllowedIpv6,
    #[error("NAT64 prefix overlaps a reserved transition or special range")]
    SpecialRange,
    #[error("NAT64 prefixes overlap")]
    Overlap,
}

#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum IdentityError {
    #[error("entry identity input exceeds the size limit")]
    TooLong,
    #[error("entry identity URL is invalid")]
    InvalidUrl,
    #[error("entry identity URL credentials are forbidden")]
    CredentialsForbidden,
}

#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum ValidatorError {
    #[error("validator is empty")]
    Empty,
    #[error("validator exceeds the size limit")]
    TooLong,
    #[error("validator storage version is unsupported")]
    UnsupportedVersion,
    #[error("validator storage encoding is invalid")]
    InvalidEncoding,
    #[error("validator header bytes are invalid")]
    InvalidHeaderValue,
}

#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum RetryAfterError {
    #[error("Retry-After is empty")]
    Empty,
    #[error("Retry-After syntax is invalid")]
    Invalid,
    #[error("Retry-After delta seconds overflow")]
    DeltaOverflow,
}

#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum ScheduleError {
    #[error("persisted failure count is negative")]
    NegativeFailureCount,
    #[error("jitter source returned a value above its bound")]
    InvalidJitter,
    #[error("scheduled time is outside the supported UTC range")]
    TimeOverflow,
}
