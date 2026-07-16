use axum::{
    body::Body,
    http::{Request, StatusCode, header},
};
use http_body_util::BodyExt;
use raindrop::app::{AppState, build_router};
use tower::ServiceExt;

#[cfg(not(debug_assertions))]
const CONTENT_SECURITY_POLICY: &str = "default-src 'self'; base-uri 'self'; connect-src 'self'; font-src 'self'; form-action 'self'; frame-ancestors 'none'; img-src 'self' data:; object-src 'none'; script-src 'self'; style-src 'self' 'unsafe-inline'";

#[tokio::test]
#[cfg(not(debug_assertions))]
async fn root_serves_the_embedded_application_with_security_headers() {
    let response = build_router(AppState::for_test())
        .oneshot(Request::get("/").body(Body::empty()).unwrap())
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers()[header::CONTENT_TYPE],
        "text/html; charset=utf-8"
    );
    assert_eq!(
        response.headers()[header::CACHE_CONTROL],
        "no-cache, no-store, must-revalidate"
    );
    assert_eq!(
        response.headers()[header::CONTENT_SECURITY_POLICY],
        CONTENT_SECURITY_POLICY
    );
    assert_eq!(
        response.headers()[header::X_CONTENT_TYPE_OPTIONS],
        "nosniff"
    );
    assert_eq!(response.headers()[header::REFERRER_POLICY], "no-referrer");
    assert_eq!(response.headers()[header::X_FRAME_OPTIONS], "DENY");

    let body = response.into_body().collect().await.unwrap().to_bytes();
    let html = std::str::from_utf8(&body).unwrap();
    assert!(html.contains("<div id=\"root\"></div>"));
    assert!(html.contains("/assets/"));
}

#[tokio::test]
#[cfg(not(debug_assertions))]
async fn hashed_javascript_asset_has_mime_and_immutable_cache_headers() {
    let router = build_router(AppState::for_test());
    let index = router
        .clone()
        .oneshot(Request::get("/").body(Body::empty()).unwrap())
        .await
        .unwrap()
        .into_body()
        .collect()
        .await
        .unwrap()
        .to_bytes();
    let html = std::str::from_utf8(&index).unwrap();
    let script_path = embedded_script_path(html);

    let response = router
        .oneshot(Request::get(script_path).body(Body::empty()).unwrap())
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers()[header::CONTENT_TYPE],
        "text/javascript; charset=utf-8"
    );
    assert_eq!(
        response.headers()[header::CACHE_CONTROL],
        "public, max-age=31536000, immutable"
    );
    assert_eq!(
        response.headers()[header::X_CONTENT_TYPE_OPTIONS],
        "nosniff"
    );
    assert!(
        !response
            .into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes()
            .is_empty()
    );
}

#[tokio::test]
#[cfg(not(debug_assertions))]
async fn unknown_non_api_route_falls_back_to_uncached_spa_html() {
    let response = build_router(AppState::for_test())
        .oneshot(
            Request::get("/library/entries/unread")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers()[header::CONTENT_TYPE],
        "text/html; charset=utf-8"
    );
    assert_eq!(
        response.headers()[header::CACHE_CONTROL],
        "no-cache, no-store, must-revalidate"
    );
    let body = response.into_body().collect().await.unwrap().to_bytes();
    assert!(
        std::str::from_utf8(&body)
            .unwrap()
            .contains("<div id=\"root\"></div>")
    );
}

#[tokio::test]
#[cfg(not(debug_assertions))]
async fn unknown_api_route_remains_a_json_not_found() {
    let response = build_router(AppState::for_test())
        .oneshot(
            Request::get("/api/v1/does-not-exist")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    assert_eq!(response.headers()[header::CONTENT_TYPE], "application/json");
    assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let body: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(body["error"]["code"], "NOT_FOUND");
    assert!(body["error"]["requestId"].is_string());
}

#[tokio::test]
#[cfg(not(debug_assertions))]
async fn unhashed_brand_asset_has_image_mime_without_immutable_caching() {
    let response = build_router(AppState::for_test())
        .oneshot(
            Request::get("/brand/raindrop-logo-32.png")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()[header::CONTENT_TYPE], "image/png");
    assert_eq!(
        response.headers()[header::CACHE_CONTROL],
        "public, max-age=3600"
    );
    let body = response.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(&body[..8], b"\x89PNG\r\n\x1a\n");
}

#[tokio::test]
#[cfg(not(debug_assertions))]
async fn asset_keys_reject_traversal_and_never_fall_back_to_spa_html() {
    for path in [
        "/assets/../index.html",
        "/assets/%2e%2e/index.html",
        "/assets/missing.js",
    ] {
        let response = build_router(AppState::for_test())
            .oneshot(Request::get(path).body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::NOT_FOUND, "{path}");
        assert_eq!(
            response.headers()[header::CONTENT_TYPE],
            "text/plain; charset=utf-8",
            "{path}"
        );
        let body = response.into_body().collect().await.unwrap().to_bytes();
        assert!(!body.windows(15).any(|window| window == b"<div id=\"root\""));
    }
}

#[tokio::test]
#[cfg(debug_assertions)]
async fn debug_router_points_to_vite_without_a_production_bundle() {
    let response = build_router(AppState::for_test())
        .oneshot(Request::get("/reader").body(Body::empty()).unwrap())
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers()[header::CONTENT_TYPE],
        "text/html; charset=utf-8"
    );
    assert_eq!(
        response.headers()[header::CACHE_CONTROL],
        "no-cache, no-store, must-revalidate"
    );
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let html = std::str::from_utf8(&body).unwrap();
    assert!(html.contains("npm --prefix web run dev"));
    assert!(html.contains("http://localhost:5173"));
}

#[cfg(not(debug_assertions))]
fn embedded_script_path(html: &str) -> &str {
    let path = html
        .split_once("<script type=\"module\" crossorigin src=\"")
        .expect("Vite index should reference a module script")
        .1;
    path.split_once('"').unwrap().0
}
