mod address_policy;
mod deadline;
mod decode;
mod error;
mod fetch;
mod identity;
mod model;
mod resolver;
mod schedule;
mod url_policy;

#[cfg(test)]
mod test_support;

pub use address_policy::AddressPolicy;
pub use error::{
    AddressPolicyError, FeedUrlError, IdentityError, RetryAfterError, ScheduleError, ValidatorError,
};
pub use fetch::{
    CryptoProviderError, FeedFetchError, FeedFetchErrorKind, FeedTransport, FetchOutcome,
    FetchRequest, FetchTimeoutStage, HttpFeedTransport, Nat64Mode, install_ring_crypto_provider,
};
pub use identity::{EntryIdentity, IdentityKind, StableEntryFields};
pub use model::{OpaqueValidator, ReusableValidators, ValidatorSet};
pub use schedule::{JitterSource, RefreshResult, RefreshSchedule, RetryAfter, ScheduleOutcome};
pub use url_policy::{FeedUrlPolicy, NormalizedFeedUrl};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AddressDecision {
    Allowed,
    Denied(AddressDenyReason),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AddressDenyReason {
    Ipv4Special,
    Ipv6OutsideGlobalUnicast,
    Ipv6Special,
    EmbeddedIpv4,
    Nat64UOctet,
    TeredoServer,
    TeredoClient,
    LocalUseNat64,
}
