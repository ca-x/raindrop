use axum::{
    body::Body,
    http::{Request, header},
};
use http_body_util::BodyExt;
use raindrop::app::{AppState, build_router};
use tower::ServiceExt;

#[tokio::test]
async fn live_health_returns_ok() {
    let response = build_router(AppState::for_test())
        .oneshot(
            Request::builder()
                .uri("/api/v1/health/live")
                .body(Body::empty())
                .expect("request should be valid"),
        )
        .await
        .expect("router should respond");

    assert_eq!(response.status(), 200);
    assert!(response.headers().get(header::CACHE_CONTROL).is_none());
    assert!(response.headers().get(header::PRAGMA).is_none());
    let body = response
        .into_body()
        .collect()
        .await
        .expect("body should be readable")
        .to_bytes();
    assert_eq!(&body[..], br#"{"status":"ok"}"#);
}
