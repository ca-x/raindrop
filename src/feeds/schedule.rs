use std::str;
use std::time::UNIX_EPOCH;

use http::HeaderValue;
use time::{Duration, OffsetDateTime, PrimitiveDateTime, UtcOffset};

use super::{RetryAfterError, ScheduleError};

const SUCCESS_DELAY: Duration = Duration::minutes(5);
const BASE_BACKOFF_US: u64 = 300_000_000;
const MAX_DELAY_US: u64 = 14_400_000_000;
const MAX_DELAY: Duration = Duration::hours(4);

pub trait JitterSource {
    fn sample_inclusive_us(&mut self, upper_bound_us: u64) -> u64;
}

impl<T> JitterSource for Box<T>
where
    T: JitterSource + ?Sized,
{
    fn sample_inclusive_us(&mut self, upper_bound_us: u64) -> u64 {
        (**self).sample_inclusive_us(upper_bound_us)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RefreshResult {
    Success,
    NotModified,
    TransientFailure { retry_after: Option<RetryAfter> },
}

pub struct RefreshSchedule<J> {
    jitter: J,
}

impl<J> RefreshSchedule<J>
where
    J: JitterSource,
{
    #[must_use]
    pub const fn new(jitter: J) -> Self {
        Self { jitter }
    }

    pub fn after_result(
        &mut self,
        now: OffsetDateTime,
        previous_failures: i64,
        result: RefreshResult,
    ) -> Result<ScheduleOutcome, ScheduleError> {
        if previous_failures < 0 {
            return Err(ScheduleError::NegativeFailureCount);
        }
        let now = now.to_offset(UtcOffset::UTC);

        match result {
            RefreshResult::Success | RefreshResult::NotModified => {
                let next_at = now
                    .checked_add(SUCCESS_DELAY)
                    .ok_or(ScheduleError::TimeOverflow)?;
                Ok(ScheduleOutcome {
                    next_at,
                    consecutive_failures: 0,
                    retry_after_at: None,
                })
            }
            RefreshResult::TransientFailure { retry_after } => {
                let consecutive_failures = previous_failures.saturating_add(1);
                let upper_bound_us = backoff_upper_bound_us(consecutive_failures);
                let sampled_us = self.jitter.sample_inclusive_us(upper_bound_us);
                if sampled_us > upper_bound_us {
                    return Err(ScheduleError::InvalidJitter);
                }

                let jitter_delay = Duration::microseconds(sampled_us as i64);
                let retry_after_at = retry_after.map(RetryAfter::at);
                let retry_after_delay = retry_after_at.map_or(Duration::ZERO, |retry_at| {
                    if retry_at > now {
                        retry_at - now
                    } else {
                        Duration::ZERO
                    }
                });
                let delay = jitter_delay.max(retry_after_delay).min(MAX_DELAY);
                let next_at = now.checked_add(delay).ok_or(ScheduleError::TimeOverflow)?;

                Ok(ScheduleOutcome {
                    next_at,
                    consecutive_failures,
                    retry_after_at,
                })
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ScheduleOutcome {
    next_at: OffsetDateTime,
    consecutive_failures: i64,
    retry_after_at: Option<OffsetDateTime>,
}

impl ScheduleOutcome {
    #[must_use]
    pub const fn next_at(self) -> OffsetDateTime {
        self.next_at
    }

    #[must_use]
    pub const fn consecutive_failures(self) -> i64 {
        self.consecutive_failures
    }

    #[must_use]
    pub const fn retry_after_at(self) -> Option<OffsetDateTime> {
        self.retry_after_at
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RetryAfter {
    at: OffsetDateTime,
}

impl RetryAfter {
    pub fn parse(raw: &HeaderValue, received_at: OffsetDateTime) -> Result<Self, RetryAfterError> {
        let bytes = trim_optional_whitespace(raw.as_bytes());
        if bytes.is_empty() {
            return Err(RetryAfterError::Empty);
        }
        let received_at = received_at.to_offset(UtcOffset::UTC);

        let at = if bytes.iter().all(u8::is_ascii_digit) {
            let digits = str::from_utf8(bytes).map_err(|_| RetryAfterError::Invalid)?;
            let seconds = digits
                .parse::<u64>()
                .map_err(|_| RetryAfterError::DeltaOverflow)?;
            if seconds > i64::MAX as u64 {
                maximum_utc()
            } else {
                received_at
                    .checked_add(Duration::seconds(seconds as i64))
                    .unwrap_or_else(maximum_utc)
            }
        } else {
            let date = str::from_utf8(bytes).map_err(|_| RetryAfterError::Invalid)?;
            let parsed = httpdate::parse_http_date(date).map_err(|_| RetryAfterError::Invalid)?;
            let since_epoch = parsed
                .duration_since(UNIX_EPOCH)
                .map_err(|_| RetryAfterError::Invalid)?;
            let seconds =
                i64::try_from(since_epoch.as_secs()).map_err(|_| RetryAfterError::Invalid)?;
            OffsetDateTime::UNIX_EPOCH
                .checked_add(Duration::seconds(seconds))
                .ok_or(RetryAfterError::Invalid)?
        };

        Ok(Self { at })
    }

    #[must_use]
    pub const fn at(self) -> OffsetDateTime {
        self.at
    }
}

fn backoff_upper_bound_us(consecutive_failures: i64) -> u64 {
    if consecutive_failures >= 7 {
        MAX_DELAY_US
    } else {
        let shift = u32::try_from(consecutive_failures - 1)
            .expect("transient failure count is at least one");
        BASE_BACKOFF_US * (1_u64 << shift)
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
