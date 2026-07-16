use axum::{Json, Router, extract::FromRef, routing::get};
use serde::Serialize;
use std::sync::Arc;
use tokio::sync::Semaphore;

use crate::{
    api::{self, AccountThrottle, RateLimiter},
    auth::SessionService,
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
}

impl AppState {
    #[must_use]
    pub fn new(setup: SetupService) -> Self {
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
