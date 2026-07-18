use super::document::{assert_required_fields, assert_schema_properties, load_openapi};

#[test]
fn reader_openapi_freezes_entry_dto_shapes() {
    let document = load_openapi();
    assert_shape(
        &document,
        "EntryPageResponse",
        &["items", "nextCursor", "snapshotGeneration"],
    );
    assert_shape(
        &document,
        "EntryListItemResponse",
        &[
            "entryId",
            "feedId",
            "feedTitle",
            "siteUrl",
            "title",
            "author",
            "summary",
            "canonicalUrl",
            "publishedAtUs",
            "sortAtUs",
            "isRead",
            "isStarred",
        ],
    );
    assert_shape(
        &document,
        "EntryDetailResponse",
        &[
            "entryId",
            "feedId",
            "feedTitle",
            "siteUrl",
            "title",
            "author",
            "summary",
            "canonicalUrl",
            "publishedAtUs",
            "sortAtUs",
            "isRead",
            "isStarred",
            "contentHtml",
            "inertImages",
            "enclosures",
        ],
    );
    assert_shape(
        &document,
        "EntryStateResponse",
        &["entryId", "isRead", "isStarred"],
    );
    assert_shape(
        &document,
        "InertImageResponse",
        &["imageIndex", "sourceUrl", "alt", "width", "height"],
    );
    assert_shape(
        &document,
        "EnclosureResponse",
        &["url", "mediaType", "length", "title", "duration"],
    );
}

#[test]
fn reader_openapi_freezes_request_and_error_shapes() {
    let document = load_openapi();
    assert_schema_properties(
        &document,
        "PatchEntryStateRequest",
        &["isRead", "isStarred"],
    );
    assert_required_fields(&document, "MarkEntriesReadRequest", &["snapshotGeneration"]);
    assert_schema_properties(
        &document,
        "MarkEntriesReadRequest",
        &["categoryId", "feedId", "snapshotGeneration"],
    );
    assert_shape(&document, "ErrorEnvelope", &["error"]);
    assert_required_fields(&document, "ApiError", &["code", "message", "requestId"]);
    assert_schema_properties(
        &document,
        "ApiError",
        &["code", "message", "fields", "requestId"],
    );
}

fn assert_shape(document: &serde_json::Value, schema: &str, fields: &[&str]) {
    assert_required_fields(document, schema, fields);
    assert_schema_properties(document, schema, fields);
}
