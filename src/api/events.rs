use std::time::Duration;

use axum::{
    Router,
    extract::State,
    http::{HeaderName, HeaderValue},
    response::{
        IntoResponse, Sse,
        sse::{Event, KeepAlive},
    },
    routing::get,
};
use futures_util::stream;

use crate::{
    app::AppState,
    auth::CurrentUser,
    realtime::{ReaderEvent, ReaderEventReceive, ReaderEventSubscription},
};

use super::routes::sensitive_cache_headers;

const X_ACCEL_BUFFERING: HeaderName = HeaderName::from_static("x-accel-buffering");

pub(super) fn router() -> Router<AppState> {
    Router::new()
        .route("/api/v1/events", get(reader_events))
        .layer(axum::middleware::map_response(sensitive_cache_headers))
}

async fn reader_events(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
) -> impl IntoResponse {
    let subscription = state.reader_events.subscribe(&user.id);
    let stream = stream::unfold(
        ReaderEventStreamState {
            initial_sync: true,
            subscription,
        },
        |mut state| async move {
            let event = if state.initial_sync {
                state.initial_sync = false;
                ReaderEvent::sync_required()
            } else {
                match state.subscription.recv().await {
                    ReaderEventReceive::Event(event) => event,
                    ReaderEventReceive::Lagged => ReaderEvent::sync_required(),
                    ReaderEventReceive::Closed => return None,
                }
            };
            Some((Event::default().event("reader").json_data(event), state))
        },
    );
    (
        [(X_ACCEL_BUFFERING, HeaderValue::from_static("no"))],
        Sse::new(stream).keep_alive(
            KeepAlive::new()
                .interval(Duration::from_secs(15))
                .text("keep-alive"),
        ),
    )
}

struct ReaderEventStreamState {
    initial_sync: bool,
    subscription: ReaderEventSubscription,
}
