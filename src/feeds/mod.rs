mod address_policy;
mod content_storage;
mod deadline;
mod decode;
mod error;
mod fetch;
mod identity;
mod model;
mod parse;
mod persistence;
mod refresh;
mod repository;
mod resolver;
mod schedule;
mod url_policy;

#[cfg(test)]
mod test_support;

pub use address_policy::AddressPolicy;
pub use content_storage::{EncodedEntryContent, EntryContentDetail, EntryContentError};
pub use error::{
    AddressPolicyError, FeedUrlError, IdentityError, RetryAfterError, ScheduleError, ValidatorError,
};
pub use fetch::{
    CryptoProviderError, FeedFetchError, FeedFetchErrorKind, FeedTransport, FetchOutcome,
    FetchRequest, FetchTimeoutStage, HttpFeedTransport, Nat64Mode, install_ring_crypto_provider,
};
pub use identity::{EntryIdentity, IdentityKind, StableEntryFields};
pub use model::{OpaqueValidator, ReusableValidators, ValidatorSet};
pub use parse::{
    FeedParseError, FeedParseErrorKind, FeedParser, FetchedDocument, FetchedDocumentError,
    ParsedEnclosure, ParsedEntry, ParsedFeed, ParsedFeedVersion, ParsedSource,
};
pub use persistence::{PersistEntry, PersistFeed, PersistResult};
pub use refresh::{
    ClaimRequest, QueueRefreshRequest, RefreshClaim, RefreshCounts, RefreshFailure,
    RefreshRepositoryError, RefreshRun, RefreshStatus, RefreshTrigger, UnknownRefreshValue,
};
pub use repository::FeedRepository;
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
