mod address_policy;
mod bulk_read;
mod content_storage;
mod deadline;
mod decode;
mod dto;
mod error;
mod fetch;
mod identity;
mod lifecycle;
mod model;
mod opml;
mod parse;
mod persistence;
mod query;
mod refresh;
mod repository;
mod resolver;
mod retention;
mod runtime;
mod schedule;
mod service;
mod state;
mod subscription;
mod url_policy;

#[cfg(test)]
mod test_support;

pub use address_policy::AddressPolicy;
pub use bulk_read::{MarkReadResult, MarkReadScope};
pub use content_storage::{EncodedEntryContent, EntryContentDetail, EntryContentError};
pub use dto::{
    EnclosureDto, EntryDetailDto, EntryListItemDto, EntryPage, EntryStateDto, InertImageDto,
    ListSubscriptionsQuery, PatchValue, QueueSubscriptionRefresh, RefreshDto, SubscribeInput,
    SubscribeOutcome, SubscriptionListItemDto, SubscriptionPage, SubscriptionPatchError,
    UpdateEntryState, UpdateSubscription,
};
pub use error::{
    AddressPolicyError, FeedUrlError, IdentityError, RetryAfterError, ScheduleError, ValidatorError,
};
pub use fetch::{
    CryptoProviderError, FeedFetchError, FeedFetchErrorKind, FeedTransport, FetchOutcome,
    FetchRequest, FetchTimeoutStage, HttpExecutionCounter, HttpFeedTransport, Nat64Mode,
    install_ring_crypto_provider,
};
pub use identity::{EntryIdentity, IdentityKind, StableEntryFields};
pub use model::{OpaqueValidator, ReusableValidators, ValidatorSet};
pub use opml::{
    MAX_OPML_BYTES, MAX_OPML_OUTLINES, OpmlDocument, OpmlError, OpmlImportResult, OpmlPreview,
};
pub use parse::{
    FeedParseError, FeedParseErrorKind, FeedParser, FetchedDocument, FetchedDocumentError,
    ParsedEnclosure, ParsedEntry, ParsedFeed, ParsedFeedVersion, ParsedSource,
};
pub use persistence::{PersistEntry, PersistFeed, PersistResult};
#[cfg(debug_assertions)]
#[doc(hidden)]
pub use persistence::{
    persistence_new_entry_insert_batch_sizes, persistence_peak_full_existing_entry_batch,
    reset_new_entry_insert_batch_observation, reset_persistence_batch_observation,
};
pub use query::{EntryListState, ListEntriesQuery, RepositoryError};
pub use refresh::{
    ClaimRequest, ExactClaimResult, QueueRefreshRequest, RefreshClaim, RefreshCounts,
    RefreshFailure, RefreshRepositoryError, RefreshRun, RefreshStatus, RefreshTrigger,
    UnknownRefreshValue,
};
pub use repository::FeedRepository;
pub use retention::{FeedRetentionError, FeedRetentionPolicy};
pub use runtime::{FeedRuntime, FeedRuntimeHandle};
pub use schedule::{JitterSource, RefreshResult, RefreshSchedule, RetryAfter, ScheduleOutcome};
pub use service::{FeedCommandService, FeedExecutor, FeedServiceError};
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
