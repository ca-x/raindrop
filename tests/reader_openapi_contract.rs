#[allow(dead_code)]
mod support;

use std::{collections::BTreeSet, fs, path::PathBuf};

use axum::{
    Router,
    body::Body,
    http::{
        HeaderMap, Method, Request, StatusCode,
        header::{CACHE_CONTROL, CONTENT_TYPE, COOKIE, HOST, ORIGIN, PRAGMA},
    },
};
use http_body_util::BodyExt;
use raindrop::{
    app::{AppState, build_router},
    auth::build_session_cookie,
    db::{DatabaseConfig, connect, entities::rss_counter, migrate},
    setup::SetupService,
};
use sea_orm::{ActiveModelTrait, ActiveValue::Set, DatabaseConnection};
use secrecy::{ExposeSecret, SecretString};
use serde_json::{Value, json};
use tempfile::TempDir;
use time::OffsetDateTime;
use tower::ServiceExt;

use support::database::{
    ENTRY_A_ID, HASH_A, SUBSCRIPTION_A_ID, USER_A_ID, entry_model, insert_feed, insert_user,
    subscription_model,
};

const OPENAPI_PATH: &str = "docs/openapi/reader-v1.json";
const ENTRY_PATH: &str = "/api/v1/entries/{entryId}";
const ENTRY_STATE_PATH: &str = "/api/v1/entries/{entryId}/state";

struct ContractFixture {
    _data: TempDir,
    app: Router,
    database: DatabaseConnection,
    cookie: String,
    csrf: String,
}

impl ContractFixture {
    async fn new() -> Self {
        let data = tempfile::tempdir().expect("temporary directory should be created");
        let database_url = format!(
            "sqlite://{}?mode=rwc",
            data.path().join("reader-openapi.db").display()
        );
        let database = connect(&DatabaseConfig::new(SecretString::from(database_url)))
            .await
            .expect("reader OpenAPI database should connect");
        migrate(&database)
            .await
            .expect("reader OpenAPI database should migrate");

        let now = OffsetDateTime::now_utc();
        insert_user(&database, USER_A_ID, "reader-openapi").await;
        insert_feed(&database, now).await;
        let mut subscription = subscription_model(SUBSCRIPTION_A_ID, USER_A_ID, now);
        subscription.start_sequence = Set(0);
        subscription
            .insert(&database)
            .await
            .expect("reader OpenAPI subscription should insert");
        entry_model(
            ENTRY_A_ID,
            1,
            "reader-openapi-entry",
            HASH_A,
            Some(1_784_246_400_000_000),
            now,
        )
        .insert(&database)
        .await
        .expect("reader OpenAPI entry should insert");
        rss_counter::ActiveModel {
            key: Set("INGEST_GENERATION".to_owned()),
            value: Set(1),
        }
        .update(&database)
        .await
        .expect("reader OpenAPI generation should update");

        let setup = SetupService::ready(data.path(), None, database.clone());
        let session = setup
            .sessions()
            .create(USER_A_ID)
            .await
            .expect("reader OpenAPI session should create");
        let cookie = build_session_cookie(&session, false)
            .to_string()
            .split(';')
            .next()
            .expect("session cookie should contain a pair")
            .to_owned();
        let csrf = session.csrf_token.expose_secret().to_owned();
        let app = build_router(AppState::new(setup));

        Self {
            _data: data,
            app,
            database,
            cookie,
            csrf,
        }
    }

    async fn request(
        &self,
        method: Method,
        uri: &str,
        body: Option<Value>,
        authenticated: bool,
        valid_csrf: bool,
    ) -> CapturedResponse {
        let mut request = Request::builder().method(method.clone()).uri(uri);
        if authenticated {
            request = request.header(COOKIE, &self.cookie);
        }
        if method == Method::PATCH {
            request = request
                .header(
                    "x-csrf-token",
                    if valid_csrf {
                        &self.csrf
                    } else {
                        "invalid-csrf"
                    },
                )
                .header(ORIGIN, "http://reader-openapi.test")
                .header(HOST, "reader-openapi.test");
        }
        if body.is_some() {
            request = request.header(CONTENT_TYPE, "application/json");
        }
        let request = request
            .body(body.map_or_else(Body::empty, |value| Body::from(value.to_string())))
            .expect("reader OpenAPI request should build");
        let response = self
            .app
            .clone()
            .oneshot(request)
            .await
            .expect("reader OpenAPI request should complete");
        CapturedResponse::from_response(response).await
    }

    async fn close_database(&self) {
        self.database
            .clone()
            .close()
            .await
            .expect("reader OpenAPI database should close");
    }
}

struct CapturedResponse {
    status: StatusCode,
    headers: HeaderMap,
    body: Vec<u8>,
}

impl CapturedResponse {
    async fn from_response(response: axum::response::Response) -> Self {
        let (parts, body) = response.into_parts();
        let body = body
            .collect()
            .await
            .expect("reader OpenAPI response should collect")
            .to_bytes()
            .to_vec();
        Self {
            status: parts.status,
            headers: parts.headers,
            body,
        }
    }

    fn json(&self) -> Value {
        serde_json::from_slice(&self.body).expect("reader response should contain JSON")
    }
}

#[test]
fn reader_openapi_declares_the_exact_public_contract() {
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
    assert_required_fields(
        &document,
        "EntryPageResponse",
        &["items", "nextCursor", "snapshotGeneration"],
    );
    assert_required_fields(
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
    assert_required_fields(
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
    assert_required_fields(
        &document,
        "EntryStateResponse",
        &["entryId", "isRead", "isStarred"],
    );
    assert_required_fields(&document, "ApiError", &["code", "message", "requestId"]);
    assert_schema_properties(
        &document,
        "PatchEntryStateRequest",
        &["isRead", "isStarred"],
    );
    assert_schema_properties(
        &document,
        "EntryPageResponse",
        &["items", "nextCursor", "snapshotGeneration"],
    );
    assert_schema_properties(
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
    assert_schema_properties(
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
    assert_schema_properties(
        &document,
        "EntryStateResponse",
        &["entryId", "isRead", "isStarred"],
    );
    assert_schema_properties(
        &document,
        "InertImageResponse",
        &["imageIndex", "sourceUrl", "alt", "width", "height"],
    );
    assert_schema_properties(
        &document,
        "EnclosureResponse",
        &["url", "mediaType", "length", "title", "duration"],
    );
    assert_schema_properties(&document, "ErrorEnvelope", &["error"]);
    assert_schema_properties(
        &document,
        "ApiError",
        &["code", "message", "fields", "requestId"],
    );

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

    assert_all_local_refs_resolve(&document);
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

#[tokio::test]
async fn reader_openapi_matches_real_router_responses() {
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
    ] {
        let broken = ContractFixture::new().await;
        broken.close_database().await;
        let response = broken.request(request_method, uri, body, true, true).await;
        assert_operation_response(
            &document,
            path,
            method,
            response,
            StatusCode::INTERNAL_SERVER_ERROR,
        );
    }
}

fn load_openapi() -> Value {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(OPENAPI_PATH);
    let bytes = fs::read(&path).unwrap_or_else(|error| {
        panic!(
            "reader OpenAPI artifact {} must exist: {error}",
            path.display()
        )
    });
    serde_json::from_slice(&bytes).expect("reader OpenAPI artifact should contain valid JSON")
}

fn documented_operations(document: &Value) -> BTreeSet<(String, String)> {
    let mut operations = BTreeSet::new();
    for (path, item) in document["paths"]
        .as_object()
        .expect("OpenAPI paths should be an object")
    {
        for method in ["get", "post", "put", "patch", "delete"] {
            if item.get(method).is_some() {
                operations.insert((method.to_ascii_uppercase(), path.clone()));
            }
        }
    }
    operations
}

fn assert_operation_statuses(document: &Value, path: &str, method: &str, expected: &[u16]) {
    let statuses = document["paths"][path][method]["responses"]
        .as_object()
        .expect("operation responses should be an object")
        .keys()
        .map(|status| {
            status
                .parse::<u16>()
                .expect("response status should be numeric")
        })
        .collect::<BTreeSet<_>>();
    assert_eq!(statuses, expected.iter().copied().collect());
}

fn parameter_names(document: &Value, path: &str, method: &str) -> BTreeSet<String> {
    let mut names = BTreeSet::new();
    for value in [
        document["paths"][path]["parameters"].as_array(),
        document["paths"][path][method]["parameters"].as_array(),
    ]
    .into_iter()
    .flatten()
    .flatten()
    {
        let parameter = resolve_ref(document, value);
        names.insert(
            parameter["name"]
                .as_str()
                .expect("parameter name should be a string")
                .to_owned(),
        );
    }
    names
}

fn assert_required_fields(document: &Value, schema: &str, expected: &[&str]) {
    let actual = document["components"]["schemas"][schema]["required"]
        .as_array()
        .expect("schema required should be an array")
        .iter()
        .map(|value| value.as_str().expect("required field should be a string"))
        .collect::<BTreeSet<_>>();
    assert_eq!(actual, expected.iter().copied().collect());
}

fn assert_schema_properties(document: &Value, schema: &str, expected: &[&str]) {
    let actual = document["components"]["schemas"][schema]["properties"]
        .as_object()
        .expect("schema properties should be an object")
        .keys()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    assert_eq!(actual, expected.iter().copied().collect());
}

fn assert_all_local_refs_resolve(document: &Value) {
    visit_local_refs(document, document, "$document");
}

fn visit_local_refs(document: &Value, value: &Value, path: &str) {
    match value {
        Value::Object(object) => {
            if let Some(reference) = object.get("$ref") {
                let reference = reference.as_str().expect("$ref should be a string");
                assert!(
                    reference.starts_with("#/"),
                    "external ref at {path}: {reference}"
                );
                let pointer = reference
                    .strip_prefix('#')
                    .expect("local ref should start with #");
                assert!(
                    document.pointer(pointer).is_some(),
                    "unresolved local ref at {path}: {reference}"
                );
            }
            for (key, child) in object {
                visit_local_refs(document, child, &format!("{path}.{key}"));
            }
        }
        Value::Array(array) => {
            for (index, child) in array.iter().enumerate() {
                visit_local_refs(document, child, &format!("{path}[{index}]"));
            }
        }
        _ => {}
    }
}

fn assert_operation_response(
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
    assert_eq!(
        response.headers.get(CONTENT_TYPE).unwrap(),
        "application/json",
        "reader responses must use JSON content type"
    );

    let response_contract = resolve_ref(
        document,
        &document["paths"][path][method]["responses"][expected_status.as_u16().to_string()],
    );
    assert!(response_contract["headers"]["Cache-Control"].is_object());
    assert!(response_contract["headers"]["Pragma"].is_object());
    let schema = &response_contract["content"]["application/json"]["schema"];
    let body = response.json();
    validate_schema(document, schema, &body, "$response")
        .unwrap_or_else(|error| panic!("{method} {path} response violates artifact: {error}"));
}

fn resolve_ref<'a>(document: &'a Value, value: &'a Value) -> &'a Value {
    if let Some(reference) = value.get("$ref").and_then(Value::as_str) {
        let pointer = reference
            .strip_prefix('#')
            .expect("only local OpenAPI refs are supported");
        return document
            .pointer(pointer)
            .unwrap_or_else(|| panic!("OpenAPI ref should resolve: {reference}"));
    }
    value
}

fn validate_schema(
    document: &Value,
    schema: &Value,
    value: &Value,
    path: &str,
) -> Result<(), String> {
    let schema = resolve_ref(document, schema);
    if let Some(any_of) = schema.get("anyOf").and_then(Value::as_array) {
        if any_of
            .iter()
            .any(|candidate| validate_schema(document, candidate, value, path).is_ok())
        {
            return validate_object_keywords(document, schema, value, path);
        }
        return Err(format!("{path} did not match anyOf"));
    }

    if let Some(enum_values) = schema.get("enum").and_then(Value::as_array)
        && !enum_values.contains(value)
    {
        return Err(format!("{path} is not an allowed enum value"));
    }

    let types = schema.get("type").map_or_else(Vec::new, |kind| match kind {
        Value::String(kind) => vec![kind.as_str()],
        Value::Array(kinds) => kinds.iter().filter_map(Value::as_str).collect(),
        _ => Vec::new(),
    });
    if !types.is_empty() && !types.iter().any(|kind| matches_type(kind, value)) {
        return Err(format!("{path} has the wrong type"));
    }

    validate_object_keywords(document, schema, value, path)?;
    if let Some(items) = schema.get("items")
        && let Some(values) = value.as_array()
    {
        for (index, item) in values.iter().enumerate() {
            validate_schema(document, items, item, &format!("{path}[{index}]"))?;
        }
    }
    Ok(())
}

fn validate_object_keywords(
    document: &Value,
    schema: &Value,
    value: &Value,
    path: &str,
) -> Result<(), String> {
    let Some(object) = value.as_object() else {
        return Ok(());
    };
    let properties = schema.get("properties").and_then(Value::as_object);
    if let Some(required) = schema.get("required").and_then(Value::as_array) {
        for field in required.iter().filter_map(Value::as_str) {
            if !object.contains_key(field) {
                return Err(format!("{path}.{field} is required"));
            }
        }
    }
    if schema.get("additionalProperties") == Some(&Value::Bool(false)) {
        let properties = properties.ok_or_else(|| format!("{path} has no properties"))?;
        for field in object.keys() {
            if !properties.contains_key(field) {
                return Err(format!("{path}.{field} is not allowed"));
            }
        }
    }
    if let Some(properties) = properties {
        for (field, field_schema) in properties {
            if let Some(field_value) = object.get(field) {
                validate_schema(
                    document,
                    field_schema,
                    field_value,
                    &format!("{path}.{field}"),
                )?;
            }
        }
    }
    if let Some(any_of) = schema.get("anyOf").and_then(Value::as_array)
        && !any_of
            .iter()
            .any(|candidate| validate_schema(document, candidate, value, path).is_ok())
    {
        return Err(format!("{path} did not match anyOf"));
    }
    Ok(())
}

fn matches_type(kind: &str, value: &Value) -> bool {
    match kind {
        "null" => value.is_null(),
        "object" => value.is_object(),
        "array" => value.is_array(),
        "string" => value.is_string(),
        "integer" => value.as_i64().is_some() || value.as_u64().is_some(),
        "number" => value.is_number(),
        "boolean" => value.is_boolean(),
        _ => false,
    }
}
