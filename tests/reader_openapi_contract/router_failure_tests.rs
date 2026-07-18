use axum::http::{Method, StatusCode};
use serde_json::json;

use crate::support::database::ENTRY_A_ID;

use super::{
    document::{ENTRY_PATH, ENTRY_STATE_PATH, MARK_READ_PATH, load_openapi},
    fixture::ContractFixture,
    response::assert_operation_response,
};

#[tokio::test]
async fn reader_openapi_matches_real_router_internal_errors() {
    let document = load_openapi();
    let entry_uri = format!("/api/v1/entries/{ENTRY_A_ID}");
    let state_uri = format!("{entry_uri}/state");
    for (path, method, request_method, uri, body) in [
        (
            "/api/v1/entries",
            "get",
            Method::GET,
            "/api/v1/entries",
            None,
        ),
        (ENTRY_PATH, "get", Method::GET, entry_uri.as_str(), None),
        (
            ENTRY_STATE_PATH,
            "patch",
            Method::PATCH,
            state_uri.as_str(),
            Some(json!({ "isRead": true })),
        ),
        (
            MARK_READ_PATH,
            "post",
            Method::POST,
            MARK_READ_PATH,
            Some(json!({ "snapshotGeneration": 1 })),
        ),
    ] {
        let fixture = ContractFixture::new().await;
        fixture.close_database().await;
        let response = fixture.request(request_method, uri, body, true, true).await;
        assert_operation_response(
            &document,
            path,
            method,
            response,
            StatusCode::INTERNAL_SERVER_ERROR,
        );
    }
}
