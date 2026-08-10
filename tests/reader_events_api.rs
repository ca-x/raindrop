#[allow(dead_code)]
mod support;

use std::time::Duration;

use axum::{
    body::Body,
    http::{
        Method, Request, StatusCode,
        header::{CACHE_CONTROL, CONTENT_TYPE, COOKIE, HOST, ORIGIN, PRAGMA},
    },
};
use http_body_util::BodyExt;
use raindrop::{
    app::{AppState, build_router},
    auth::build_session_cookie,
    db::{DatabaseConfig, connect, migrate},
    setup::SetupService,
};
use secrecy::{ExposeSecret, SecretString};
use serde_json::json;
use tower::ServiceExt;

use support::database::{USER_A_ID, insert_user};

#[tokio::test]
async fn authenticated_event_stream_starts_with_sync_and_receives_user_mutations() {
    let data = tempfile::tempdir().expect("temporary event API directory");
    let database_url = format!(
        "sqlite://{}?mode=rwc",
        data.path().join("reader-events.db").display()
    );
    let database = connect(&DatabaseConfig::new(SecretString::from(database_url)))
        .await
        .expect("event API database should connect");
    migrate(&database)
        .await
        .expect("event API database should migrate");
    insert_user(&database, USER_A_ID, "event-reader").await;
    let setup = SetupService::ready(data.path(), None, database);
    let session = setup
        .sessions()
        .create(USER_A_ID)
        .await
        .expect("event API session should create");
    let cookie_header = build_session_cookie(&session, false).to_string();
    let cookie = cookie_header
        .split(';')
        .next()
        .expect("session cookie should contain a pair")
        .to_owned();
    let csrf = session.csrf_token.expose_secret().to_owned();
    let app = build_router(AppState::new(setup));

    let unauthenticated = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/events")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("unauthenticated event request should complete");
    assert_eq!(unauthenticated.status(), StatusCode::UNAUTHORIZED);

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/events")
                .header(COOKIE, &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("authenticated event request should complete");
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers().get(CONTENT_TYPE).unwrap(),
        "text/event-stream"
    );
    assert_eq!(response.headers().get(CACHE_CONTROL).unwrap(), "no-store");
    assert_eq!(response.headers().get(PRAGMA).unwrap(), "no-cache");
    assert_eq!(response.headers().get("x-accel-buffering").unwrap(), "no");
    let mut body = response.into_body();
    let initial = next_data_frame(&mut body).await;
    assert!(initial.contains("event: reader"));
    assert!(initial.contains(r#""kind":"SYNC_REQUIRED""#));

    let category_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/api/v1/categories")
                .header(COOKIE, &cookie)
                .header("x-csrf-token", csrf)
                .header(ORIGIN, "http://events.test")
                .header(HOST, "events.test")
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(json!({ "title": "Realtime" }).to_string()))
                .unwrap(),
        )
        .await
        .expect("category mutation should complete");
    assert_eq!(category_response.status(), StatusCode::CREATED);

    let changed = next_data_frame(&mut body).await;
    assert!(changed.contains(r#""kind":"SUBSCRIPTIONS_CHANGED""#));
}

async fn next_data_frame(body: &mut Body) -> String {
    let frame = tokio::time::timeout(Duration::from_secs(2), body.frame())
        .await
        .expect("event frame should arrive promptly")
        .expect("event stream should remain open")
        .expect("event frame should be valid");
    String::from_utf8(
        frame
            .into_data()
            .expect("event stream should produce a data frame")
            .to_vec(),
    )
    .expect("event stream frame should be UTF-8")
}
