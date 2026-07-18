#[allow(dead_code)]
mod support;

use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
};

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
        entities::{feed, feed_refresh_run, subscription},
        migrate,
    },
    setup::SetupService,
};
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ConnectionTrait, DatabaseConnection, DbBackend,
    EntityTrait, IntoActiveModel, Statement,
};
use secrecy::{ExposeSecret, SecretString};
use serde_json::{Value, json};
use tempfile::TempDir;
use time::{
    OffsetDateTime, UtcOffset,
    macros::{datetime, format_description},
};
use tower::ServiceExt;
use uuid::Uuid;

use support::database::{USER_A_ID, insert_user};

const OPENAPI_PATH: &str = "docs/openapi/subscription-v1.json";
const SUBSCRIPTION_PATH: &str = "/api/v1/subscriptions/{subscriptionId}";
const REFRESH_PATH: &str = "/api/v1/subscriptions/{subscriptionId}/refresh";
const PUBLIC_TIME_PATTERN: &str =
    r"^[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}\.[0-9]{6}Z$";
const LOCATION_PATTERN: &str = r"^/api/v1/subscriptions/[0-9a-f-]{36}$";
const HTTPS_FEED_URL_PATTERN: &str = r"^https://[^/?#@]+(?:[/?#].*)?$";
const OPENAPI_METHODS: [&str; 8] = [
    "get", "put", "post", "delete", "options", "head", "patch", "trace",
];

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

#[derive(Default)]
struct ObservedResponses {
    by_operation: BTreeMap<(String, String), BTreeSet<u16>>,
    scenarios: BTreeSet<(String, String, String, u16)>,
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
        ("PATCH".to_owned(), SUBSCRIPTION_PATH.to_owned()),
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
        "patch",
        &[200, 401, 403, 404, 422, 429, 500],
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
        &[200, 202, 401, 403, 409, 422, 429, 500],
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
            "categoryId",
            "titleOverride",
            "position",
            "title",
            "siteUrl",
            "unreadCount",
            "refresh",
        ],
    );
    assert_required_fields(&document, "CreateSubscriptionRequest", &["url"]);
    assert_eq!(
        document["components"]["schemas"]["UpdateSubscriptionRequest"]["minProperties"],
        1
    );
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

#[test]
fn subscription_openapi_schema_manifest_and_local_refs_are_frozen() {
    let document = load_openapi();
    assert_all_local_refs_resolve(&document);
    assert_eq!(
        project_operation_contracts(&document),
        frozen_operation_contracts()
    );
    assert_eq!(
        project_response_contracts(&document),
        frozen_response_contracts()
    );
    assert_eq!(
        project_parameter_contracts(&document),
        frozen_parameter_contracts()
    );
    assert_eq!(
        project_header_contracts(&document),
        frozen_header_contracts()
    );

    let schemas = document["components"]["schemas"]
        .as_object()
        .expect("OpenAPI schemas should be an object");
    let actual = schemas
        .iter()
        .map(|(name, schema)| (name.clone(), project_schema_shape(schema)))
        .collect::<serde_json::Map<_, _>>();
    assert_eq!(Value::Object(actual), frozen_schema_manifest());

    assert_request_schema_accepts(
        &document,
        "/api/v1/subscriptions",
        "post",
        &json!({ "url": "https://request.example/rss.xml" }),
    );
    let overlong_url = format!("https://request.example/{}", "a".repeat(4_096));
    for invalid in [
        json!({ "url": "https://request.example/rss.xml", "providerDetails": true }),
        json!({ "url": 7 }),
        json!({ "url": "http://request.example/rss.xml" }),
        json!({ "url": "https://user:password@request.example/rss.xml" }),
        json!({ "url": overlong_url }),
    ] {
        assert_request_schema_rejects(&document, "/api/v1/subscriptions", "post", &invalid);
    }

    for valid in [
        json!({ "categoryId": null }),
        json!({ "titleOverride": null }),
        json!({ "position": 0 }),
        json!({
            "categoryId": "00000000-0000-4000-8000-000000000501",
            "titleOverride": "Technology",
            "position": 1024
        }),
    ] {
        assert_request_schema_accepts(&document, SUBSCRIPTION_PATH, "patch", &valid);
    }
    for invalid in [
        json!({}),
        json!({ "position": null }),
        json!({ "position": -1 }),
        json!({ "position": "0" }),
        json!({ "categoryId": "not-a-uuid" }),
        json!({ "titleOverride": 7 }),
        json!({ "titleOverride": "a".repeat(201) }),
        json!({ "position": 0, "providerDetails": true }),
    ] {
        assert_request_schema_rejects(&document, SUBSCRIPTION_PATH, "patch", &invalid);
    }

    assert_request_schema_accepts(
        &document,
        REFRESH_PATH,
        "post",
        &json!({ "requestId": "00000000-0000-4000-8000-000000000799" }),
    );
    for invalid in [
        json!({
            "requestId": "00000000-0000-4000-8000-000000000799",
            "providerDetails": true
        }),
        json!({ "requestId": 7 }),
    ] {
        assert_request_schema_rejects(&document, REFRESH_PATH, "post", &invalid);
    }

    assert!(is_public_time("2026-07-18T01:02:03.123456Z"));
    for invalid in [
        "2026-07-18T01:02:03.12345Z",
        "2026-07-18T01:02:03.123456+00:00",
        "2026-02-30T01:02:03.123456Z",
        "2026-07-18T24:02:03.123456Z",
    ] {
        assert!(!is_public_time(invalid), "invalid public time: {invalid}");
    }
}

#[tokio::test]
async fn subscription_openapi_matches_real_router_responses() {
    const REQUEST_A: &str = "00000000-0000-4000-8000-000000000701";
    const REQUEST_B: &str = "00000000-0000-4000-8000-000000000702";

    let document = load_openapi();
    let mut covered_statuses = ObservedResponses::default();
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
    record_observed(
        &mut covered_statuses,
        "/api/v1/subscriptions",
        "get",
        "/api/v1/subscriptions",
        "list_success",
        list.status,
    );

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
    record_observed(
        &mut covered_statuses,
        "/api/v1/subscriptions",
        "post",
        "/api/v1/subscriptions",
        "create_new",
        created.status,
    );

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
    record_observed(
        &mut covered_statuses,
        "/api/v1/subscriptions",
        "post",
        "/api/v1/subscriptions",
        "create_existing",
        existing.status,
    );

    let detail_uri = format!("/api/v1/subscriptions/{subscription_id}");
    let detail = fixture
        .request(Method::GET, &detail_uri, None, true, true)
        .await;
    assert_operation_response(&document, SUBSCRIPTION_PATH, "get", StatusCode::OK, &detail);
    record_observed(
        &mut covered_statuses,
        SUBSCRIPTION_PATH,
        "get",
        &detail_uri,
        "detail_success",
        detail.status,
    );

    let patched = fixture
        .request(
            Method::PATCH,
            &detail_uri,
            Some(json!({
                "titleOverride": "OpenAPI title",
                "position": 512
            })),
            true,
            true,
        )
        .await;
    assert_and_record(
        &document,
        &mut covered_statuses,
        SUBSCRIPTION_PATH,
        "patch",
        &detail_uri,
        "patch_success",
        StatusCode::OK,
        &patched,
    );
    assert_eq!(patched.json()["titleOverride"], "OpenAPI title");
    assert_eq!(patched.json()["position"], 512);

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
    record_observed(
        &mut covered_statuses,
        REFRESH_PATH,
        "post",
        &refresh_uri,
        "refresh_accepted",
        accepted.status,
    );

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
    record_observed(
        &mut covered_statuses,
        REFRESH_PATH,
        "post",
        &refresh_uri,
        "refresh_active_conflict",
        conflict.status,
    );

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
    record_observed(
        &mut covered_statuses,
        REFRESH_PATH,
        "post",
        &refresh_uri,
        "refresh_terminal_replay",
        replay.status,
    );

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
    record_observed(
        &mut covered_statuses,
        SUBSCRIPTION_PATH,
        "delete",
        &detail_uri,
        "delete_success",
        deleted.status,
    );

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
    record_observed(
        &mut covered_statuses,
        "/api/v1/subscriptions",
        "get",
        "/api/v1/subscriptions",
        "list_unauthenticated",
        unauthorized.status,
    );

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
    record_observed(
        &mut covered_statuses,
        "/api/v1/subscriptions",
        "post",
        "/api/v1/subscriptions",
        "create_forbidden",
        forbidden.status,
    );

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
    record_observed(
        &mut covered_statuses,
        SUBSCRIPTION_PATH,
        "get",
        "/api/v1/subscriptions/00000000-0000-4000-8000-000000000299",
        "detail_missing",
        missing.status,
    );

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
    record_observed(
        &mut covered_statuses,
        "/api/v1/subscriptions",
        "get",
        "/api/v1/subscriptions?limit=0",
        "list_invalid_limit",
        invalid.status,
    );

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
    record_observed(
        &mut covered_statuses,
        "/api/v1/subscriptions",
        "post",
        "/api/v1/subscriptions",
        "memory_limiter",
        temporal_limit.status,
    );

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
    record_observed(
        &mut covered_statuses,
        REFRESH_PATH,
        "post",
        &format!("/api/v1/subscriptions/{hard_subscription_id}/refresh"),
        "active_refresh_limit",
        hard_limit.status,
    );

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
    record_observed(
        &mut covered_statuses,
        SUBSCRIPTION_PATH,
        "get",
        &format!("/api/v1/subscriptions/{internal_subscription_id}"),
        "detail_internal_error",
        internal.status,
    );

    let list_internal_fixture = ContractFixture::new().await;
    let list_internal_created = create_subscription(
        &list_internal_fixture,
        "https://list-internal.example/rss.xml",
    )
    .await;
    corrupt_run_status(
        &list_internal_fixture.database,
        &list_internal_created.operation_id,
    )
    .await;
    let list_internal = list_internal_fixture
        .request(Method::GET, "/api/v1/subscriptions", None, true, true)
        .await;
    assert_and_record(
        &document,
        &mut covered_statuses,
        "/api/v1/subscriptions",
        "get",
        "/api/v1/subscriptions",
        "list_internal_error",
        StatusCode::INTERNAL_SERVER_ERROR,
        &list_internal,
    );

    let create_unauthorized = fixture
        .request(
            Method::POST,
            "/api/v1/subscriptions",
            Some(json!({ "url": "https://unauthorized.example/rss.xml" })),
            false,
            false,
        )
        .await;
    assert_and_record(
        &document,
        &mut covered_statuses,
        "/api/v1/subscriptions",
        "post",
        "/api/v1/subscriptions",
        "create_unauthenticated",
        StatusCode::UNAUTHORIZED,
        &create_unauthorized,
    );

    let overlong_url = format!("https://invalid.example/{}", "a".repeat(4_096));
    for (scenario, body) in [
        (
            "create_unknown_field",
            json!({ "url": "https://invalid.example/rss.xml", "providerDetails": true }),
        ),
        ("create_wrong_type", json!({ "url": 7 })),
        (
            "create_http_url",
            json!({ "url": "http://invalid.example/rss.xml" }),
        ),
        (
            "create_credential_url",
            json!({ "url": "https://user:password@invalid.example/rss.xml" }),
        ),
        ("create_overlong_url", json!({ "url": overlong_url })),
    ] {
        assert_request_schema_rejects(&document, "/api/v1/subscriptions", "post", &body);
        let invalid_create = fixture
            .request(
                Method::POST,
                "/api/v1/subscriptions",
                Some(body),
                true,
                true,
            )
            .await;
        assert_and_record(
            &document,
            &mut covered_statuses,
            "/api/v1/subscriptions",
            "post",
            "/api/v1/subscriptions",
            scenario,
            StatusCode::UNPROCESSABLE_ENTITY,
            &invalid_create,
        );
    }

    let create_internal_fixture = ContractFixture::new().await;
    drop_table(&create_internal_fixture.database, "feeds").await;
    let create_internal = create_internal_fixture
        .request(
            Method::POST,
            "/api/v1/subscriptions",
            Some(json!({ "url": "https://create-internal.example/rss.xml" })),
            true,
            true,
        )
        .await;
    assert_and_record(
        &document,
        &mut covered_statuses,
        "/api/v1/subscriptions",
        "post",
        "/api/v1/subscriptions",
        "create_internal_error",
        StatusCode::INTERNAL_SERVER_ERROR,
        &create_internal,
    );

    let subscription_limit_fixture = ContractFixture::new().await;
    seed_subscription_quota(&subscription_limit_fixture.database).await;
    let subscription_limit = subscription_limit_fixture
        .request(
            Method::POST,
            "/api/v1/subscriptions",
            Some(json!({ "url": "https://subscription-limit.example/rss.xml" })),
            true,
            true,
        )
        .await;
    assert_and_record(
        &document,
        &mut covered_statuses,
        "/api/v1/subscriptions",
        "post",
        "/api/v1/subscriptions",
        "subscription_limit",
        StatusCode::TOO_MANY_REQUESTS,
        &subscription_limit,
    );
    assert_rate_limited_without_retry(&subscription_limit);

    let detail_unauthorized = fixture
        .request(
            Method::GET,
            "/api/v1/subscriptions/00000000-0000-4000-8000-000000000299",
            None,
            false,
            false,
        )
        .await;
    assert_and_record(
        &document,
        &mut covered_statuses,
        SUBSCRIPTION_PATH,
        "get",
        "/api/v1/subscriptions/00000000-0000-4000-8000-000000000299",
        "detail_unauthenticated",
        StatusCode::UNAUTHORIZED,
        &detail_unauthorized,
    );
    let detail_invalid = fixture
        .request(
            Method::GET,
            "/api/v1/subscriptions/not-a-uuid",
            None,
            true,
            true,
        )
        .await;
    assert_and_record(
        &document,
        &mut covered_statuses,
        SUBSCRIPTION_PATH,
        "get",
        "/api/v1/subscriptions/not-a-uuid",
        "detail_invalid_id",
        StatusCode::UNPROCESSABLE_ENTITY,
        &detail_invalid,
    );

    let patch_path = "/api/v1/subscriptions/00000000-0000-4000-8000-000000000299";
    let patch_body = json!({ "position": 1024 });
    let patch_unauthorized = fixture
        .request(
            Method::PATCH,
            patch_path,
            Some(patch_body.clone()),
            false,
            false,
        )
        .await;
    assert_and_record(
        &document,
        &mut covered_statuses,
        SUBSCRIPTION_PATH,
        "patch",
        patch_path,
        "patch_unauthenticated",
        StatusCode::UNAUTHORIZED,
        &patch_unauthorized,
    );
    let patch_forbidden = fixture
        .request(
            Method::PATCH,
            patch_path,
            Some(patch_body.clone()),
            true,
            false,
        )
        .await;
    assert_and_record(
        &document,
        &mut covered_statuses,
        SUBSCRIPTION_PATH,
        "patch",
        patch_path,
        "patch_forbidden",
        StatusCode::FORBIDDEN,
        &patch_forbidden,
    );
    let patch_missing = fixture
        .request(
            Method::PATCH,
            patch_path,
            Some(patch_body.clone()),
            true,
            true,
        )
        .await;
    assert_and_record(
        &document,
        &mut covered_statuses,
        SUBSCRIPTION_PATH,
        "patch",
        patch_path,
        "patch_missing",
        StatusCode::NOT_FOUND,
        &patch_missing,
    );
    let invalid_patch_body = json!({});
    assert_request_schema_rejects(&document, SUBSCRIPTION_PATH, "patch", &invalid_patch_body);
    let patch_invalid = fixture
        .request(
            Method::PATCH,
            patch_path,
            Some(invalid_patch_body),
            true,
            true,
        )
        .await;
    assert_and_record(
        &document,
        &mut covered_statuses,
        SUBSCRIPTION_PATH,
        "patch",
        patch_path,
        "patch_empty_body",
        StatusCode::UNPROCESSABLE_ENTITY,
        &patch_invalid,
    );

    let patch_internal_fixture = ContractFixture::new().await;
    drop_table(&patch_internal_fixture.database, "subscriptions").await;
    let patch_internal = patch_internal_fixture
        .request(
            Method::PATCH,
            patch_path,
            Some(patch_body.clone()),
            true,
            true,
        )
        .await;
    assert_and_record(
        &document,
        &mut covered_statuses,
        SUBSCRIPTION_PATH,
        "patch",
        patch_path,
        "patch_internal_error",
        StatusCode::INTERNAL_SERVER_ERROR,
        &patch_internal,
    );

    let delete_unauthorized = fixture
        .request(
            Method::DELETE,
            "/api/v1/subscriptions/00000000-0000-4000-8000-000000000299",
            None,
            false,
            false,
        )
        .await;
    assert_and_record(
        &document,
        &mut covered_statuses,
        SUBSCRIPTION_PATH,
        "delete",
        "/api/v1/subscriptions/00000000-0000-4000-8000-000000000299",
        "delete_unauthenticated",
        StatusCode::UNAUTHORIZED,
        &delete_unauthorized,
    );
    let delete_forbidden = fixture
        .request(
            Method::DELETE,
            "/api/v1/subscriptions/00000000-0000-4000-8000-000000000299",
            None,
            true,
            false,
        )
        .await;
    assert_and_record(
        &document,
        &mut covered_statuses,
        SUBSCRIPTION_PATH,
        "delete",
        "/api/v1/subscriptions/00000000-0000-4000-8000-000000000299",
        "delete_forbidden",
        StatusCode::FORBIDDEN,
        &delete_forbidden,
    );
    let delete_invalid = fixture
        .request(
            Method::DELETE,
            "/api/v1/subscriptions/not-a-uuid",
            None,
            true,
            true,
        )
        .await;
    assert_and_record(
        &document,
        &mut covered_statuses,
        SUBSCRIPTION_PATH,
        "delete",
        "/api/v1/subscriptions/not-a-uuid",
        "delete_invalid_id",
        StatusCode::UNPROCESSABLE_ENTITY,
        &delete_invalid,
    );

    let delete_limiter_fixture = ContractFixture::new().await;
    exhaust_mutation_limit(
        &delete_limiter_fixture,
        "https://delete-limit.example/rss.xml",
    )
    .await;
    let delete_limited = delete_limiter_fixture
        .request(
            Method::DELETE,
            "/api/v1/subscriptions/00000000-0000-4000-8000-000000000299",
            None,
            true,
            true,
        )
        .await;
    assert_and_record(
        &document,
        &mut covered_statuses,
        SUBSCRIPTION_PATH,
        "delete",
        "/api/v1/subscriptions/00000000-0000-4000-8000-000000000299",
        "delete_memory_limiter",
        StatusCode::TOO_MANY_REQUESTS,
        &delete_limited,
    );
    assert_rate_limited_with_retry(&delete_limited, None);

    let patch_limited = delete_limiter_fixture
        .request(
            Method::PATCH,
            "/api/v1/subscriptions/00000000-0000-4000-8000-000000000299",
            Some(json!({ "position": 1024 })),
            true,
            true,
        )
        .await;
    assert_and_record(
        &document,
        &mut covered_statuses,
        SUBSCRIPTION_PATH,
        "patch",
        "/api/v1/subscriptions/00000000-0000-4000-8000-000000000299",
        "patch_memory_limiter",
        StatusCode::TOO_MANY_REQUESTS,
        &patch_limited,
    );
    assert_rate_limited_with_retry(&patch_limited, None);

    let delete_internal_fixture = ContractFixture::new().await;
    drop_table(&delete_internal_fixture.database, "subscriptions").await;
    let delete_internal = delete_internal_fixture
        .request(
            Method::DELETE,
            "/api/v1/subscriptions/00000000-0000-4000-8000-000000000299",
            None,
            true,
            true,
        )
        .await;
    assert_and_record(
        &document,
        &mut covered_statuses,
        SUBSCRIPTION_PATH,
        "delete",
        "/api/v1/subscriptions/00000000-0000-4000-8000-000000000299",
        "delete_internal_error",
        StatusCode::INTERNAL_SERVER_ERROR,
        &delete_internal,
    );

    let refresh_error_path = "/api/v1/subscriptions/00000000-0000-4000-8000-000000000299/refresh";
    let refresh_request = json!({ "requestId": "00000000-0000-4000-8000-000000000704" });
    let refresh_unauthorized = fixture
        .request(
            Method::POST,
            refresh_error_path,
            Some(refresh_request.clone()),
            false,
            false,
        )
        .await;
    assert_and_record(
        &document,
        &mut covered_statuses,
        REFRESH_PATH,
        "post",
        refresh_error_path,
        "refresh_unauthenticated",
        StatusCode::UNAUTHORIZED,
        &refresh_unauthorized,
    );
    let refresh_forbidden = fixture
        .request(
            Method::POST,
            refresh_error_path,
            Some(refresh_request.clone()),
            true,
            false,
        )
        .await;
    assert_and_record(
        &document,
        &mut covered_statuses,
        REFRESH_PATH,
        "post",
        refresh_error_path,
        "refresh_forbidden",
        StatusCode::FORBIDDEN,
        &refresh_forbidden,
    );
    let refresh_missing = fixture
        .request(
            Method::POST,
            refresh_error_path,
            Some(refresh_request),
            true,
            true,
        )
        .await;
    assert_and_record(
        &document,
        &mut covered_statuses,
        REFRESH_PATH,
        "post",
        refresh_error_path,
        "refresh_missing",
        StatusCode::UNPROCESSABLE_ENTITY,
        &refresh_missing,
    );

    for (scenario, body) in [
        (
            "refresh_unknown_field",
            json!({
                "requestId": "00000000-0000-4000-8000-000000000705",
                "providerDetails": true
            }),
        ),
        ("refresh_wrong_type", json!({ "requestId": 7 })),
    ] {
        assert_request_schema_rejects(&document, REFRESH_PATH, "post", &body);
        let invalid_refresh = fixture
            .request(Method::POST, refresh_error_path, Some(body), true, true)
            .await;
        assert_and_record(
            &document,
            &mut covered_statuses,
            REFRESH_PATH,
            "post",
            refresh_error_path,
            scenario,
            StatusCode::UNPROCESSABLE_ENTITY,
            &invalid_refresh,
        );
    }

    let refresh_internal_fixture = ContractFixture::new().await;
    let refresh_internal_created = create_subscription(
        &refresh_internal_fixture,
        "https://refresh-internal.example/rss.xml",
    )
    .await;
    drop_table(&refresh_internal_fixture.database, "feed_refresh_runs").await;
    let refresh_internal_path = format!(
        "/api/v1/subscriptions/{}/refresh",
        refresh_internal_created.subscription_id
    );
    let refresh_internal = refresh_internal_fixture
        .request(
            Method::POST,
            &refresh_internal_path,
            Some(json!({ "requestId": "00000000-0000-4000-8000-000000000706" })),
            true,
            true,
        )
        .await;
    assert_and_record(
        &document,
        &mut covered_statuses,
        REFRESH_PATH,
        "post",
        &refresh_internal_path,
        "refresh_internal_error",
        StatusCode::INTERNAL_SERVER_ERROR,
        &refresh_internal,
    );

    let cooldown_fixture = ContractFixture::new().await;
    let cooldown_created = create_subscription(
        &cooldown_fixture,
        "https://cooldown-contract.example/rss.xml",
    )
    .await;
    mark_run_terminal(&cooldown_fixture.database, &cooldown_created.operation_id).await;
    let database_now = sqlite_database_now(&cooldown_fixture.database).await;
    let retry_at = database_now + time::Duration::seconds(60) + time::Duration::milliseconds(900);
    set_feed_retry_after(
        &cooldown_fixture.database,
        &cooldown_created.feed_id,
        retry_at,
    )
    .await;
    let cooldown_path = format!(
        "/api/v1/subscriptions/{}/refresh",
        cooldown_created.subscription_id
    );
    let cooldown = cooldown_fixture
        .request(
            Method::POST,
            &cooldown_path,
            Some(json!({ "requestId": "00000000-0000-4000-8000-000000000707" })),
            true,
            true,
        )
        .await;
    assert_and_record(
        &document,
        &mut covered_statuses,
        REFRESH_PATH,
        "post",
        &cooldown_path,
        "repository_cooldown",
        StatusCode::TOO_MANY_REQUESTS,
        &cooldown,
    );
    assert_rate_limited_with_retry(&cooldown, Some(public_time(retry_at).as_str()));

    assert_rate_limited_with_retry(&temporal_limit, None);
    assert_rate_limited_without_retry(&hard_limit);

    let rate_limit_scenarios = covered_statuses
        .scenarios
        .iter()
        .filter(|(_, _, _, status)| *status == StatusCode::TOO_MANY_REQUESTS.as_u16())
        .map(|(_, _, scenario, _)| scenario.clone())
        .collect::<BTreeSet<_>>();
    assert_eq!(
        rate_limit_scenarios,
        BTreeSet::from([
            "active_refresh_limit".to_owned(),
            "delete_memory_limiter".to_owned(),
            "memory_limiter".to_owned(),
            "patch_memory_limiter".to_owned(),
            "repository_cooldown".to_owned(),
            "subscription_limit".to_owned(),
        ])
    );

    assert_eq!(
        covered_statuses.by_operation,
        documented_operation_statuses(&document)
    );

    for (method, uri, expected) in [
        (Method::GET, "/api/v1/subscriptions/", StatusCode::NOT_FOUND),
        (
            Method::GET,
            "/api/v1/subscriptions/not-a-real-route/extra",
            StatusCode::NOT_FOUND,
        ),
        (
            Method::PUT,
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

fn frozen_schema_manifest() -> Value {
    json!({
        "CreateSubscriptionRequest": {
            "type": "object",
            "additionalProperties": false,
            "required": ["url"],
            "properties": {
                "url": {
                    "type": "string",
                    "format": "uri",
                    "pattern": HTTPS_FEED_URL_PATTERN,
                    "maxLength": 4096
                }
            }
        },
        "UpdateSubscriptionRequest": {
            "type": "object",
            "additionalProperties": false,
            "minProperties": 1,
            "properties": {
                "categoryId": {
                    "type": ["string", "null"],
                    "format": "uuid"
                },
                "titleOverride": {
                    "type": ["string", "null"],
                    "maxLength": 200
                },
                "position": {
                    "type": "integer",
                    "minimum": 0
                }
            }
        },
        "RefreshSubscriptionRequest": {
            "type": "object",
            "additionalProperties": false,
            "required": ["requestId"],
            "properties": {
                "requestId": { "type": "string", "format": "uuid" }
            }
        },
        "SubscriptionPage": {
            "type": "object",
            "additionalProperties": false,
            "required": ["items", "nextCursor"],
            "properties": {
                "items": {
                    "type": "array",
                    "items": { "$ref": "#/components/schemas/Subscription" }
                },
                "nextCursor": { "type": ["string", "null"] }
            }
        },
        "CreateSubscriptionResponse": {
            "type": "object",
            "additionalProperties": false,
            "required": ["created", "subscription"],
            "properties": {
                "created": { "type": "boolean" },
                "subscription": { "$ref": "#/components/schemas/Subscription" }
            }
        },
        "Subscription": {
            "type": "object",
            "additionalProperties": false,
            "required": [
                "subscriptionId",
                "feedId",
                "categoryId",
                "titleOverride",
                "position",
                "title",
                "siteUrl",
                "unreadCount",
                "refresh"
            ],
            "properties": {
                "subscriptionId": { "type": "string", "format": "uuid" },
                "feedId": { "type": "string", "format": "uuid" },
                "categoryId": { "type": ["string", "null"], "format": "uuid" },
                "titleOverride": { "type": ["string", "null"], "maxLength": 200 },
                "position": { "type": "integer", "minimum": 0 },
                "title": { "type": "string" },
                "siteUrl": { "type": ["string", "null"], "format": "uri" },
                "unreadCount": { "type": "integer", "minimum": 0 },
                "refresh": {
                    "anyOf": [
                        { "$ref": "#/components/schemas/Refresh" },
                        { "type": "null" }
                    ]
                }
            }
        },
        "Refresh": {
            "type": "object",
            "additionalProperties": false,
            "required": [
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
                "completedAt"
            ],
            "properties": {
                "operationId": { "type": "string", "format": "uuid" },
                "state": {
                    "type": "string",
                    "enum": ["PENDING", "READY", "DEGRADED", "BACKING_OFF", "ERROR"]
                },
                "newCount": { "type": "integer", "minimum": 0 },
                "updatedCount": { "type": "integer", "minimum": 0 },
                "droppedCount": { "type": "integer", "minimum": 0 },
                "generation": { "type": ["integer", "null"], "minimum": 0 },
                "errorCode": {
                    "type": ["string", "null"],
                    "enum": ["REFRESH_FAILED", "UPSTREAM_RATE_LIMITED", null]
                },
                "retryAt": {
                    "type": ["string", "null"],
                    "format": "date-time",
                    "pattern": PUBLIC_TIME_PATTERN
                },
                "queuedAt": {
                    "type": "string",
                    "format": "date-time",
                    "pattern": PUBLIC_TIME_PATTERN
                },
                "startedAt": {
                    "type": ["string", "null"],
                    "format": "date-time",
                    "pattern": PUBLIC_TIME_PATTERN
                },
                "completedAt": {
                    "type": ["string", "null"],
                    "format": "date-time",
                    "pattern": PUBLIC_TIME_PATTERN
                }
            }
        },
        "ErrorEnvelope": {
            "type": "object",
            "additionalProperties": false,
            "required": ["error"],
            "properties": {
                "error": { "$ref": "#/components/schemas/ApiError" }
            }
        },
        "ApiError": {
            "type": "object",
            "additionalProperties": false,
            "required": ["code", "message", "requestId"],
            "properties": {
                "code": {
                    "type": "string",
                    "enum": [
                        "AUTHENTICATION_REQUIRED",
                        "FORBIDDEN",
                        "VALIDATION_ERROR",
                        "NOT_FOUND",
                        "METHOD_NOT_ALLOWED",
                        "CONFLICT",
                        "REFRESH_IN_PROGRESS",
                        "RATE_LIMITED",
                        "INTERNAL_ERROR"
                    ]
                },
                "message": { "type": "string" },
                "fields": {
                    "type": "object",
                    "additionalProperties": { "type": "string" }
                },
                "requestId": { "type": "string", "format": "uuid" }
            }
        }
    })
}

fn frozen_operation_contracts() -> Value {
    json!({
        "delete /api/v1/subscriptions/{subscriptionId}": {
            "parameters": [
                "#/components/parameters/SubscriptionId",
                "#/components/parameters/CsrfToken"
            ],
            "requestSchema": null,
            "responses": {
                "204": "#/components/responses/NoContent",
                "401": "#/components/responses/Error",
                "403": "#/components/responses/Error",
                "422": "#/components/responses/Error",
                "429": "#/components/responses/RateLimited",
                "500": "#/components/responses/Error"
            }
        },
        "get /api/v1/subscriptions": {
            "parameters": [
                "#/components/parameters/Cursor",
                "#/components/parameters/Limit"
            ],
            "requestSchema": null,
            "responses": {
                "200": "#/components/responses/SubscriptionPage",
                "401": "#/components/responses/Error",
                "422": "#/components/responses/Error",
                "500": "#/components/responses/Error"
            }
        },
        "get /api/v1/subscriptions/{subscriptionId}": {
            "parameters": ["#/components/parameters/SubscriptionId"],
            "requestSchema": null,
            "responses": {
                "200": "#/components/responses/Subscription",
                "401": "#/components/responses/Error",
                "404": "#/components/responses/Error",
                "422": "#/components/responses/Error",
                "500": "#/components/responses/Error"
            }
        },
        "patch /api/v1/subscriptions/{subscriptionId}": {
            "parameters": [
                "#/components/parameters/SubscriptionId",
                "#/components/parameters/CsrfToken"
            ],
            "requestSchema": "#/components/schemas/UpdateSubscriptionRequest",
            "responses": {
                "200": "#/components/responses/Subscription",
                "401": "#/components/responses/Error",
                "403": "#/components/responses/Error",
                "404": "#/components/responses/Error",
                "422": "#/components/responses/Error",
                "429": "#/components/responses/RateLimited",
                "500": "#/components/responses/Error"
            }
        },
        "post /api/v1/subscriptions": {
            "parameters": ["#/components/parameters/CsrfToken"],
            "requestSchema": "#/components/schemas/CreateSubscriptionRequest",
            "responses": {
                "200": "#/components/responses/CreateSubscription",
                "201": "#/components/responses/CreateSubscription",
                "401": "#/components/responses/Error",
                "403": "#/components/responses/Error",
                "422": "#/components/responses/Error",
                "429": "#/components/responses/RateLimited",
                "500": "#/components/responses/Error"
            }
        },
        "post /api/v1/subscriptions/{subscriptionId}/refresh": {
            "parameters": [
                "#/components/parameters/SubscriptionId",
                "#/components/parameters/CsrfToken"
            ],
            "requestSchema": "#/components/schemas/RefreshSubscriptionRequest",
            "responses": {
                "200": "#/components/responses/Refresh",
                "202": "#/components/responses/Refresh",
                "401": "#/components/responses/Error",
                "403": "#/components/responses/Error",
                "409": "#/components/responses/Error",
                "422": "#/components/responses/Error",
                "429": "#/components/responses/RateLimited",
                "500": "#/components/responses/Error"
            }
        }
    })
}

fn frozen_response_contracts() -> Value {
    json!({
        "SubscriptionPage": {
            "headers": {
                "Cache-Control": "#/components/headers/CacheControl",
                "Pragma": "#/components/headers/Pragma"
            },
            "schema": "#/components/schemas/SubscriptionPage"
        },
        "Subscription": {
            "headers": {
                "Cache-Control": "#/components/headers/CacheControl",
                "Pragma": "#/components/headers/Pragma"
            },
            "schema": "#/components/schemas/Subscription"
        },
        "CreateSubscription": {
            "headers": {
                "Cache-Control": "#/components/headers/CacheControl",
                "Pragma": "#/components/headers/Pragma",
                "Location": "#/components/headers/Location"
            },
            "schema": "#/components/schemas/CreateSubscriptionResponse"
        },
        "Refresh": {
            "headers": {
                "Cache-Control": "#/components/headers/CacheControl",
                "Pragma": "#/components/headers/Pragma"
            },
            "schema": "#/components/schemas/Refresh"
        },
        "NoContent": {
            "headers": {
                "Cache-Control": "#/components/headers/CacheControl",
                "Pragma": "#/components/headers/Pragma"
            },
            "schema": null
        },
        "Error": {
            "headers": {
                "Cache-Control": "#/components/headers/CacheControl",
                "Pragma": "#/components/headers/Pragma"
            },
            "schema": "#/components/schemas/ErrorEnvelope"
        },
        "RateLimited": {
            "headers": {
                "Cache-Control": "#/components/headers/CacheControl",
                "Pragma": "#/components/headers/Pragma",
                "Retry-After": { "type": "integer", "minimum": 1 }
            },
            "schema": "#/components/schemas/ErrorEnvelope"
        }
    })
}

fn frozen_parameter_contracts() -> Value {
    json!({
        "Cursor": {
            "name": "cursor",
            "in": "query",
            "required": false,
            "schema": { "type": "string" }
        },
        "Limit": {
            "name": "limit",
            "in": "query",
            "required": false,
            "schema": {
                "type": "integer",
                "minimum": 1,
                "maximum": 100,
                "default": 50
            }
        },
        "SubscriptionId": {
            "name": "subscriptionId",
            "in": "path",
            "required": true,
            "schema": { "type": "string", "format": "uuid" }
        },
        "CsrfToken": {
            "name": "x-csrf-token",
            "in": "header",
            "required": true,
            "schema": { "type": "string" }
        }
    })
}

fn frozen_header_contracts() -> Value {
    json!({
        "CacheControl": {
            "type": "string",
            "enum": ["no-store"]
        },
        "Pragma": {
            "type": "string",
            "enum": ["no-cache"]
        },
        "Location": {
            "type": "string",
            "pattern": LOCATION_PATTERN
        },
        "RetryAfter": {
            "type": "integer",
            "minimum": 1
        }
    })
}

fn project_operation_contracts(document: &Value) -> Value {
    let mut contracts = serde_json::Map::new();
    for (path, item) in document["paths"]
        .as_object()
        .expect("OpenAPI paths should be an object")
    {
        for method in OPENAPI_METHODS {
            let Some(operation) = item.get(method) else {
                continue;
            };
            let parameters = item
                .get("parameters")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .chain(
                    operation
                        .get("parameters")
                        .and_then(Value::as_array)
                        .into_iter()
                        .flatten(),
                )
                .map(|parameter| {
                    Value::String(
                        parameter["$ref"]
                            .as_str()
                            .expect("frozen operation parameter should use a local ref")
                            .to_owned(),
                    )
                })
                .collect();
            let request_schema = operation
                .pointer("/requestBody/content/application~1json/schema/$ref")
                .cloned()
                .unwrap_or(Value::Null);
            let responses = operation["responses"]
                .as_object()
                .expect("operation responses should be an object")
                .iter()
                .map(|(status, response)| {
                    (
                        status.clone(),
                        Value::String(
                            response["$ref"]
                                .as_str()
                                .expect("frozen operation response should use a local ref")
                                .to_owned(),
                        ),
                    )
                })
                .collect();
            contracts.insert(
                format!("{method} {path}"),
                json!({
                    "parameters": Value::Array(parameters),
                    "requestSchema": request_schema,
                    "responses": Value::Object(responses),
                }),
            );
        }
    }
    Value::Object(contracts)
}

fn project_response_contracts(document: &Value) -> Value {
    let contracts = document["components"]["responses"]
        .as_object()
        .expect("OpenAPI response components should be an object")
        .iter()
        .map(|(name, response)| {
            let headers = response["headers"]
                .as_object()
                .expect("response component headers should be an object")
                .iter()
                .map(|(header_name, header)| {
                    let target = header.get("$ref").cloned().unwrap_or_else(|| {
                        header
                            .get("schema")
                            .cloned()
                            .expect("inline response header should declare a schema")
                    });
                    (header_name.clone(), target)
                })
                .collect();
            let schema = response
                .pointer("/content/application~1json/schema/$ref")
                .cloned()
                .unwrap_or(Value::Null);
            (
                name.clone(),
                json!({
                    "headers": Value::Object(headers),
                    "schema": schema,
                }),
            )
        })
        .collect();
    Value::Object(contracts)
}

fn project_parameter_contracts(document: &Value) -> Value {
    let contracts = document["components"]["parameters"]
        .as_object()
        .expect("OpenAPI parameter components should be an object")
        .iter()
        .map(|(name, parameter)| {
            (
                name.clone(),
                json!({
                    "name": parameter["name"].clone(),
                    "in": parameter["in"].clone(),
                    "required": parameter["required"].clone(),
                    "schema": parameter["schema"].clone(),
                }),
            )
        })
        .collect();
    Value::Object(contracts)
}

fn project_header_contracts(document: &Value) -> Value {
    let contracts = document["components"]["headers"]
        .as_object()
        .expect("OpenAPI header components should be an object")
        .iter()
        .map(|(name, header)| (name.clone(), header["schema"].clone()))
        .collect();
    Value::Object(contracts)
}

fn project_schema_shape(schema: &Value) -> Value {
    let object = schema
        .as_object()
        .expect("frozen public schemas should be objects");
    let mut projected = serde_json::Map::new();
    for key in [
        "$ref",
        "type",
        "additionalProperties",
        "required",
        "properties",
        "items",
        "anyOf",
        "enum",
        "format",
        "pattern",
        "minimum",
        "maximum",
        "minProperties",
        "maxLength",
        "default",
    ] {
        let Some(value) = object.get(key) else {
            continue;
        };
        let value = match key {
            "properties" => Value::Object(
                value
                    .as_object()
                    .expect("schema properties should be an object")
                    .iter()
                    .map(|(name, property)| (name.clone(), project_schema_shape(property)))
                    .collect(),
            ),
            "items" | "additionalProperties" if value.is_object() => project_schema_shape(value),
            "anyOf" => Value::Array(
                value
                    .as_array()
                    .expect("schema anyOf should be an array")
                    .iter()
                    .map(project_schema_shape)
                    .collect(),
            ),
            _ => value.clone(),
        };
        projected.insert(key.to_owned(), value);
    }
    Value::Object(projected)
}

fn request_schema<'a>(document: &'a Value, path: &str, method: &str) -> &'a Value {
    &document["paths"][path][method]["requestBody"]["content"]["application/json"]["schema"]
}

fn assert_request_schema_accepts(document: &Value, path: &str, method: &str, body: &Value) {
    validate_schema(
        document,
        request_schema(document, path, method),
        body,
        "request",
    )
    .unwrap_or_else(|error| panic!("{method} {path} request should match artifact: {error}"));
}

fn assert_request_schema_rejects(document: &Value, path: &str, method: &str, body: &Value) {
    assert!(
        validate_schema(
            document,
            request_schema(document, path, method),
            body,
            "request"
        )
        .is_err(),
        "{method} {path} request should be rejected by artifact: {body}"
    );
}

fn assert_all_local_refs_resolve(document: &Value) {
    visit_local_refs(document, document, "$");
}

fn visit_local_refs(document: &Value, value: &Value, path: &str) {
    match value {
        Value::Object(object) => {
            if let Some(reference) = object.get("$ref").and_then(Value::as_str) {
                let pointer = reference.strip_prefix('#').unwrap_or_else(|| {
                    panic!("external ref is not allowed at {path}: {reference}")
                });
                assert!(
                    document.pointer(pointer).is_some(),
                    "dangling local ref at {path}: {reference}"
                );
                let expected_prefix = expected_ref_prefix(path);
                assert!(
                    reference.starts_with(expected_prefix),
                    "wrong local ref target at {path}: expected {expected_prefix}, got {reference}"
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

fn expected_ref_prefix(path: &str) -> &'static str {
    if path.starts_with("$.components.schemas.") {
        "#/components/schemas/"
    } else if path.starts_with("$.components.responses.") && path.contains(".headers.") {
        "#/components/headers/"
    } else if (path.starts_with("$.components.responses.") && path.contains(".content."))
        || (path.starts_with("$.paths.") && path.contains(".requestBody."))
    {
        "#/components/schemas/"
    } else if path.starts_with("$.paths.") && path.contains(".responses.") {
        "#/components/responses/"
    } else if path.starts_with("$.paths.") && path.contains(".parameters[") {
        "#/components/parameters/"
    } else {
        panic!("unclassified local ref source: {path}")
    }
}

fn documented_operations(document: &Value) -> BTreeSet<(String, String)> {
    let mut operations = BTreeSet::new();
    for (path, item) in document["paths"]
        .as_object()
        .expect("OpenAPI paths should be an object")
    {
        for method in OPENAPI_METHODS {
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
        for method in OPENAPI_METHODS {
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

fn documented_operation_statuses(document: &Value) -> BTreeMap<(String, String), BTreeSet<u16>> {
    let mut operations = BTreeMap::new();
    for (path, item) in document["paths"]
        .as_object()
        .expect("OpenAPI paths should be an object")
    {
        for method in OPENAPI_METHODS {
            let Some(operation) = item.get(method) else {
                continue;
            };
            let statuses = operation["responses"]
                .as_object()
                .expect("operation responses should be an object")
                .keys()
                .map(|status| status.parse().expect("response status should be numeric"))
                .collect();
            operations.insert((method.to_owned(), path.clone()), statuses);
        }
    }
    operations
}

fn record_observed(
    observed: &mut ObservedResponses,
    path: &str,
    method: &str,
    concrete_path: &str,
    scenario: &str,
    status: StatusCode,
) {
    observed
        .by_operation
        .entry((method.to_owned(), path.to_owned()))
        .or_default()
        .insert(status.as_u16());
    assert!(
        observed.scenarios.insert((
            method.to_owned(),
            concrete_path.to_owned(),
            scenario.to_owned(),
            status.as_u16(),
        )),
        "duplicate router scenario key: {method} {concrete_path} {scenario} {status}"
    );
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
    assert_response_headers_declared(document, response_spec, response);

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
    let body = response.json();
    assert_public_time_fields(&body);
    validate_schema(document, schema, &body, "$")
        .unwrap_or_else(|error| panic!("{method} {path} {expected} schema mismatch: {error}"));
}

#[allow(clippy::too_many_arguments)]
fn assert_and_record(
    document: &Value,
    observed: &mut ObservedResponses,
    path: &str,
    method: &str,
    concrete_path: &str,
    scenario: &str,
    expected: StatusCode,
    response: &CapturedResponse,
) {
    assert_operation_response(document, path, method, expected, response);
    record_observed(
        observed,
        path,
        method,
        concrete_path,
        scenario,
        response.status,
    );
}

fn assert_response_headers_declared(
    document: &Value,
    response_spec: &Value,
    response: &CapturedResponse,
) {
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
    for (name, header) in headers {
        let Some(actual) = response.headers.get(name) else {
            assert_eq!(name, "Retry-After", "required response header is missing");
            continue;
        };
        let header = resolve_ref(document, header);
        let schema = &header["schema"];
        let value = if schema["type"] == "integer" {
            json!(
                actual
                    .to_str()
                    .expect("integer response header should be ASCII")
                    .parse::<u64>()
                    .expect("integer response header should be numeric")
            )
        } else {
            json!(
                actual
                    .to_str()
                    .expect("public response header should be ASCII")
            )
        };
        validate_schema(document, schema, &value, &format!("header.{name}"))
            .unwrap_or_else(|error| panic!("response header schema mismatch: {error}"));
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

fn assert_public_time_fields(value: &Value) {
    match value {
        Value::Object(object) => {
            for (name, value) in object {
                if matches!(
                    name.as_str(),
                    "retryAt" | "queuedAt" | "startedAt" | "completedAt"
                ) && let Some(value) = value.as_str()
                {
                    assert!(
                        is_public_time(value),
                        "{name} must use strict UTC six-microsecond format: {value}"
                    );
                }
                assert_public_time_fields(value);
            }
        }
        Value::Array(values) => {
            for value in values {
                assert_public_time_fields(value);
            }
        }
        _ => {}
    }
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

    if actual_type == "string" {
        let string = value.as_str().expect("string type was checked");
        if let Some(format) = schema.get("format").and_then(Value::as_str) {
            validate_format(format, string, path)?;
        }
        if let Some(pattern) = schema.get("pattern").and_then(Value::as_str) {
            validate_pattern(pattern, string, path)?;
        }
        if let Some(max_length) = schema.get("maxLength").and_then(Value::as_u64)
            && string.chars().count() as u64 > max_length
        {
            return Err(format!("{path} exceeds maxLength {max_length}"));
        }
    }
    if actual_type == "integer" {
        let integer = value
            .as_i64()
            .or_else(|| value.as_u64().and_then(|value| i64::try_from(value).ok()))
            .ok_or_else(|| format!("{path} integer is outside i64"))?;
        if let Some(minimum) = schema.get("minimum").and_then(Value::as_i64)
            && integer < minimum
        {
            return Err(format!("{path} is below minimum {minimum}"));
        }
        if let Some(maximum) = schema.get("maximum").and_then(Value::as_i64)
            && integer > maximum
        {
            return Err(format!("{path} is above maximum {maximum}"));
        }
    }

    match actual_type {
        "object" => {
            let object = value.as_object().expect("object type was checked");
            if let Some(min_properties) = schema.get("minProperties").and_then(Value::as_u64)
                && object.len() < min_properties as usize
            {
                return Err(format!(
                    "{path} has {} properties, below minProperties {min_properties}",
                    object.len()
                ));
            }
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

fn validate_format(format: &str, value: &str, path: &str) -> Result<(), String> {
    let valid = match format {
        "uuid" => Uuid::parse_str(value).is_ok_and(|parsed| parsed.to_string() == value),
        "uri" => url::Url::parse(value).is_ok(),
        "date-time" => is_public_time(value),
        other => return Err(format!("{path} uses unsupported format {other}")),
    };
    if valid {
        Ok(())
    } else {
        Err(format!("{path} does not satisfy format {format}: {value}"))
    }
}

fn validate_pattern(pattern: &str, value: &str, path: &str) -> Result<(), String> {
    let valid = match pattern {
        PUBLIC_TIME_PATTERN => is_public_time(value),
        LOCATION_PATTERN => value
            .strip_prefix("/api/v1/subscriptions/")
            .is_some_and(|id| Uuid::parse_str(id).is_ok_and(|parsed| parsed.to_string() == id)),
        HTTPS_FEED_URL_PATTERN => value.strip_prefix("https://").is_some_and(|remainder| {
            let authority_end = remainder.find(['/', '?', '#']).unwrap_or(remainder.len());
            let authority = &remainder[..authority_end];
            !authority.is_empty() && !authority.contains('@')
        }),
        other => return Err(format!("{path} uses unsupported pattern {other}")),
    };
    if valid {
        Ok(())
    } else {
        Err(format!(
            "{path} does not satisfy pattern {pattern}: {value}"
        ))
    }
}

struct CreatedSubscription {
    subscription_id: String,
    feed_id: String,
    operation_id: String,
}

async fn create_subscription(fixture: &ContractFixture, url: &str) -> CreatedSubscription {
    let response = fixture
        .request(
            Method::POST,
            "/api/v1/subscriptions",
            Some(json!({ "url": url })),
            true,
            true,
        )
        .await;
    assert_eq!(response.status, StatusCode::CREATED);
    let body = response.json();
    CreatedSubscription {
        subscription_id: body["subscription"]["subscriptionId"]
            .as_str()
            .expect("created subscription should expose subscriptionId")
            .to_owned(),
        feed_id: body["subscription"]["feedId"]
            .as_str()
            .expect("created subscription should expose feedId")
            .to_owned(),
        operation_id: body["subscription"]["refresh"]["operationId"]
            .as_str()
            .expect("created subscription should expose operationId")
            .to_owned(),
    }
}

async fn exhaust_mutation_limit(fixture: &ContractFixture, url: &str) {
    for _ in 0..30 {
        let response = fixture
            .request(
                Method::POST,
                "/api/v1/subscriptions",
                Some(json!({ "url": url })),
                true,
                true,
            )
            .await;
        assert!(matches!(
            response.status,
            StatusCode::OK | StatusCode::CREATED
        ));
    }
}

async fn drop_table(database: &DatabaseConnection, table: &str) {
    database
        .execute(Statement::from_string(
            DbBackend::Sqlite,
            format!("DROP TABLE {table}"),
        ))
        .await
        .expect("error fixture table should drop");
}

async fn sqlite_database_now(database: &DatabaseConnection) -> OffsetDateTime {
    database
        .query_one(Statement::from_string(
            DbBackend::Sqlite,
            "SELECT strftime('%Y-%m-%dT%H:%M:%f000Z','now') AS database_now".to_owned(),
        ))
        .await
        .expect("database time should query")
        .expect("database time row should exist")
        .try_get("", "database_now")
        .expect("database time should decode")
}

async fn set_feed_retry_after(
    database: &DatabaseConnection,
    feed_id: &str,
    retry_at: OffsetDateTime,
) {
    let stored_feed = feed::Entity::find_by_id(feed_id)
        .one(database)
        .await
        .expect("cooldown feed should query")
        .expect("cooldown feed should exist");
    let mut stored_feed = stored_feed.into_active_model();
    stored_feed.last_attempt_at = Set(None);
    stored_feed.retry_after_at = Set(Some(retry_at));
    stored_feed
        .update(database)
        .await
        .expect("cooldown retry time should persist");
}

fn public_time(value: OffsetDateTime) -> String {
    value
        .to_offset(UtcOffset::UTC)
        .format(format_description!(
            "[year]-[month]-[day]T[hour]:[minute]:[second].[subsecond digits:6]Z"
        ))
        .expect("public time fixture should format")
}

fn assert_rate_limited_with_retry(response: &CapturedResponse, retry_at: Option<&str>) {
    let body = response.json();
    assert_eq!(body["error"]["code"], "RATE_LIMITED");
    assert_eq!(body["error"]["message"], "Too many requests");
    let fields = body["error"]["fields"]
        .as_object()
        .expect("temporal 429 should include fields");
    assert_eq!(fields.len(), 1);
    let actual_retry_at = fields["retryAt"]
        .as_str()
        .expect("temporal 429 should include retryAt");
    assert!(is_public_time(actual_retry_at));
    if let Some(retry_at) = retry_at {
        assert_eq!(actual_retry_at, retry_at);
    }
    let retry_after = response
        .headers
        .get(RETRY_AFTER)
        .expect("temporal 429 should include Retry-After")
        .to_str()
        .expect("Retry-After should be ASCII")
        .parse::<u64>()
        .expect("Retry-After should be integer seconds");
    assert!(retry_after >= 1);
}

fn assert_rate_limited_without_retry(response: &CapturedResponse) {
    let body = response.json();
    assert_eq!(body["error"]["code"], "RATE_LIMITED");
    assert_eq!(body["error"]["message"], "Too many requests");
    assert!(body["error"].get("fields").is_none());
    assert!(response.headers.get(RETRY_AFTER).is_none());
}

fn is_public_time(value: &str) -> bool {
    let bytes = value.as_bytes();
    if bytes.len() != 27
        || ![4, 7].into_iter().all(|index| bytes[index] == b'-')
        || bytes[10] != b'T'
        || ![13, 16].into_iter().all(|index| bytes[index] == b':')
        || bytes[19] != b'.'
        || bytes[26] != b'Z'
        || !bytes.iter().enumerate().all(|(index, byte)| {
            matches!(index, 4 | 7 | 10 | 13 | 16 | 19 | 26) || byte.is_ascii_digit()
        })
    {
        return false;
    }
    let Ok(year) = value[0..4].parse::<u16>() else {
        return false;
    };
    let Ok(month) = value[5..7].parse::<u8>() else {
        return false;
    };
    let Ok(day) = value[8..10].parse::<u8>() else {
        return false;
    };
    let days = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if year.is_multiple_of(400) || (year.is_multiple_of(4) && !year.is_multiple_of(100)) => {
            29
        }
        2 => 28,
        _ => return false,
    };
    (1..=days).contains(&day)
        && value[11..13].parse::<u8>().is_ok_and(|hour| hour <= 23)
        && value[14..16].parse::<u8>().is_ok_and(|minute| minute <= 59)
        && value[17..19].parse::<u8>().is_ok_and(|second| second <= 59)
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

async fn seed_subscription_quota(database: &DatabaseConnection) {
    let at = datetime!(2026-07-17 12:00:00 UTC);
    for index in 0..1_000_u128 {
        let feed_id =
            Uuid::from_u128(0x9400_0000_0000_4000_8000_0000_0000_0000 + index).to_string();
        let subscription_id =
            Uuid::from_u128(0x9500_0000_0000_4000_8000_0000_0000_0000 + index).to_string();
        quota_feed_model(
            feed_id.clone(),
            format!("https://openapi-subscription-quota-{index:04}.example/rss.xml"),
        )
        .insert(database)
        .await
        .expect("subscription quota feed should insert");
        subscription::ActiveModel {
            id: Set(subscription_id),
            user_id: Set(USER_A_ID.to_owned()),
            feed_id: Set(feed_id),
            category_id: Set(None),
            title_override: Set(None),
            position: Set(0),
            start_sequence: Set(0),
            read_through_sequence: Set(0),
            state_revision: Set(0),
            created_at: Set(at),
            updated_at: Set(at),
        }
        .insert(database)
        .await
        .expect("subscription quota row should insert");
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
