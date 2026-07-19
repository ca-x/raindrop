use std::{collections::HashMap, collections::VecDeque, sync::Arc, time::Duration};

use tokio::sync::{Mutex, OwnedSemaphorePermit, Semaphore};
use tokio::time::Instant;

use crate::content::provider::ProviderPolicy;

const REQUEST_WINDOW: Duration = Duration::from_secs(60);

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct AdmissionKey {
    provider_id: String,
    revision: u64,
}

#[derive(Default)]
pub(super) struct ProviderAdmissionController {
    states: Mutex<HashMap<AdmissionKey, Arc<ProviderAdmissionState>>>,
}

impl ProviderAdmissionController {
    pub(super) async fn acquire(
        &self,
        provider_id: &str,
        revision: u64,
        policy: ProviderPolicy,
    ) -> Result<ProviderAdmissionGuard, ProviderAdmissionError> {
        let key = AdmissionKey {
            provider_id: provider_id.to_owned(),
            revision,
        };
        let state = {
            let mut states = self.states.lock().await;
            states.retain(|existing, state| {
                existing.provider_id != provider_id
                    || existing.revision == revision
                    || Arc::strong_count(state) > 1
            });
            states
                .entry(key)
                .or_insert_with(|| Arc::new(ProviderAdmissionState::new(policy)))
                .clone()
        };
        let permit = state
            .concurrency
            .clone()
            .try_acquire_owned()
            .map_err(|_| ProviderAdmissionError::ConcurrencyLimited)?;
        if let Err(error) = state.reserve_request(Instant::now()).await {
            drop(permit);
            return Err(error);
        }
        Ok(ProviderAdmissionGuard { _permit: permit })
    }
}

struct ProviderAdmissionState {
    concurrency: Arc<Semaphore>,
    requests_per_minute: Option<u32>,
    request_window: Mutex<VecDeque<Instant>>,
}

impl ProviderAdmissionState {
    fn new(policy: ProviderPolicy) -> Self {
        Self {
            concurrency: Arc::new(Semaphore::new(usize::from(policy.max_concurrency))),
            requests_per_minute: policy.requests_per_minute,
            request_window: Mutex::new(VecDeque::new()),
        }
    }

    async fn reserve_request(&self, now: Instant) -> Result<(), ProviderAdmissionError> {
        let Some(limit) = self.requests_per_minute else {
            return Ok(());
        };
        let mut window = self.request_window.lock().await;
        while window
            .front()
            .is_some_and(|started| now.saturating_duration_since(*started) >= REQUEST_WINDOW)
        {
            window.pop_front();
        }
        let limit = usize::try_from(limit).unwrap_or(usize::MAX);
        if window.len() >= limit {
            let retry_after = window.front().map(|started| {
                REQUEST_WINDOW.saturating_sub(now.saturating_duration_since(*started))
            });
            return Err(ProviderAdmissionError::RateLimited { retry_after });
        }
        window.push_back(now);
        Ok(())
    }
}

pub(super) struct ProviderAdmissionGuard {
    _permit: OwnedSemaphorePermit,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ProviderAdmissionError {
    ConcurrencyLimited,
    RateLimited { retry_after: Option<Duration> },
}

#[cfg(test)]
mod tests {
    use super::*;

    fn policy(max_concurrency: u16, requests_per_minute: Option<u32>) -> ProviderPolicy {
        ProviderPolicy {
            max_concurrency,
            requests_per_minute,
            max_input_tokens_per_request: 1024,
            max_output_tokens_per_request: 1024,
            input_cost_micros_per_million_tokens: None,
            output_cost_micros_per_million_tokens: None,
            max_cost_micros_per_request: None,
        }
    }

    #[tokio::test]
    async fn admission_rejects_concurrency_without_queueing() {
        let admission = ProviderAdmissionController::default();
        let first = admission
            .acquire("provider", 1, policy(1, None))
            .await
            .expect("first permit");
        assert!(matches!(
            admission.acquire("provider", 1, policy(1, None)).await,
            Err(ProviderAdmissionError::ConcurrencyLimited),
        ));
        drop(first);
        admission
            .acquire("provider", 1, policy(1, None))
            .await
            .expect("released permit should be reusable");
    }

    #[tokio::test]
    async fn admission_reserves_a_sliding_minute_request() {
        let state = ProviderAdmissionState::new(policy(1, Some(1)));
        let start = Instant::now();
        state.reserve_request(start).await.expect("first request");
        assert!(matches!(
            state.reserve_request(start + Duration::from_secs(1)).await,
            Err(ProviderAdmissionError::RateLimited {
                retry_after: Some(duration)
            }) if duration == Duration::from_secs(59)
        ));
        state
            .reserve_request(start + REQUEST_WINDOW)
            .await
            .expect("exact window expiry should admit");
    }

    #[tokio::test]
    async fn admission_revision_changes_create_a_new_policy_state() {
        let admission = ProviderAdmissionController::default();
        let first = admission
            .acquire("provider", 1, policy(1, None))
            .await
            .expect("revision one permit");
        admission
            .acquire("provider", 2, policy(1, None))
            .await
            .expect("revision two should use a distinct state");
        drop(first);
    }
}
