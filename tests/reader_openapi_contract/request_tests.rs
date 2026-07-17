use serde_json::json;

use super::{document::load_openapi, schema::validate_schema};

#[test]
fn reader_openapi_patch_request_is_strict() {
    let document = load_openapi();
    let request = &document["components"]["schemas"]["PatchEntryStateRequest"];
    assert!(validate_schema(&document, request, &json!({ "isRead": true }), "$patch").is_ok());
    assert!(
        validate_schema(
            &document,
            request,
            &json!({ "isRead": false, "isStarred": true }),
            "$patch"
        )
        .is_ok()
    );
    for invalid in [
        json!({}),
        json!({ "isRead": null }),
        json!({ "isRead": 1 }),
        json!({ "isStarred": "true" }),
        json!({ "isRead": true, "revision": 7 }),
    ] {
        assert!(
            validate_schema(&document, request, &invalid, "$patch").is_err(),
            "strict patch schema accepted {invalid}"
        );
    }
}

#[test]
fn reader_openapi_does_not_leak_internal_fields() {
    let document = load_openapi();
    let serialized = serde_json::to_string(&document)
        .expect("reader OpenAPI artifact should serialize")
        .to_ascii_lowercase();
    for forbidden in [
        "staterevision",
        "revision",
        "storage",
        "sanitizedcontent",
        "sourcecontenthash",
        "contenthash",
        "pipelineversion",
        "ingestgeneration",
        "identitykind",
        "identityhash",
        "readoverride",
        "leaseowner",
        "leasetoken",
        "fetchurl",
    ] {
        assert!(
            !serialized.contains(forbidden),
            "reader OpenAPI leaks forbidden internal detail: {forbidden}"
        );
    }
}
