use std::{fmt, str::FromStr, time::Duration};

use time::OffsetDateTime;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum RefreshTrigger {
    Scheduled,
    Manual,
    Subscribe,
    Import,
    Retry,
}

impl RefreshTrigger {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Scheduled => "SCHEDULED",
            Self::Manual => "MANUAL",
            Self::Subscribe => "SUBSCRIBE",
            Self::Import => "IMPORT",
            Self::Retry => "RETRY",
        }
    }
}

impl fmt::Display for RefreshTrigger {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for RefreshTrigger {
    type Err = UnknownRefreshValue;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "SCHEDULED" => Ok(Self::Scheduled),
            "MANUAL" => Ok(Self::Manual),
            "SUBSCRIBE" => Ok(Self::Subscribe),
            "IMPORT" => Ok(Self::Import),
            "RETRY" => Ok(Self::Retry),
            _ => Err(UnknownRefreshValue),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum RefreshStatus {
    Queued,
    Running,
    Success,
    NotModified,
    Partial,
    Error,
    LeaseLost,
    Cancelled,
}

impl RefreshStatus {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "QUEUED",
            Self::Running => "RUNNING",
            Self::Success => "SUCCESS",
            Self::NotModified => "NOT_MODIFIED",
            Self::Partial => "PARTIAL",
            Self::Error => "ERROR",
            Self::LeaseLost => "LEASE_LOST",
            Self::Cancelled => "CANCELLED",
        }
    }
}

impl fmt::Display for RefreshStatus {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for RefreshStatus {
    type Err = UnknownRefreshValue;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "QUEUED" => Ok(Self::Queued),
            "RUNNING" => Ok(Self::Running),
            "SUCCESS" => Ok(Self::Success),
            "NOT_MODIFIED" => Ok(Self::NotModified),
            "PARTIAL" => Ok(Self::Partial),
            "ERROR" => Ok(Self::Error),
            "LEASE_LOST" => Ok(Self::LeaseLost),
            "CANCELLED" => Ok(Self::Cancelled),
            _ => Err(UnknownRefreshValue),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UnknownRefreshValue;

impl fmt::Display for UnknownRefreshValue {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("unknown refresh value")
    }
}

impl std::error::Error for UnknownRefreshValue {}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct QueueRefreshRequest {
    pub feed_id: String,
    pub requested_by_user_id: Option<String>,
    pub trigger: RefreshTrigger,
    pub idempotency_key: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ClaimRequest {
    pub owner: String,
    pub lease_duration: Duration,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RefreshRun {
    pub id: String,
    pub feed_id: String,
    pub requested_by_user_id: Option<String>,
    pub trigger: RefreshTrigger,
    pub status: RefreshStatus,
    pub idempotency_key: String,
    pub lease_token: Option<i64>,
    pub queued_at: OffsetDateTime,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RefreshClaim {
    pub run_id: String,
    pub feed_id: String,
    pub owner: String,
    pub lease_token: i64,
    pub lease_deadline: OffsetDateTime,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ExactClaimResult {
    Claimed(RefreshClaim),
    TemporarilyBlocked,
    FeedDisabled,
    Existing(RefreshStatus),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RefreshCounts {
    pub new_count: i32,
    pub updated_count: i32,
    pub dropped_count: i32,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RefreshFailure {
    pub error_code: String,
    pub http_status: Option<i32>,
    pub retry_at: Option<OffsetDateTime>,
}

#[derive(thiserror::Error)]
pub enum RefreshRepositoryError {
    #[error("refresh repository database operation failed")]
    Database(#[source] sea_orm::DbErr),
    #[error("refresh request is invalid")]
    InvalidRequest,
    #[error("refresh idempotency key has conflicting semantics")]
    IdempotencyConflict,
    #[error("refresh lifecycle event has conflicting semantics")]
    LifecycleEventConflict,
    #[error("refresh lifecycle payload exceeds the size limit")]
    LifecyclePayloadTooLarge,
    #[error("refresh lifecycle payload serialization failed")]
    InvalidLifecyclePayload,
    #[error("refresh lease token is exhausted")]
    TokenExhausted,
    #[error("refresh repository data is corrupt")]
    CorruptData,
    #[error("refresh lease authorization failed")]
    LeaseLost,
    #[error("refresh status transition is invalid")]
    InvalidTransition,
    #[error("refresh run was not found")]
    RunNotFound,
    #[error("entry content storage validation failed")]
    Content(#[source] super::EntryContentError),
    #[error("entry content serialization failed")]
    InvalidContent,
    #[error("entry identity hash collision detected")]
    IdentityHashCollision,
    #[error("persisted validator data is corrupt")]
    CorruptValidator,
    #[error("database time could not be represented")]
    InvalidTime,
    #[error("ingest generation is exhausted")]
    GenerationExhausted,
    #[error("feed entry sequence is exhausted")]
    SequenceExhausted,
    #[error("refresh count is too large")]
    CountOverflow,
}

impl fmt::Debug for RefreshRepositoryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Database(_) => "RefreshRepositoryError::Database([REDACTED])",
            Self::InvalidRequest => "RefreshRepositoryError::InvalidRequest",
            Self::IdempotencyConflict => "RefreshRepositoryError::IdempotencyConflict",
            Self::LifecycleEventConflict => "RefreshRepositoryError::LifecycleEventConflict",
            Self::LifecyclePayloadTooLarge => "RefreshRepositoryError::LifecyclePayloadTooLarge",
            Self::InvalidLifecyclePayload => "RefreshRepositoryError::InvalidLifecyclePayload",
            Self::TokenExhausted => "RefreshRepositoryError::TokenExhausted",
            Self::CorruptData => "RefreshRepositoryError::CorruptData",
            Self::LeaseLost => "RefreshRepositoryError::LeaseLost",
            Self::InvalidTransition => "RefreshRepositoryError::InvalidTransition",
            Self::RunNotFound => "RefreshRepositoryError::RunNotFound",
            Self::Content(_) => "RefreshRepositoryError::Content([REDACTED])",
            Self::InvalidContent => "RefreshRepositoryError::InvalidContent",
            Self::IdentityHashCollision => "RefreshRepositoryError::IdentityHashCollision",
            Self::CorruptValidator => "RefreshRepositoryError::CorruptValidator",
            Self::InvalidTime => "RefreshRepositoryError::InvalidTime",
            Self::GenerationExhausted => "RefreshRepositoryError::GenerationExhausted",
            Self::SequenceExhausted => "RefreshRepositoryError::SequenceExhausted",
            Self::CountOverflow => "RefreshRepositoryError::CountOverflow",
        })
    }
}

impl From<sea_orm::DbErr> for RefreshRepositoryError {
    fn from(value: sea_orm::DbErr) -> Self {
        Self::Database(value)
    }
}
