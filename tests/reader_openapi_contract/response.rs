use axum::http::{
    StatusCode,
    header::{CACHE_CONTROL, CONTENT_TYPE, PRAGMA},
};
use serde_json::Value;

use super::{document::resolve_ref, fixture::CapturedResponse, schema::validate_schema};

pub(crate) fn assert_operation_response(
    document: &Value,
    path: &str,
    method: &str,
    response: CapturedResponse,
    expected_status: StatusCode,
) {
    assert_eq!(
        response.status, expected_status,
        "unexpected status for {method} {path}"
    );
    assert_eq!(response.headers.get(CACHE_CONTROL).unwrap(), "no-store");
    assert_eq!(response.headers.get(PRAGMA).unwrap(), "no-cache");
    let response_contract = resolve_ref(
        document,
        &document["paths"][path][method]["responses"][expected_status.as_u16().to_string()],
    );
    assert!(response_contract["headers"]["Cache-Control"].is_object());
    assert!(response_contract["headers"]["Pragma"].is_object());
    if expected_status == StatusCode::NO_CONTENT {
        assert!(response.headers.get(CONTENT_TYPE).is_none());
        assert!(response_contract.get("content").is_none());
        assert!(response.body_is_empty());
        return;
    }
    assert_eq!(
        response.headers.get(CONTENT_TYPE).unwrap(),
        "application/json",
        "reader responses must use JSON content type"
    );
    let schema = &response_contract["content"]["application/json"]["schema"];
    let body = response.json();
    validate_schema(document, schema, &body, "$response")
        .unwrap_or_else(|error| panic!("{method} {path} response violates artifact: {error}"));
}
