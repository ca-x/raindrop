#[allow(dead_code)]
mod support;

use std::{collections::BTreeSet, fs};

use axum::{
    Router,
    body::Body,
    http::{
        HeaderMap, Method, Request, StatusCode,
        header::{
            CACHE_CONTROL, CONTENT_TYPE, COOKIE, HOST, LOCATION, ORIGIN, PRAGMA, RETRY_AFTER,
        },
    },
};
use http_body_util::BodyExt;
use raindrop::{
    app::{AppState, build_router},
    auth::build_session_cookie,
    db::{
        DatabaseConfig, connect,
        entities::{feed, feed_refresh_run},
        migrate,
    },
    setup::SetupService,
};
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, DatabaseConnection, EntityTrait, IntoActiveModel,
};
use secrecy::{ExposeSecret, SecretString};
use serde_json::{Value, json};
use tempfile::TempDir;
use time::macros::datetime;
use tower::ServiceExt;
use uuid::Uuid;

use support::database::{USER_A_ID, insert_user};

const OPENAPI_PATH: &str = "docs/openapi/subscription-v1.json";
const SUBSCRIPTION_PATH: &str = "/api/v1/subscriptions/{subscriptionId}";
const REFRESH_PATH: &str = "/api/v1/subscriptions/{subscriptionId}/refresh";

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
            data.path().join("openapi-contract.db").display()
        );
        let database = connect(&DatabaseConfig::new(SecretString::from(database_url)))
            .await
            .expect("OpenAPI contract database should connect");
        migrate(&database)
            .await
            .expect("OpenAPI contract database should migrate");
        insert_user(&database, USER_A_ID, "openapi-user").await;

        let setup = SetupService::ready(data.path(), None, database.clone());
        let session = setup
            .sessions()
            .create(USER_A_ID)
            .await
            .expect("OpenAPI contract session should create");
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
        let mutation = matches!(method, Method::POST | Method::PATCH | Method::DELETE);
        let mut request = Request::builder().method(method).uri(uri);
        if authenticated {
            request = request.header(COOKIE, &self.cookie);
        }
        if mutation {
            request = request
                .header(
                    "x-csrf-token",
                    if valid_csrf {
                        &self.csrf
                    } else {
                        "invalid-csrf"
                    },
                )
                .header(ORIGIN, "http://openapi.test")
                .header(HOST, "openapi.test");
        }
        if body.is_some() {
            request = request.header(CONTENT_TYPE, "application/json");
        }
        let request = request
            .body(body.map_or_else(Body::empty, |value| Body::from(value.to_string())))
            .expect("OpenAPI contract request should build");
        let response = self
            .app
            .clone()
            .oneshot(request)
            .await
            .expect("OpenAPI contract request should complete");
        CapturedResponse::from_response(response).await
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
            .expect("OpenAPI contract response body should collect")
            .to_bytes()
            .to_vec();
        Self {
            status: parts.status,
            headers: parts.headers,
            body,
        }
    }

    fn json(&self) -> Value {
        serde_json::from_slice(&self.body).expect("response should contain JSON")
    }
}

#[test]
fn subscription_openapi_declares_only_the_real_public_surface() {
    let document = load_openapi();
    assert_eq!(document["openapi"], "3.1.0");

    let actual_operations = documented_operations(&document);
    let expected_operations = BTreeSet::from([
        ("GET".to_owned(), "/api/v1/subscriptions".to_owned()),
        ("POST".to_owned(), "/api/v1/subscriptions".to_owned()),
        ("GET".to_owned(), SUBSCRIPTION_PATH.to_owned()),
        ("DELETE".to_owned(), SUBSCRIPTION_PATH.to_owned()),
        ("POST".to_owned(), REFRESH_PATH.to_owned()),
    ]);
    assert_eq!(actual_operations, expected_operations);

    assert_operation_statuses(
        &document,
        "/api/v1/subscriptions",
        "get",
        &[200, 401, 422, 500],
    );
    assert_operation_statuses(
        &document,
        "/api/v1/subscriptions",
        "post",
        &[200, 201, 401, 403, 422, 429, 500],
    );
    assert_operation_statuses(
        &document,
        SUBSCRIPTION_PATH,
        "get",
        &[200, 401, 404, 422, 500],
    );
    assert_operation_statuses(
        &document,
        SUBSCRIPTION_PATH,
        "delete",
        &[204, 401, 403, 422, 429, 500],
    );
    assert_operation_statuses(
        &document,
        REFRESH_PATH,
        "post",
        &[200, 202, 401, 403, 404, 409, 422, 429, 500],
    );
    assert_eq!(
        documented_statuses(&document),
        BTreeSet::from([200, 201, 202, 204, 401, 403, 404, 409, 422, 429, 500])
    );

    assert_eq!(
        string_set(&document["components"]["schemas"]["Refresh"]["properties"]["state"]["enum"]),
        BTreeSet::from([
            "BACKING_OFF".to_owned(),
            "DEGRADED".to_owned(),
            "ERROR".to_owned(),
            "PENDING".to_owned(),
            "READY".to_owned(),
        ])
    );
    assert_eq!(
        string_set(
            &document["components"]["schemas"]["Refresh"]["properties"]["errorCode"]["enum"]
        ),
        BTreeSet::from([
            "REFRESH_FAILED".to_owned(),
            "UPSTREAM_RATE_LIMITED".to_owned(),
        ])
    );

    assert_required_fields(
        &document,
        "Subscription",
        &[
            "subscriptionId",
            "feedId",
            "title",
            "siteUrl",
            "unreadCount",
            "refresh",
        ],
    );
    assert_required_fields(&document, "CreateSubscriptionRequest", &["url"]);
    assert_required_fields(&document, "RefreshSubscriptionRequest", &["requestId"]);
    assert_required_fields(
        &document,
        "Refresh",
        &[
            "operationId",
            "state",
            "newCount",
            "updatedCount",
            "droppedCount",
            "generation",
            "errorCode",
            "retryAt",
            "queuedAt",
            "startedAt",
            "completedAt",
        ],
    );
    assert_required_fields(&document, "ApiError", &["code", "message", "requestId"]);
    assert!(
        !string_set(&document["components"]["schemas"]["ApiError"]["required"]).contains("fields")
    );

    let rate_limited = resolve_ref(
        &document,
        &document["components"]["responses"]["RateLimited"],
    );
    assert_eq!(
        rate_limited["headers"]["Retry-After"]["schema"]["type"],
        "integer"
    );
    assert_eq!(
        rate_limited["headers"]["Retry-After"]["schema"]["minimum"],
        1
    );

    let serialized = serde_json::to_string(&document)
        .expect("OpenAPI artifact should serialize")
        .to_ascii_lowercase();
    for forbidden in [
        "sourceurl",
        "fetchurl",
        "httpstatus",
        "leaseowner",
        "leasetoken",
        "frontier",
        "staterevision",
        "providererror",
        "databaseurl",
    ] {
        assert!(
            !serialized.contains(forbidden),
            "OpenAPI artifact leaks forbidden internal detail: {forbidden}"
        );
    }
}

#[tokio::test]
async fn subscription_openapi_matches_real_router_responses() {
    const REQUEST_A: &str = "00000000-0000-4000-8000-000000000701";
    const REQUEST_B: &str = "00000000-0000-4000-8000-000000000702";

    let document = load_openapi();
    let mut covered_statuses = BTreeSet::new();
    let fixture = ContractFixture::new().await;

    let list = fixture
        .request(Method::GET, "/api/v1/subscriptions", None, true, true)
        .await;
    assert_operation_response(
        &document,
        "/api/v1/subscriptions",
        "get",
        StatusCode::OK,
        &list,
    );
    covered_statuses.insert(list.status.as_u16());

    let created = fixture
        .request(
            Method::POST,
            "/api/v1/subscriptions",
            Some(json!({ "url": "https://feed.example/rss.xml" })),
            true,
            true,
        )
        .await;
    assert_operation_response(
        &document,
        "/api/v1/subscriptions",
        "post",
        StatusCode::CREATED,
        &created,
    );
    let created_body = created.json();
    let subscription_id = created_body["subscription"]["subscriptionId"]
        .as_str()
        .expect("created subscription should expose subscriptionId")
        .to_owned();
    let subscribe_operation_id = created_body["subscription"]["refresh"]["operationId"]
        .as_str()
        .expect("created subscription should expose operationId")
        .to_owned();
    assert_eq!(
        created.headers.get(LOCATION).unwrap(),
        &format!("/api/v1/subscriptions/{subscription_id}")
    );
    covered_statuses.insert(created.status.as_u16());

    let existing = fixture
        .request(
            Method::POST,
            "/api/v1/subscriptions",
            Some(json!({ "url": "https://feed.example/rss.xml" })),
            true,
            true,
        )
        .await;
    assert_operation_response(
        &document,
        "/api/v1/subscriptions",
        "post",
        StatusCode::OK,
        &existing,
    );
    assert_eq!(existing.json()["created"], false);

    let detail_uri = format!("/api/v1/subscriptions/{subscription_id}");
    let detail = fixture
        .request(Method::GET, &detail_uri, None, true, true)
        .await;
    assert_operation_response(&document, SUBSCRIPTION_PATH, "get", StatusCode::OK, &detail);

    mark_run_terminal(&fixture.database, &subscribe_operation_id).await;
    let refresh_uri = format!("/api/v1/subscriptions/{subscription_id}/refresh");
    let accepted = fixture
        .request(
            Method::POST,
            &refresh_uri,
            Some(json!({ "requestId": REQUEST_A })),
            true,
            true,
        )
        .await;
    assert_operation_response(
        &document,
        REFRESH_PATH,
        "post",
        StatusCode::ACCEPTED,
        &accepted,
    );
    let manual_operation_id = accepted.json()["operationId"]
        .as_str()
        .expect("accepted refresh should expose operationId")
        .to_owned();
    covered_statuses.insert(accepted.status.as_u16());

    let conflict = fixture
        .request(
            Method::POST,
            &refresh_uri,
            Some(json!({ "requestId": REQUEST_B })),
            true,
            true,
        )
        .await;
    assert_operation_response(
        &document,
        REFRESH_PATH,
        "post",
        StatusCode::CONFLICT,
        &conflict,
    );
    assert_eq!(conflict.json()["error"]["code"], "REFRESH_IN_PROGRESS");
    covered_statuses.insert(conflict.status.as_u16());

    mark_run_terminal(&fixture.database, &manual_operation_id).await;
    let replay = fixture
        .request(
            Method::POST,
            &refresh_uri,
            Some(json!({ "requestId": REQUEST_A })),
            true,
            true,
        )
        .await;
    assert_operation_response(&document, REFRESH_PATH, "post", StatusCode::OK, &replay);

    let deleted = fixture
        .request(Method::DELETE, &detail_uri, None, true, true)
        .await;
    assert_operation_response(
        &document,
        SUBSCRIPTION_PATH,
        "delete",
        StatusCode::NO_CONTENT,
        &deleted,
    );
    covered_statuses.insert(deleted.status.as_u16());

    let unauthorized = fixture
        .request(Method::GET, "/api/v1/subscriptions", None, false, false)
        .await;
    assert_operation_response(
        &document,
        "/api/v1/subscriptions",
        "get",
        StatusCode::UNAUTHORIZED,
        &unauthorized,
    );
    covered_statuses.insert(unauthorized.status.as_u16());

    let forbidden = fixture
        .request(
            Method::POST,
            "/api/v1/subscriptions",
            Some(json!({ "url": "https://forbidden.example/rss.xml" })),
            true,
            false,
        )
        .await;
    assert_operation_response(
        &document,
        "/api/v1/subscriptions",
        "post",
        StatusCode::FORBIDDEN,
        &forbidden,
    );
    covered_statuses.insert(forbidden.status.as_u16());

    let missing = fixture
        .request(
            Method::GET,
            "/api/v1/subscriptions/00000000-0000-4000-8000-000000000299",
            None,
            true,
            true,
        )
        .await;
    assert_operation_response(
        &document,
        SUBSCRIPTION_PATH,
        "get",
        StatusCode::NOT_FOUND,
        &missing,
    );
    covered_statuses.insert(missing.status.as_u16());

    let invalid = fixture
        .request(
            Method::GET,
            "/api/v1/subscriptions?limit=0",
            None,
            true,
            true,
        )
        .await;
    assert_operation_response(
        &document,
        "/api/v1/subscriptions",
        "get",
        StatusCode::UNPROCESSABLE_ENTITY,
        &invalid,
    );
    covered_statuses.insert(invalid.status.as_u16());

    let temporal_fixture = ContractFixture::new().await;
    for _ in 0..30 {
        let response = temporal_fixture
            .request(
                Method::POST,
                "/api/v1/subscriptions",
                Some(json!({ "url": "https://limited.example/rss.xml" })),
                true,
                true,
            )
            .await;
        assert!(matches!(
            response.status,
            StatusCode::OK | StatusCode::CREATED
        ));
    }
    let temporal_limit = temporal_fixture
        .request(
            Method::POST,
            "/api/v1/subscriptions",
            Some(json!({ "url": "https://limited.example/rss.xml" })),
            true,
            true,
        )
        .await;
    assert_operation_response(
        &document,
        "/api/v1/subscriptions",
        "post",
        StatusCode::TOO_MANY_REQUESTS,
        &temporal_limit,
    );
    let retry_after = temporal_limit
        .headers
        .get(RETRY_AFTER)
        .expect("temporal 429 should include Retry-After")
        .to_str()
        .expect("Retry-After should be ASCII")
        .parse::<u64>()
        .expect("Retry-After should be integer seconds");
    assert!(retry_after >= 1);
    assert!(
        temporal_limit.json()["error"]["fields"]["retryAt"]
            .as_str()
            .is_some_and(|value| value.ends_with('Z'))
    );
    covered_statuses.insert(temporal_limit.status.as_u16());

    let hard_fixture = ContractFixture::new().await;
    let hard_created = hard_fixture
        .request(
            Method::POST,
            "/api/v1/subscriptions",
            Some(json!({ "url": "https://hard-limit.example/rss.xml" })),
            true,
            true,
        )
        .await;
    let hard_body = hard_created.json();
    let hard_subscription_id = hard_body["subscription"]["subscriptionId"]
        .as_str()
        .expect("hard-limit subscription should expose subscriptionId")
        .to_owned();
    let hard_subscribe_operation_id = hard_body["subscription"]["refresh"]["operationId"]
        .as_str()
        .expect("hard-limit subscription should expose operationId")
        .to_owned();
    mark_run_terminal(&hard_fixture.database, &hard_subscribe_operation_id).await;
    seed_active_refresh_quota(&hard_fixture.database).await;
    let hard_limit = hard_fixture
        .request(
            Method::POST,
            &format!("/api/v1/subscriptions/{hard_subscription_id}/refresh"),
            Some(json!({ "requestId": "00000000-0000-4000-8000-000000000703" })),
            true,
            true,
        )
        .await;
    assert_operation_response(
        &document,
        REFRESH_PATH,
        "post",
        StatusCode::TOO_MANY_REQUESTS,
        &hard_limit,
    );
    assert!(hard_limit.headers.get(RETRY_AFTER).is_none());
    assert!(hard_limit.json()["error"].get("fields").is_none());

    let internal_fixture = ContractFixture::new().await;
    let internal_created = internal_fixture
        .request(
            Method::POST,
            "/api/v1/subscriptions",
            Some(json!({ "url": "https://internal.example/rss.xml" })),
            true,
            true,
        )
        .await;
    let internal_body = internal_created.json();
    let internal_subscription_id = internal_body["subscription"]["subscriptionId"]
        .as_str()
        .expect("internal-error subscription should expose subscriptionId")
        .to_owned();
    let internal_operation_id = internal_body["subscription"]["refresh"]["operationId"]
        .as_str()
        .expect("internal-error subscription should expose operationId")
        .to_owned();
    corrupt_run_status(&internal_fixture.database, &internal_operation_id).await;
    let internal = internal_fixture
        .request(
            Method::GET,
            &format!("/api/v1/subscriptions/{internal_subscription_id}"),
            None,
            true,
            true,
        )
        .await;
    assert_operation_response(
        &document,
        SUBSCRIPTION_PATH,
        "get",
        StatusCode::INTERNAL_SERVER_ERROR,
        &internal,
    );
    covered_statuses.insert(internal.status.as_u16());

    assert_eq!(covered_statuses, documented_statuses(&document));

    for (method, uri, expected) in [
        (Method::GET, "/api/v1/subscriptions/", StatusCode::NOT_FOUND),
        (
            Method::GET,
            "/api/v1/subscriptions/not-a-real-route/extra",
            StatusCode::NOT_FOUND,
        ),
        (
            Method::PATCH,
            "/api/v1/subscriptions/00000000-0000-4000-8000-000000000299",
            StatusCode::METHOD_NOT_ALLOWED,
        ),
    ] {
        let fallback = fixture.request(method, uri, None, true, true).await;
        assert_eq!(fallback.status, expected);
        assert_json_cache_contract(&fallback);
        validate_schema(
            &document,
            &document["components"]["schemas"]["ErrorEnvelope"],
            &fallback.json(),
            "$",
        )
        .unwrap_or_else(|error| panic!("fallback error schema mismatch: {error}"));
    }
}

fn load_openapi() -> Value {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(OPENAPI_PATH);
    let artifact = fs::read_to_string(&path).unwrap_or_else(|error| {
        panic!("OpenAPI artifact {} should exist: {error}", path.display())
    });
    serde_json::from_str(&artifact).expect("OpenAPI artifact should contain valid JSON")
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

fn documented_statuses(document: &Value) -> BTreeSet<u16> {
    let mut statuses = BTreeSet::new();
    for (_, item) in document["paths"]
        .as_object()
        .expect("OpenAPI paths should be an object")
    {
        for method in ["get", "post", "put", "patch", "delete"] {
            if let Some(operation) = item.get(method) {
                for status in operation["responses"]
                    .as_object()
                    .expect("operation responses should be an object")
                    .keys()
                {
                    statuses.insert(
                        status.parse::<u16>().unwrap_or_else(|_| {
                            panic!("response status should be numeric: {status}")
                        }),
                    );
                }
            }
        }
    }
    statuses
}

fn assert_required_fields(document: &Value, schema: &str, expected: &[&str]) {
    assert_eq!(
        string_set(&document["components"]["schemas"][schema]["required"]),
        expected.iter().map(|value| (*value).to_owned()).collect()
    );
}

fn assert_operation_statuses(document: &Value, path: &str, method: &str, expected: &[u16]) {
    let actual = document["paths"][path][method]["responses"]
        .as_object()
        .expect("operation responses should be an object")
        .keys()
        .map(|status| status.parse().expect("response status should be numeric"))
        .collect::<BTreeSet<u16>>();
    assert_eq!(
        actual,
        expected.iter().copied().collect(),
        "{method} {path}"
    );
}

fn string_set(value: &Value) -> BTreeSet<String> {
    value
        .as_array()
        .expect("value should be an array")
        .iter()
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .collect()
}

fn assert_operation_response(
    document: &Value,
    path: &str,
    method: &str,
    expected: StatusCode,
    response: &CapturedResponse,
) {
    assert_eq!(response.status, expected, "{method} {path}");
    let response_spec = resolve_ref(
        document,
        &document["paths"][path][method]["responses"][expected.as_u16().to_string()],
    );
    assert_response_headers_declared(response_spec, response);

    if expected == StatusCode::NO_CONTENT {
        assert!(
            response.body.is_empty(),
            "204 response body should be empty"
        );
        assert!(response.headers.get(CONTENT_TYPE).is_none());
        assert!(response_spec.get("content").is_none());
        return;
    }

    assert_json_cache_contract(response);
    let schema = &response_spec["content"]["application/json"]["schema"];
    assert!(
        !schema.is_null(),
        "{method} {path} {expected} needs a schema"
    );
    validate_schema(document, schema, &response.json(), "$")
        .unwrap_or_else(|error| panic!("{method} {path} {expected} schema mismatch: {error}"));
}

fn assert_response_headers_declared(response_spec: &Value, response: &CapturedResponse) {
    let headers = response_spec["headers"]
        .as_object()
        .expect("every response should declare headers");
    assert!(headers.contains_key("Cache-Control"));
    assert!(headers.contains_key("Pragma"));
    assert_eq!(response.headers.get(CACHE_CONTROL).unwrap(), "no-store");
    assert_eq!(response.headers.get(PRAGMA).unwrap(), "no-cache");
    if headers.contains_key("Location") {
        assert!(response.headers.get(LOCATION).is_some());
    }
}

fn assert_json_cache_contract(response: &CapturedResponse) {
    assert_eq!(response.headers.get(CACHE_CONTROL).unwrap(), "no-store");
    assert_eq!(response.headers.get(PRAGMA).unwrap(), "no-cache");
    assert_eq!(
        response.headers.get(CONTENT_TYPE).unwrap(),
        "application/json"
    );
    let _ = response.json();
}

fn resolve_ref<'a>(document: &'a Value, value: &'a Value) -> &'a Value {
    let Some(reference) = value.get("$ref").and_then(Value::as_str) else {
        return value;
    };
    let pointer = reference
        .strip_prefix('#')
        .expect("only local OpenAPI references are supported");
    document
        .pointer(pointer)
        .unwrap_or_else(|| panic!("OpenAPI reference should resolve: {reference}"))
}

fn validate_schema(
    document: &Value,
    schema: &Value,
    value: &Value,
    path: &str,
) -> Result<(), String> {
    let schema = resolve_ref(document, schema);

    if let Some(variants) = schema.get("anyOf").and_then(Value::as_array) {
        let errors = variants
            .iter()
            .filter_map(|variant| validate_schema(document, variant, value, path).err())
            .collect::<Vec<_>>();
        if errors.len() == variants.len() {
            return Err(format!("{path} did not match anyOf: {}", errors.join("; ")));
        }
        return Ok(());
    }

    if let Some(values) = schema.get("enum").and_then(Value::as_array)
        && !value.is_null()
        && !values.contains(value)
    {
        return Err(format!("{path} value {value} is outside enum {values:?}"));
    }

    let types = match schema.get("type") {
        Some(Value::String(kind)) => vec![kind.as_str()],
        Some(Value::Array(kinds)) => kinds.iter().filter_map(Value::as_str).collect(),
        Some(_) => return Err(format!("{path} schema type is invalid")),
        None => Vec::new(),
    };
    if value.is_null() {
        return if types.contains(&"null") {
            Ok(())
        } else {
            Err(format!("{path} is null but schema is not nullable"))
        };
    }

    let actual_type = if value.is_object() {
        "object"
    } else if value.is_array() {
        "array"
    } else if value.is_string() {
        "string"
    } else if value.is_boolean() {
        "boolean"
    } else if value.as_i64().is_some() || value.as_u64().is_some() {
        "integer"
    } else if value.is_number() {
        "number"
    } else {
        return Err(format!("{path} has an unsupported JSON type"));
    };
    if !types.is_empty() && !types.contains(&actual_type) {
        return Err(format!(
            "{path} has type {actual_type}, expected one of {types:?}"
        ));
    }

    match actual_type {
        "object" => {
            let object = value.as_object().expect("object type was checked");
            let properties = schema
                .get("properties")
                .and_then(Value::as_object)
                .cloned()
                .unwrap_or_default();
            if let Some(required) = schema.get("required").and_then(Value::as_array) {
                for field in required.iter().filter_map(Value::as_str) {
                    if !object.contains_key(field) {
                        return Err(format!("{path}.{field} is required"));
                    }
                }
            }
            for (field, field_value) in object {
                if let Some(field_schema) = properties.get(field) {
                    validate_schema(
                        document,
                        field_schema,
                        field_value,
                        &format!("{path}.{field}"),
                    )?;
                    continue;
                }
                match schema.get("additionalProperties") {
                    Some(Value::Bool(false)) => {
                        return Err(format!("{path}.{field} is not declared"));
                    }
                    Some(additional) if additional.is_object() => {
                        validate_schema(
                            document,
                            additional,
                            field_value,
                            &format!("{path}.{field}"),
                        )?;
                    }
                    _ => {}
                }
            }
        }
        "array" => {
            if let Some(items) = schema.get("items") {
                for (index, item) in value
                    .as_array()
                    .expect("array type was checked")
                    .iter()
                    .enumerate()
                {
                    validate_schema(document, items, item, &format!("{path}[{index}]"))?;
                }
            }
        }
        _ => {}
    }
    Ok(())
}

async fn mark_run_terminal(database: &DatabaseConnection, operation_id: &str) {
    let run = feed_refresh_run::Entity::find_by_id(operation_id)
        .one(database)
        .await
        .expect("refresh run should query")
        .expect("refresh run should exist");
    let mut run = run.into_active_model();
    run.status = Set("SUCCESS".to_owned());
    run.completed_at = Set(Some(datetime!(2026-07-17 12:00:00 UTC)));
    run.update(database)
        .await
        .expect("refresh run should become terminal");
}

async fn corrupt_run_status(database: &DatabaseConnection, operation_id: &str) {
    let run = feed_refresh_run::Entity::find_by_id(operation_id)
        .one(database)
        .await
        .expect("refresh run should query")
        .expect("refresh run should exist");
    let mut run = run.into_active_model();
    run.status = Set("INTERNAL_ONLY_STATUS".to_owned());
    run.update(database)
        .await
        .expect("refresh run should become corrupt for the error fixture");
}

fn quota_feed_model(id: String, url: String) -> feed::ActiveModel {
    let at = datetime!(2026-07-17 12:00:00 UTC);
    feed::ActiveModel {
        id: Set(id),
        source_url: Set(url.clone()),
        normalized_url: Set(url.clone()),
        normalized_url_hash: Set(blake3::hash(url.as_bytes()).to_hex().to_string()),
        fetch_url: Set(url),
        title: Set(Some("Quota feed".to_owned())),
        site_url: Set(None),
        validator_url: Set(None),
        etag: Set(None),
        last_modified: Set(None),
        response_content_hash: Set(None),
        entry_sequence_head: Set(0),
        last_attempt_at: Set(None),
        last_success_at: Set(None),
        last_changed_at: Set(None),
        next_fetch_at: Set(at + time::Duration::days(3_650)),
        retry_after_at: Set(None),
        consecutive_failures: Set(0),
        last_error_code: Set(None),
        is_disabled: Set(false),
        orphaned_at: Set(None),
        lease_owner: Set(None),
        lease_token: Set(0),
        lease_until: Set(None),
        created_at: Set(at),
        updated_at: Set(at),
    }
}

async fn seed_active_refresh_quota(database: &DatabaseConnection) {
    let at = datetime!(2026-07-17 12:00:00 UTC);
    for index in 0..20_u128 {
        let feed_id =
            Uuid::from_u128(0x9200_0000_0000_4000_8000_0000_0000_0000 + index).to_string();
        let run_id = Uuid::from_u128(0x9300_0000_0000_4000_8000_0000_0000_0000 + index).to_string();
        quota_feed_model(
            feed_id.clone(),
            format!("https://openapi-active-quota-{index:02}.example/rss.xml"),
        )
        .insert(database)
        .await
        .expect("active quota feed should insert");
        feed_refresh_run::ActiveModel {
            id: Set(run_id),
            feed_id: Set(feed_id),
            requested_by_user_id: Set(Some(USER_A_ID.to_owned())),
            trigger_kind: Set("MANUAL".to_owned()),
            status: Set("QUEUED".to_owned()),
            idempotency_key: Set(format!("openapi-active-quota-{index}")),
            lease_token: Set(None),
            commit_generation: Set(None),
            queued_at: Set(at + time::Duration::seconds(index as i64)),
            started_at: Set(None),
            fetched_at: Set(None),
            persisted_at: Set(None),
            completed_at: Set(None),
            http_status: Set(None),
            new_count: Set(0),
            updated_count: Set(0),
            dropped_count: Set(0),
            error_code: Set(None),
            retry_at: Set(None),
        }
        .insert(database)
        .await
        .expect("active quota run should insert");
    }
}
