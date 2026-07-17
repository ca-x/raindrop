use std::{fmt, time::Duration};

use rand_core::{OsRng, RngCore};
use tokio::sync::Mutex;
use uuid::Uuid;

use super::subscription::SubscriptionRepositoryError;
use super::{
    ClaimRequest, ExactClaimResult, FeedFetchError, FeedParser, FeedRepository, FeedTransport,
    FeedUrlError, FeedUrlPolicy, FetchOutcome, FetchRequest, FetchedDocument, JitterSource,
    PersistFeed, QueueRefreshRequest, RefreshDto, RefreshFailure, RefreshRepositoryError,
    RefreshResult, RefreshSchedule, RefreshStatus, RefreshTrigger, RepositoryError, ScheduleError,
    SubscribeInput, SubscriptionDto,
};

const CLAIM_ATTEMPTS: usize = 700;
const CLAIM_RETRY_DELAY: Duration = Duration::from_millis(100);
const LEASE_DURATION: Duration = Duration::from_secs(60);

pub struct FeedService<T>
where
    T: FeedTransport,
{
    repository: FeedRepository,
    url_policy: FeedUrlPolicy,
    transport: T,
    parser: FeedParser,
    schedule: Mutex<RefreshSchedule<Box<dyn JitterSource + Send>>>,
}

impl<T> FeedService<T>
where
    T: FeedTransport,
{
    #[must_use]
    pub fn new(repository: FeedRepository, url_policy: FeedUrlPolicy, transport: T) -> Self {
        Self::with_jitter(repository, url_policy, transport, SystemJitter)
    }

    #[must_use]
    pub fn with_jitter<J>(
        repository: FeedRepository,
        url_policy: FeedUrlPolicy,
        transport: T,
        jitter: J,
    ) -> Self
    where
        J: JitterSource + Send + 'static,
    {
        Self {
            repository,
            url_policy,
            transport,
            parser: FeedParser::new(),
            schedule: Mutex::new(RefreshSchedule::new(Box::new(jitter))),
        }
    }

    pub async fn subscribe(
        &self,
        user_id: &str,
        input: SubscribeInput,
    ) -> Result<SubscriptionDto, FeedServiceError> {
        validate_uuid(user_id).map_err(|()| FeedServiceError::InvalidUserId)?;
        let normalized = self
            .url_policy
            .normalize(&input.url)
            .map_err(FeedServiceError::Url)?;
        let record = self
            .repository
            .subscribe_transaction(user_id, &input.url, &normalized)
            .await
            .map_err(map_subscription_error)?;
        let refresh = self
            .execute_run(&record.feed_id, &record.run_id, record.run_status)
            .await?;
        self.repository
            .subscription_dto(user_id, &record.subscription_id, refresh)
            .await
            .map_err(map_subscription_error)
    }

    pub async fn refresh_subscription(
        &self,
        user_id: &str,
        subscription_id: &str,
    ) -> Result<RefreshDto, FeedServiceError> {
        validate_uuid(user_id).map_err(|()| FeedServiceError::InvalidUserId)?;
        validate_uuid(subscription_id).map_err(|()| FeedServiceError::InvalidSubscriptionId)?;
        let Some((_, feed_id)) = self
            .repository
            .find_owned_subscription(user_id, subscription_id)
            .await
            .map_err(map_subscription_error)?
        else {
            return Err(FeedServiceError::Unauthorized);
        };
        let nonce = Uuid::new_v4();
        let run = self
            .repository
            .queue_refresh(QueueRefreshRequest {
                feed_id: feed_id.clone(),
                requested_by_user_id: Some(user_id.to_owned()),
                trigger: RefreshTrigger::Manual,
                idempotency_key: format!("manual:{nonce}"),
            })
            .await
            .map_err(FeedServiceError::RefreshRepository)?;
        self.execute_run(&feed_id, &run.id, run.status).await
    }

    async fn execute_run(
        &self,
        feed_id: &str,
        run_id: &str,
        initial_status: RefreshStatus,
    ) -> Result<RefreshDto, FeedServiceError> {
        if is_terminal(initial_status) {
            return self.load_refresh(run_id).await;
        }
        let owner = format!("feed-service:{}", Uuid::new_v4());
        let mut claimed = None;
        for _ in 0..CLAIM_ATTEMPTS {
            match self
                .repository
                .claim_run(
                    run_id,
                    ClaimRequest {
                        owner: owner.clone(),
                        lease_duration: LEASE_DURATION,
                    },
                )
                .await
                .map_err(FeedServiceError::RefreshRepository)?
            {
                ExactClaimResult::Claimed(claim) => {
                    claimed = Some(claim);
                    break;
                }
                ExactClaimResult::Existing(status) => {
                    return if status == RefreshStatus::Running || is_terminal(status) {
                        self.load_refresh(run_id).await
                    } else {
                        Err(FeedServiceError::RunUnavailable)
                    };
                }
                ExactClaimResult::TemporarilyBlocked => {}
                ExactClaimResult::FeedDisabled => return Err(FeedServiceError::FeedDisabled),
            }
            tokio::time::sleep(CLAIM_RETRY_DELAY).await;
        }
        let claim = claimed.ok_or(FeedServiceError::RunUnavailable)?;
        if claim.feed_id != feed_id {
            return Err(FeedServiceError::RunMismatch);
        }

        let context = self
            .repository
            .load_refresh_context(feed_id)
            .await
            .map_err(map_subscription_error)?;
        let fetch_url = self
            .url_policy
            .normalize(&context.fetch_url)
            .map_err(|_| FeedServiceError::CorruptFeed)?;
        let validators = self
            .repository
            .load_validators(feed_id)
            .await
            .map_err(FeedServiceError::RefreshRepository)?;
        let outcome = self
            .transport
            .fetch(FetchRequest::new(fetch_url, validators))
            .await;

        match outcome {
            Ok(outcome @ FetchOutcome::Document { .. }) => {
                let document = match FetchedDocument::try_from(outcome) {
                    Ok(document) => document,
                    Err(_) => {
                        return self
                            .complete_failure(
                                &claim,
                                context.consecutive_failures,
                                "DOCUMENT_REJECTED",
                                None,
                                None,
                            )
                            .await;
                    }
                };
                let parsed = match self.parser.parse(document).await {
                    Ok(parsed) => parsed,
                    Err(_) => {
                        return self
                            .complete_failure(
                                &claim,
                                context.consecutive_failures,
                                "PARSE_FAILED",
                                None,
                                None,
                            )
                            .await;
                    }
                };
                let persisted = match PersistFeed::try_from(parsed) {
                    Ok(persisted) => persisted,
                    Err(error) if operational_persist_error(&error) => {
                        return self
                            .complete_failure(
                                &claim,
                                context.consecutive_failures,
                                "CONTENT_REJECTED",
                                None,
                                None,
                            )
                            .await;
                    }
                    Err(error) => return Err(FeedServiceError::RefreshRepository(error)),
                };
                let schedule = self
                    .schedule(context.consecutive_failures, RefreshResult::Success)
                    .await?;
                self.repository
                    .persist_feed_scheduled(&claim, persisted, schedule)
                    .await
                    .map_err(FeedServiceError::RefreshRepository)?;
                self.load_refresh(run_id).await
            }
            Ok(FetchOutcome::NotModified {
                url,
                etag,
                last_modified,
            }) => {
                let schedule = self
                    .schedule(context.consecutive_failures, RefreshResult::NotModified)
                    .await?;
                self.repository
                    .complete_not_modified_scheduled(
                        &claim,
                        &url,
                        etag.as_ref(),
                        last_modified.as_ref(),
                        schedule,
                    )
                    .await
                    .map_err(FeedServiceError::RefreshRepository)?;
                self.load_refresh(run_id).await
            }
            Err(error) => {
                let http_status = error.status().map(|status| i32::from(status.as_u16()));
                let retry_after = error.retry_after();
                self.complete_failure(
                    &claim,
                    context.consecutive_failures,
                    fetch_error_code(&error),
                    http_status,
                    retry_after,
                )
                .await
            }
        }
    }

    async fn complete_failure(
        &self,
        claim: &super::RefreshClaim,
        previous_failures: i64,
        error_code: &str,
        http_status: Option<i32>,
        retry_after: Option<super::RetryAfter>,
    ) -> Result<RefreshDto, FeedServiceError> {
        let schedule = self
            .schedule(
                previous_failures,
                RefreshResult::TransientFailure { retry_after },
            )
            .await?;
        self.repository
            .complete_failure_scheduled(
                claim,
                RefreshFailure {
                    error_code: error_code.to_owned(),
                    http_status,
                    retry_at: Some(schedule.next_at()),
                },
                schedule,
            )
            .await
            .map_err(FeedServiceError::RefreshRepository)?;
        self.load_refresh(&claim.run_id).await
    }

    async fn schedule(
        &self,
        previous_failures: i64,
        result: RefreshResult,
    ) -> Result<super::ScheduleOutcome, FeedServiceError> {
        let now = self
            .repository
            .database_now()
            .await
            .map_err(map_subscription_error)?;
        self.schedule
            .lock()
            .await
            .after_result(now, previous_failures, result)
            .map_err(FeedServiceError::Schedule)
    }

    async fn load_refresh(&self, run_id: &str) -> Result<RefreshDto, FeedServiceError> {
        self.repository
            .load_refresh_dto(run_id)
            .await
            .map_err(map_subscription_error)
    }
}

#[derive(thiserror::Error)]
pub enum FeedServiceError {
    #[error("user identifier is invalid")]
    InvalidUserId,
    #[error("subscription identifier is invalid")]
    InvalidSubscriptionId,
    #[error("feed URL is invalid")]
    Url(#[source] FeedUrlError),
    #[error("feed URL hash collision detected")]
    FeedUrlHashCollision,
    #[error("subscription is not authorized")]
    Unauthorized,
    #[error("refresh run is temporarily unavailable")]
    RunUnavailable,
    #[error("refresh run does not belong to the requested feed")]
    RunMismatch,
    #[error("stored feed data is corrupt")]
    CorruptFeed,
    #[error("feed is disabled")]
    FeedDisabled,
    #[error("refresh repository operation failed")]
    RefreshRepository(#[source] RefreshRepositoryError),
    #[error("entry repository operation failed")]
    EntryRepository(#[source] RepositoryError),
    #[error("refresh scheduling failed")]
    Schedule(#[source] ScheduleError),
}

impl fmt::Debug for FeedServiceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidUserId => "FeedServiceError::InvalidUserId",
            Self::InvalidSubscriptionId => "FeedServiceError::InvalidSubscriptionId",
            Self::Url(_) => "FeedServiceError::Url([REDACTED])",
            Self::FeedUrlHashCollision => "FeedServiceError::FeedUrlHashCollision",
            Self::Unauthorized => "FeedServiceError::Unauthorized",
            Self::RunUnavailable => "FeedServiceError::RunUnavailable",
            Self::RunMismatch => "FeedServiceError::RunMismatch",
            Self::CorruptFeed => "FeedServiceError::CorruptFeed",
            Self::FeedDisabled => "FeedServiceError::FeedDisabled",
            Self::RefreshRepository(_) => "FeedServiceError::RefreshRepository([REDACTED])",
            Self::EntryRepository(_) => "FeedServiceError::EntryRepository([REDACTED])",
            Self::Schedule(_) => "FeedServiceError::Schedule([REDACTED])",
        })
    }
}

fn map_subscription_error(error: SubscriptionRepositoryError) -> FeedServiceError {
    match error {
        SubscriptionRepositoryError::UserNotFound => FeedServiceError::Unauthorized,
        SubscriptionRepositoryError::FeedUrlHashCollision => FeedServiceError::FeedUrlHashCollision,
        SubscriptionRepositoryError::InvalidRequest => FeedServiceError::CorruptFeed,
        SubscriptionRepositoryError::Database(error) => {
            FeedServiceError::RefreshRepository(RefreshRepositoryError::Database(error))
        }
        SubscriptionRepositoryError::CorruptData | SubscriptionRepositoryError::RunConflict => {
            FeedServiceError::CorruptFeed
        }
    }
}

fn validate_uuid(value: &str) -> Result<(), ()> {
    let parsed = Uuid::parse_str(value).map_err(|_| ())?;
    (parsed.to_string() == value).then_some(()).ok_or(())
}

fn is_terminal(status: RefreshStatus) -> bool {
    matches!(
        status,
        RefreshStatus::Success
            | RefreshStatus::NotModified
            | RefreshStatus::Partial
            | RefreshStatus::Error
            | RefreshStatus::LeaseLost
            | RefreshStatus::Cancelled
    )
}

fn operational_persist_error(error: &RefreshRepositoryError) -> bool {
    matches!(
        error,
        RefreshRepositoryError::Content(_)
            | RefreshRepositoryError::InvalidContent
            | RefreshRepositoryError::IdentityHashCollision
            | RefreshRepositoryError::CountOverflow
    )
}

fn fetch_error_code(_error: &FeedFetchError) -> &'static str {
    "FETCH_FAILED"
}

struct SystemJitter;

impl JitterSource for SystemJitter {
    fn sample_inclusive_us(&mut self, upper_bound_us: u64) -> u64 {
        let mut rng = OsRng;
        rng.next_u64() % (upper_bound_us + 1)
    }
}
