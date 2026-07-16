use std::{
    collections::{HashMap, VecDeque},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

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

#[cfg(test)]
mod tests {
    use super::*;

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
}
