use std::{
    collections::{HashMap, VecDeque},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

#[derive(Clone)]
pub(crate) struct RateLimiter {
    max_attempts: usize,
    window: Duration,
    max_keys: usize,
    attempts: Arc<Mutex<HashMap<String, VecDeque<Instant>>>>,
}

impl RateLimiter {
    pub(crate) fn new(max_attempts: usize, window: Duration, max_keys: usize) -> Self {
        Self {
            max_attempts,
            window,
            max_keys,
            attempts: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub(crate) fn check(&self, key: &str) -> bool {
        let now = Instant::now();
        let cutoff = now.checked_sub(self.window).unwrap_or(now);
        let mut attempts = self
            .attempts
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        attempts.retain(|_, values| {
            while values.front().is_some_and(|value| *value <= cutoff) {
                values.pop_front();
            }
            !values.is_empty()
        });
        if !attempts.contains_key(key) && attempts.len() >= self.max_keys {
            return false;
        }
        let values = attempts.entry(key.to_owned()).or_default();
        if values.len() >= self.max_attempts {
            return false;
        }
        values.push_back(now);
        true
    }
}
