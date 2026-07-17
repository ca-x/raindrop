use serde_json::json;

use super::{document::load_openapi, schema::validate_schema};

#[test]
fn reader_schema_validator_enforces_declared_formats() {
    let document = load_openapi();
    let uuid_schema =
        &document["components"]["schemas"]["EntryStateResponse"]["properties"]["entryId"];
    let uri_schema =
        &document["components"]["schemas"]["InertImageResponse"]["properties"]["sourceUrl"];

    assert!(
        validate_schema(
            &document,
            uuid_schema,
            &json!("00000000-0000-4000-8000-000000000301"),
            "$uuid"
        )
        .is_ok()
    );
    assert!(validate_schema(&document, uuid_schema, &json!("not-a-uuid"), "$uuid").is_err());
    assert!(
        validate_schema(
            &document,
            uri_schema,
            &json!("https://example.test/image.png"),
            "$uri"
        )
        .is_ok()
    );
    assert!(validate_schema(&document, uri_schema, &json!("/relative.png"), "$uri").is_err());
}

#[test]
fn reader_schema_validator_enforces_declared_numeric_bounds() {
    let document = load_openapi();
    let minimum_schema =
        &document["components"]["schemas"]["EntryPageResponse"]["properties"]["snapshotGeneration"];
    let bounded_schema = &document["components"]["parameters"]["Limit"]["schema"];

    assert!(validate_schema(&document, minimum_schema, &json!(0), "$minimum").is_ok());
    assert!(validate_schema(&document, minimum_schema, &json!(-1), "$minimum").is_err());
    assert!(validate_schema(&document, bounded_schema, &json!(1), "$bounded").is_ok());
    assert!(validate_schema(&document, bounded_schema, &json!(100), "$bounded").is_ok());
    assert!(validate_schema(&document, bounded_schema, &json!(101), "$bounded").is_err());
}

#[test]
fn reader_schema_validator_enforces_typed_additional_properties() {
    let document = load_openapi();
    let fields_schema = &document["components"]["schemas"]["ApiError"]["properties"]["fields"];

    assert!(
        validate_schema(
            &document,
            fields_schema,
            &json!({ "entryId": "Entry identifier is invalid" }),
            "$fields"
        )
        .is_ok()
    );
    assert!(
        validate_schema(
            &document,
            fields_schema,
            &json!({ "entryId": 7 }),
            "$fields"
        )
        .is_err()
    );
}
