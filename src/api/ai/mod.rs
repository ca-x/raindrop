mod config;
mod content;
mod providers;

use axum::Router;

use crate::app::AppState;

pub(super) fn router() -> Router<AppState> {
    Router::new()
        .merge(providers::router())
        .merge(config::router())
        .merge(content::router())
}
