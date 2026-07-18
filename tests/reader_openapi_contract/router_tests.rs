use axum::http::{Method, StatusCode};
use serde_json::json;

use crate::support::database::ENTRY_A_ID;

use super::{
    document::{ENTRY_PATH, ENTRY_STATE_PATH, load_openapi},
    fixture::ContractFixture,
    response::assert_operation_response,
};

#[tokio::test]
async fn reader_openapi_matches_real_router_success_and_error_responses() {
    let document = load_openapi();
    let fixture = ContractFixture::new().await;
    let entry_uri = format!("/api/v1/entries/{ENTRY_A_ID}");
    let state_uri = format!("{entry_uri}/state");

    assert_operation_response(
        &document,
        "/api/v1/entries",
        "get",
        fixture
            .request(Method::GET, "/api/v1/entries", None, true, true)
            .await,
        StatusCode::OK,
    );
    assert_operation_response(
        &document,
        ENTRY_PATH,
        "get",
        fixture
            .request(Method::GET, &entry_uri, None, true, true)
            .await,
        StatusCode::OK,
    );
    assert_operation_response(
        &document,
        ENTRY_STATE_PATH,
        "patch",
        fixture
            .request(
                Method::PATCH,
                &state_uri,
                Some(json!({ "isStarred": true })),
                true,
                true,
            )
            .await,
        StatusCode::OK,
    );

    for (path, method, response, status) in [
        (
            "/api/v1/entries",
            "get",
            fixture
                .request(Method::GET, "/api/v1/entries?limit=0", None, true, true)
                .await,
            StatusCode::UNPROCESSABLE_ENTITY,
        ),
        (
            "/api/v1/entries",
            "get",
            fixture
                .request(
                    Method::GET,
                    "/api/v1/entries?categoryId=00000000-0000-4000-8000-000000000501",
                    None,
                    true,
                    true,
                )
                .await,
            StatusCode::OK,
        ),
        (
            "/api/v1/entries",
            "get",
            fixture
                .request(
                    Method::GET,
                    "/api/v1/entries?feedId=00000000-0000-4000-8000-000000000101&state=ALL&search=safe",
                    None,
                    true,
                    true,
                )
                .await,
            StatusCode::OK,
        ),
        (
            "/api/v1/entries",
            "get",
            fixture
                .request(
                    Method::GET,
                    "/api/v1/entries?state=ALL&search=safe",
                    None,
                    true,
                    true,
                )
                .await,
            StatusCode::UNPROCESSABLE_ENTITY,
        ),
        (
            "/api/v1/entries",
            "get",
            fixture
                .request(
                    Method::GET,
                    "/api/v1/entries?feedId=00000000-0000-4000-8000-000000000101&categoryId=00000000-0000-4000-8000-000000000501",
                    None,
                    true,
                    true,
                )
                .await,
            StatusCode::UNPROCESSABLE_ENTITY,
        ),
        (
            "/api/v1/entries",
            "get",
            fixture
                .request(Method::GET, "/api/v1/entries", None, false, false)
                .await,
            StatusCode::UNAUTHORIZED,
        ),
        (
            ENTRY_PATH,
            "get",
            fixture
                .request(Method::GET, "/api/v1/entries/not-a-uuid", None, true, true)
                .await,
            StatusCode::UNPROCESSABLE_ENTITY,
        ),
        (
            ENTRY_PATH,
            "get",
            fixture
                .request(
                    Method::GET,
                    "/api/v1/entries/00000000-0000-4000-8000-000000000399",
                    None,
                    true,
                    true,
                )
                .await,
            StatusCode::NOT_FOUND,
        ),
        (
            ENTRY_PATH,
            "get",
            fixture
                .request(Method::GET, &entry_uri, None, false, false)
                .await,
            StatusCode::UNAUTHORIZED,
        ),
        (
            ENTRY_STATE_PATH,
            "patch",
            fixture
                .request(
                    Method::PATCH,
                    &state_uri,
                    Some(json!({ "isRead": true })),
                    false,
                    false,
                )
                .await,
            StatusCode::UNAUTHORIZED,
        ),
        (
            ENTRY_STATE_PATH,
            "patch",
            fixture
                .request(
                    Method::PATCH,
                    &state_uri,
                    Some(json!({ "isRead": true })),
                    true,
                    false,
                )
                .await,
            StatusCode::FORBIDDEN,
        ),
        (
            ENTRY_STATE_PATH,
            "patch",
            fixture
                .request(
                    Method::PATCH,
                    "/api/v1/entries/00000000-0000-4000-8000-000000000399/state",
                    Some(json!({ "isRead": true })),
                    true,
                    true,
                )
                .await,
            StatusCode::NOT_FOUND,
        ),
        (
            ENTRY_STATE_PATH,
            "patch",
            fixture
                .request(Method::PATCH, &state_uri, Some(json!({})), true, true)
                .await,
            StatusCode::UNPROCESSABLE_ENTITY,
        ),
        (
            ENTRY_STATE_PATH,
            "patch",
            fixture
                .request(
                    Method::PATCH,
                    &state_uri,
                    Some(json!({ "isRead": true, "revision": 7 })),
                    true,
                    true,
                )
                .await,
            StatusCode::UNPROCESSABLE_ENTITY,
        ),
    ] {
        assert_operation_response(&document, path, method, response, status);
    }
}
