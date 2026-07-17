use std::collections::BTreeSet;

use serde_json::json;

use super::document::{
    ENTRY_PATH, ENTRY_STATE_PATH, assert_all_local_refs_resolve, assert_operation_statuses,
    documented_operations, load_openapi, parameter_names,
};

#[test]
fn reader_openapi_declares_exact_operations_and_security() {
    let document = load_openapi();
    assert_eq!(document["openapi"], "3.1.0");
    assert_eq!(
        documented_operations(&document),
        BTreeSet::from([
            ("GET".to_owned(), "/api/v1/entries".to_owned()),
            ("GET".to_owned(), ENTRY_PATH.to_owned()),
            ("PATCH".to_owned(), ENTRY_STATE_PATH.to_owned()),
        ])
    );
    assert_operation_statuses(&document, "/api/v1/entries", "get", &[200, 401, 422, 500]);
    assert_operation_statuses(&document, ENTRY_PATH, "get", &[200, 401, 404, 422, 500]);
    assert_operation_statuses(
        &document,
        ENTRY_STATE_PATH,
        "patch",
        &[200, 401, 403, 404, 422, 500],
    );

    assert_eq!(
        parameter_names(&document, "/api/v1/entries", "get"),
        BTreeSet::from([
            "cursor".to_owned(),
            "feedId".to_owned(),
            "limit".to_owned(),
            "state".to_owned(),
        ])
    );
    assert_eq!(
        parameter_names(&document, ENTRY_PATH, "get"),
        BTreeSet::from(["entryId".to_owned()])
    );
    assert_eq!(
        parameter_names(&document, ENTRY_STATE_PATH, "patch"),
        BTreeSet::from(["entryId".to_owned(), "x-csrf-token".to_owned()])
    );
    for (path, method) in [
        ("/api/v1/entries", "get"),
        (ENTRY_PATH, "get"),
        (ENTRY_STATE_PATH, "patch"),
    ] {
        assert_eq!(
            document["paths"][path][method]["security"],
            json!([{ "sessionCookie": [] }]),
            "{method} {path} must require the session cookie"
        );
    }
}

#[test]
fn reader_openapi_declares_only_the_frozen_public_schemas() {
    let document = load_openapi();
    let schemas = document["components"]["schemas"]
        .as_object()
        .expect("reader schemas should be an object");
    assert_eq!(
        schemas.keys().cloned().collect::<BTreeSet<_>>(),
        BTreeSet::from([
            "ApiError".to_owned(),
            "EnclosureResponse".to_owned(),
            "EntryDetailResponse".to_owned(),
            "EntryListItemResponse".to_owned(),
            "EntryListState".to_owned(),
            "EntryPageResponse".to_owned(),
            "EntryStateResponse".to_owned(),
            "ErrorEnvelope".to_owned(),
            "InertImageResponse".to_owned(),
            "PatchEntryStateRequest".to_owned(),
        ])
    );
    for schema in [
        "EntryPageResponse",
        "EntryListItemResponse",
        "EntryDetailResponse",
        "PatchEntryStateRequest",
        "EntryStateResponse",
        "InertImageResponse",
        "EnclosureResponse",
        "ErrorEnvelope",
        "ApiError",
    ] {
        assert_eq!(
            document["components"]["schemas"][schema]["additionalProperties"], false,
            "{schema} must reject undeclared fields"
        );
    }
    assert_all_local_refs_resolve(&document);
}
