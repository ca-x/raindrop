use std::{
    collections::{HashMap, VecDeque},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use time::OffsetDateTime;

const USER_MUTATION_LIMIT: u32 = 30;
const USER_MUTATION_WINDOW: Duration = Duration::from_secs(15 * 60);
const USER_MUTATION_MAX_KEYS: usize = 10_000;

#[derive(Clone)]
pub(crate) struct RateLimiter {
    max_attempts: usize,
    window: Duration,
    attempts: Arc<Mutex<VecDeque<Instant>>>,
}

impl RateLimiter {
    pub(crate) fn new(max_attempts: usize, window: Duration) -> Self {
        Self {
            max_attempts,
            window,
            attempts: Arc::new(Mutex::new(VecDeque::new())),
        }
    }

    pub(crate) fn check(&self) -> bool {
        self.check_at(Instant::now())
    }

    fn check_at(&self, now: Instant) -> bool {
        let cutoff = now.checked_sub(self.window).unwrap_or(now);
        let mut attempts = self
            .attempts
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        while attempts.front().is_some_and(|value| *value <= cutoff) {
            attempts.pop_front();
        }
        if attempts.len() >= self.max_attempts {
            return false;
        }
        attempts.push_back(now);
        true
    }
}

#[derive(Clone)]
pub(crate) struct AccountThrottle {
    window: Duration,
    base_delay: Duration,
    max_delay: Duration,
    max_keys: usize,
    failures: Arc<Mutex<HashMap<String, FailureHistory>>>,
}

#[derive(Clone, Copy)]
struct FailureHistory {
    count: u32,
    updated_at: Instant,
}

impl AccountThrottle {
    pub(crate) fn new(
        window: Duration,
        base_delay: Duration,
        max_delay: Duration,
        max_keys: usize,
    ) -> Self {
        Self {
            window,
            base_delay,
            max_delay,
            max_keys,
            failures: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub(crate) fn delay(&self, key: &str) -> Duration {
        let now = Instant::now();
        let mut failures = self
            .failures
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        self.cleanup(&mut failures, now);
        let Some(history) = failures.get(key) else {
            return Duration::ZERO;
        };
        let multiplier = 1_u32
            .checked_shl(history.count.saturating_sub(1).min(31))
            .unwrap_or(u32::MAX);
        self.base_delay
            .checked_mul(multiplier)
            .unwrap_or(self.max_delay)
            .min(self.max_delay)
    }

    pub(crate) fn record_failure(&self, key: &str) {
        let now = Instant::now();
        let mut failures = self
            .failures
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        self.cleanup(&mut failures, now);
        if !failures.contains_key(key)
            && failures.len() >= self.max_keys
            && let Some(oldest) = failures
                .iter()
                .min_by_key(|(_, history)| history.updated_at)
                .map(|(key, _)| key.clone())
        {
            failures.remove(&oldest);
        }
        failures
            .entry(key.to_owned())
            .and_modify(|history| {
                history.count = history.count.saturating_add(1);
                history.updated_at = now;
            })
            .or_insert(FailureHistory {
                count: 1,
                updated_at: now,
            });
    }

    pub(crate) fn clear(&self, key: &str) {
        self.failures
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(key);
    }

    fn cleanup(&self, failures: &mut HashMap<String, FailureHistory>, now: Instant) {
        let cutoff = now.checked_sub(self.window).unwrap_or(now);
        failures.retain(|_, history| history.updated_at > cutoff);
    }
}

#[derive(Clone)]
pub struct UserMutationLimiter {
    inner: Arc<Mutex<UserMutationLimiterState>>,
    limit: u32,
    window: Duration,
    max_keys: usize,
}

struct UserMutationLimiterState {
    users: HashMap<String, UserMutationBucket>,
}

struct UserMutationBucket {
    attempts: VecDeque<Instant>,
    updated_at: Instant,
}

#[derive(Debug)]
pub struct RateLimitRejection {
    pub retry_at: OffsetDateTime,
    pub retry_after_seconds: u64,
}

impl UserMutationLimiter {
    #[must_use]
    pub fn new() -> Self {
        Self::with_limits(
            USER_MUTATION_LIMIT,
            USER_MUTATION_WINDOW,
            USER_MUTATION_MAX_KEYS,
        )
    }

    fn with_limits(limit: u32, window: Duration, max_keys: usize) -> Self {
        Self {
            inner: Arc::new(Mutex::new(UserMutationLimiterState {
                users: HashMap::new(),
            })),
            limit,
            window,
            max_keys,
        }
    }

    pub fn check(&self, user_id: &str) -> Result<(), RateLimitRejection> {
        self.check_at(user_id, Instant::now(), OffsetDateTime::now_utc())
    }

    fn check_at(
        &self,
        user_id: &str,
        now: Instant,
        wall_clock: OffsetDateTime,
    ) -> Result<(), RateLimitRejection> {
        let mut state = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.users.retain(|_, bucket| {
            while bucket
                .attempts
                .front()
                .is_some_and(|attempt| now.saturating_duration_since(*attempt) >= self.window)
            {
                bucket.attempts.pop_front();
            }
            !bucket.attempts.is_empty()
        });

        if let Some(bucket) = state.users.get_mut(user_id) {
            if bucket.attempts.len() >= self.limit as usize {
                let oldest = *bucket
                    .attempts
                    .front()
                    .expect("a full mutation bucket must contain an attempt");
                return Err(rate_limit_rejection(now, wall_clock, oldest + self.window));
            }
            bucket.attempts.push_back(now);
            bucket.updated_at = now;
            return Ok(());
        }

        if state.users.len() >= self.max_keys
            && let Some(oldest_user) = state
                .users
                .iter()
                .min_by_key(|(_, bucket)| bucket.updated_at)
                .map(|(user_id, _)| user_id.clone())
        {
            state.users.remove(&oldest_user);
        }
        state.users.insert(
            user_id.to_owned(),
            UserMutationBucket {
                attempts: VecDeque::from([now]),
                updated_at: now,
            },
        );
        Ok(())
    }
}

impl Default for UserMutationLimiter {
    fn default() -> Self {
        Self::new()
    }
}

fn rate_limit_rejection(
    now: Instant,
    wall_clock: OffsetDateTime,
    retry_deadline: Instant,
) -> RateLimitRejection {
    let remaining = retry_deadline.saturating_duration_since(now);
    let retry_after_seconds = remaining
        .as_secs()
        .saturating_add(u64::from(remaining.subsec_nanos() > 0))
        .max(1);
    RateLimitRejection {
        retry_at: wall_clock
            + time::Duration::try_from(remaining)
                .expect("the bounded mutation window must fit time::Duration"),
        retry_after_seconds,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use time::macros::datetime;

    #[test]
    fn global_fuse_rejects_after_the_exact_threshold_and_expires_by_window() {
        let window = Duration::from_secs(60);
        let limiter = RateLimiter::new(2, window);
        let start = Instant::now();

        assert!(limiter.check_at(start));
        assert!(limiter.check_at(start));
        assert!(!limiter.check_at(start));
        assert!(limiter.check_at(start + window));
    }

    #[test]
    fn account_history_adds_only_bounded_delay_and_success_clears_it() {
        let throttle = AccountThrottle::new(
            Duration::from_secs(60),
            Duration::from_millis(5),
            Duration::from_millis(20),
            2,
        );

        assert_eq!(throttle.delay("reader"), Duration::ZERO);
        throttle.record_failure("reader");
        assert_eq!(throttle.delay("reader"), Duration::from_millis(5));
        throttle.record_failure("reader");
        throttle.record_failure("reader");
        throttle.record_failure("reader");
        assert_eq!(throttle.delay("reader"), Duration::from_millis(20));
        throttle.clear("reader");
        assert_eq!(throttle.delay("reader"), Duration::ZERO);

        throttle.record_failure("first");
        throttle.record_failure("second");
        throttle.record_failure("third");
        assert_eq!(
            throttle
                .failures
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .len(),
            2
        );
    }

    #[test]
    fn user_mutation_limiter_isolates_users_at_exact_threshold() {
        let limiter = UserMutationLimiter::with_limits(2, Duration::from_secs(60), 10);
        let started_at = Instant::now();
        let wall_clock = datetime!(2026-07-17 12:00 UTC);

        assert!(limiter.check_at("user-a", started_at, wall_clock).is_ok());
        assert!(limiter.check_at("user-a", started_at, wall_clock).is_ok());
        assert!(limiter.check_at("user-a", started_at, wall_clock).is_err());
        assert!(limiter.check_at("user-b", started_at, wall_clock).is_ok());
        assert!(limiter.check_at("user-b", started_at, wall_clock).is_ok());
        assert!(limiter.check_at("user-b", started_at, wall_clock).is_err());
    }

    #[test]
    fn user_mutation_limiter_returns_ceil_retry_after_at_least_one() {
        let limiter = UserMutationLimiter::with_limits(1, Duration::from_millis(1_500), 10);
        let started_at = Instant::now();
        let wall_clock = datetime!(2026-07-17 12:00 UTC);
        limiter
            .check_at("user", started_at, wall_clock)
            .expect("first mutation should be admitted");

        let rejection = limiter
            .check_at(
                "user",
                started_at + Duration::from_millis(499),
                wall_clock + time::Duration::milliseconds(499),
            )
            .expect_err("mutation inside the window should be rejected");
        assert_eq!(rejection.retry_after_seconds, 2);
        assert_eq!(
            rejection.retry_at,
            wall_clock + time::Duration::milliseconds(1_500)
        );

        let rejection = limiter
            .check_at(
                "user",
                started_at + Duration::from_millis(1_499),
                wall_clock + time::Duration::milliseconds(1_499),
            )
            .expect_err("a positive sub-second wait should still reject");
        assert_eq!(rejection.retry_after_seconds, 1);
    }

    #[test]
    fn user_mutation_limiter_does_not_expire_when_window_predates_instant_origin() {
        let limiter = UserMutationLimiter::with_limits(2, Duration::MAX, 10);
        let started_at = Instant::now();
        let wall_clock = datetime!(2026-07-17 12:00 UTC);

        limiter
            .check_at("user", started_at, wall_clock)
            .expect("first mutation should be admitted");
        limiter
            .check_at("user", started_at, wall_clock)
            .expect("same-instant mutation must not expire the first attempt");

        let state = limiter
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        assert_eq!(state.users["user"].attempts.len(), 2);
    }

    #[test]
    fn user_mutation_limiter_expires_and_bounds_key_storage() {
        let limiter = UserMutationLimiter::with_limits(1, Duration::from_secs(60), 2);
        let started_at = Instant::now();
        let wall_clock = datetime!(2026-07-17 12:00 UTC);

        limiter
            .check_at("expired", started_at, wall_clock)
            .expect("first key should be admitted");
        limiter
            .check_at(
                "retained",
                started_at + Duration::from_secs(30),
                wall_clock + time::Duration::seconds(30),
            )
            .expect("second key should be admitted");
        limiter
            .check_at(
                "new",
                started_at + Duration::from_secs(60),
                wall_clock + time::Duration::seconds(60),
            )
            .expect("expired key should be cleaned before capacity eviction");

        let state = limiter
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        assert_eq!(state.users.len(), 2);
        assert!(!state.users.contains_key("expired"));
        assert!(state.users.contains_key("retained"));
        assert!(state.users.contains_key("new"));
        drop(state);

        limiter
            .check_at(
                "overflow",
                started_at + Duration::from_secs(60),
                wall_clock + time::Duration::seconds(60),
            )
            .expect("a new key should evict the oldest active key at capacity");
        let state = limiter
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        assert_eq!(state.users.len(), 2);
        assert!(!state.users.contains_key("retained"));
        assert!(state.users.contains_key("new"));
        assert!(state.users.contains_key("overflow"));
    }
}
