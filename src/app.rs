use axum::{Json, Router, extract::FromRef, routing::get};
use serde::Serialize;
use std::sync::Arc;
use tokio::sync::Semaphore;

use crate::{
    api::{self, AccountThrottle, RateLimiter, UserMutationLimiter},
    auth::SessionService,
    feeds::{FeedRuntime, FeedRuntimeHandle, FeedServiceError, HttpFeedTransport},
    setup::SetupService,
    web,
};

#[derive(Clone)]
pub struct AppState {
    pub version: &'static str,
    pub(crate) setup: SetupService,
    pub(crate) login_limiter: RateLimiter,
    pub(crate) login_authentication_semaphore: Arc<Semaphore>,
    pub(crate) login_account_throttle: AccountThrottle,
    pub(crate) setup_limiter: RateLimiter,
    pub feed_runtime: FeedRuntimeHandle,
    pub subscription_mutation_limiter: UserMutationLimiter,
}

impl AppState {
    #[must_use]
    pub fn new(setup: SetupService) -> Self {
        let (_runtime, handle) = FeedRuntime::<HttpFeedTransport>::new(setup.clone(), |_| {
            Err(FeedServiceError::CorruptFeed)
        });
        Self::with_feed_runtime(setup, handle)
    }

    #[must_use]
    pub fn with_feed_runtime(setup: SetupService, feed_runtime: FeedRuntimeHandle) -> Self {
        Self {
            version: env!("CARGO_PKG_VERSION"),
            setup,
            login_limiter: RateLimiter::new(4_096, std::time::Duration::from_secs(15 * 60)),
            login_authentication_semaphore: Arc::new(Semaphore::new(4)),
            login_account_throttle: AccountThrottle::new(
                std::time::Duration::from_secs(15 * 60),
                std::time::Duration::from_millis(5),
                std::time::Duration::from_millis(100),
                10_000,
            ),
            setup_limiter: RateLimiter::new(30, std::time::Duration::from_secs(15 * 60)),
            feed_runtime,
            subscription_mutation_limiter: UserMutationLimiter::new(),
        }
    }

    #[must_use]
    pub fn for_test() -> Self {
        Self::new(SetupService::required(
            std::path::Path::new("."),
            secrecy::SecretString::from("health-test-setup-token".to_owned()),
            None,
        ))
    }
}

impl FromRef<AppState> for SessionService {
    fn from_ref(state: &AppState) -> Self {
        state.setup.sessions()
    }
}

#[derive(Serialize)]
struct HealthResponse {
    status: &'static str,
}

pub fn build_router(state: AppState) -> Router {
    Router::new()
        .route("/api/v1/health/live", get(live_health))
        .merge(api::router())
        .fallback(web::serve)
        .with_state(state)
}

async fn live_health() -> Json<HealthResponse> {
    Json(HealthResponse { status: "ok" })
}

#[cfg(test)]
mod tests {
    use secrecy::SecretString;

    use crate::feeds::{FeedRuntime, FeedServiceError, HttpFeedTransport};

    use super::*;

    #[test]
    fn app_state_new_provides_inert_runtime_and_exact_mutation_limit() {
        let data = tempfile::tempdir().expect("temporary directory should be created");
        let state = AppState::new(SetupService::required(
            data.path(),
            SecretString::from("setup-token"),
            None,
        ));

        state.feed_runtime.notify();
        state.feed_runtime.shutdown();
        for _ in 0..30 {
            state
                .subscription_mutation_limiter
                .check("user")
                .expect("the exact production mutation budget should be admitted");
        }
        assert!(state.subscription_mutation_limiter.check("user").is_err());
    }

    #[tokio::test]
    async fn app_state_with_feed_runtime_preserves_the_production_handle() {
        let data = tempfile::tempdir().expect("temporary directory should be created");
        let setup = SetupService::required(data.path(), SecretString::from("setup-token"), None);
        let (runtime, handle) = FeedRuntime::<HttpFeedTransport>::new(setup.clone(), |_| {
            Err(FeedServiceError::CorruptFeed)
        });
        let state = AppState::with_feed_runtime(setup, handle);

        state.feed_runtime.shutdown();
        runtime
            .run()
            .await
            .expect("pre-start shutdown should stop the production runtime cleanly");
    }
}
