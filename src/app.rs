use axum::{Json, Router, routing::get};
use serde::Serialize;

#[derive(Clone)]
pub struct AppState {
    pub version: &'static str,
}

impl AppState {
    #[must_use]
    pub const fn for_test() -> Self {
        Self {
            version: env!("CARGO_PKG_VERSION"),
        }
    }
}

#[derive(Serialize)]
struct HealthResponse {
    status: &'static str,
}

pub fn build_router(state: AppState) -> Router {
    Router::new()
        .route("/api/v1/health/live", get(live_health))
        .with_state(state)
}

async fn live_health() -> Json<HealthResponse> {
    Json(HealthResponse { status: "ok" })
}
