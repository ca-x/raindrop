#[allow(dead_code)]
mod support;

use std::{collections::BTreeSet, fs};

use axum::{
    Router,
    body::Body,
    http::{
        Method, Request, StatusCode,
        header::{CACHE_CONTROL, CONTENT_TYPE, COOKIE, HOST, LOCATION, ORIGIN},
    },
};
use http_body_util::BodyExt;
use raindrop::{
    app::{AppState, build_router},
    auth::build_session_cookie,
    db::{DatabaseConfig, connect, migrate},
    setup::SetupService,
};
use secrecy::{ExposeSecret, SecretString};
use serde_json::{Value, json};
use support::database::{USER_A_ID, insert_user};
use tempfile::TempDir;
use tower::ServiceExt;

const OPENAPI_PATH: &str = "docs/openapi/organization-v1.json";
const CATEGORY_PATH: &str = "/api/v1/categories/{categoryId}";
const METHODS: [&str; 8] = [
    "get", "put", "post", "delete", "options", "head", "patch", "trace",
];

#[test]
fn organization_openapi_freezes_the_public_surface_and_schema() {
    let document = load_openapi();
    assert_eq!(document["openapi"], "3.1.0");
    assert_eq!(
        documented_operations(&document),
        BTreeSet::from([
            ("GET".to_owned(), "/api/v1/categories".to_owned()),
            ("POST".to_owned(), "/api/v1/categories".to_owned()),
            ("PATCH".to_owned(), CATEGORY_PATH.to_owned()),
            ("DELETE".to_owned(), CATEGORY_PATH.to_owned()),
        ])
    );
    assert_statuses(&document, "/api/v1/categories", "get", &[200, 401, 500]);
    assert_statuses(
        &document,
        "/api/v1/categories",
        "post",
        &[201, 401, 403, 409, 422, 429, 500],
    );
    assert_statuses(
        &document,
        CATEGORY_PATH,
        "patch",
        &[200, 401, 403, 404, 409, 422, 429, 500],
    );
    assert_statuses(
        &document,
        CATEGORY_PATH,
        "delete",
        &[204, 401, 403, 404, 422, 429, 500],
    );
    assert_required(&document, "Category", &["categoryId", "title", "position"]);
    assert_required(&document, "CategoryList", &["items"]);
    assert_required(&document, "CreateCategoryRequest", &["title"]);
    assert_required(&document, "ApiError", &["code", "message", "requestId"]);
    assert_eq!(
        document["components"]["schemas"]["CategoryList"]["properties"]["items"]["maxItems"],
        250
    );
    assert_eq!(
        document["components"]["schemas"]["Category"]["properties"]["position"]["minimum"],
        0
    );
    assert_eq!(
        document["components"]["schemas"]["UpdateCategoryRequest"]["anyOf"]
            .as_array()
            .expect("update anyOf should be an array")
            .len(),
        2
    );
    assert_all_local_refs_resolve(&document, &document);

    let serialized = serde_json::to_string(&document)
        .expect("organization OpenAPI should serialize")
        .to_ascii_lowercase();
    for forbidden in [
        "normalizedtitle",
        "userid",
        "createdat",
        "updatedat",
        "password",
        "databaseurl",
    ] {
        assert!(
            !serialized.contains(forbidden),
            "organization OpenAPI leaks internal field {forbidden}"
        );
    }
}

#[tokio::test]
async fn organization_openapi_matches_real_router_responses() {
    let fixture = ContractFixture::new().await;
    let document = load_openapi();

    let unauthenticated = fixture
        .request(Method::GET, "/api/v1/categories", None, false)
        .await;
    assert_observed(
        &document,
        "/api/v1/categories",
        "get",
        &unauthenticated,
        401,
    );
    assert_error_envelope(&unauthenticated.json());

    let empty = fixture
        .request(Method::GET, "/api/v1/categories", None, true)
        .await;
    assert_observed(&document, "/api/v1/categories", "get", &empty, 200);
    assert_eq!(empty.json(), json!({ "items": [] }));

    let invalid = fixture
        .request(
            Method::POST,
            "/api/v1/categories",
            Some(json!({ "title": "" })),
            true,
        )
        .await;
    assert_observed(&document, "/api/v1/categories", "post", &invalid, 422);
    assert_error_envelope(&invalid.json());

    let created = fixture
        .request(
            Method::POST,
            "/api/v1/categories",
            Some(json!({ "title": "Technology" })),
            true,
        )
        .await;
    assert_observed(&document, "/api/v1/categories", "post", &created, 201);
    let created_json = created.json();
    assert_category(&created_json);
    let category_id = created_json["categoryId"]
        .as_str()
        .expect("created category ID should be a string");
    assert_eq!(
        created
            .headers
            .get(LOCATION)
            .and_then(|value| value.to_str().ok()),
        Some(format!("/api/v1/categories/{category_id}").as_str())
    );

    let updated = fixture
        .request(
            Method::PATCH,
            &format!("/api/v1/categories/{category_id}"),
            Some(json!({ "title": "Science", "position": 512 })),
            true,
        )
        .await;
    assert_observed(&document, CATEGORY_PATH, "patch", &updated, 200);
    assert_category(&updated.json());

    let deleted = fixture
        .request(
            Method::DELETE,
            &format!("/api/v1/categories/{category_id}"),
            None,
            true,
        )
        .await;
    assert_observed(&document, CATEGORY_PATH, "delete", &deleted, 204);
    assert!(deleted.body.is_empty());
}

struct ContractFixture {
    _data: TempDir,
    app: Router,
    cookie: String,
    csrf: String,
}

impl ContractFixture {
    async fn new() -> Self {
        let data = tempfile::tempdir().expect("temporary directory should be created");
        let url = format!(
            "sqlite://{}?mode=rwc",
            data.path().join("organization-openapi.db").display()
        );
        let database = connect(&DatabaseConfig::new(SecretString::from(url)))
            .await
            .expect("organization OpenAPI database should connect");
        migrate(&database)
            .await
            .expect("organization OpenAPI database should migrate");
        insert_user(&database, USER_A_ID, "organization-openapi").await;
        let setup = SetupService::ready(data.path(), None, database);
        let session = setup
            .sessions()
            .create(USER_A_ID)
            .await
            .expect("organization OpenAPI session should create");
        let cookie = build_session_cookie(&session, false)
            .to_string()
            .split(';')
            .next()
            .expect("session cookie should contain a pair")
            .to_owned();
        let csrf = session.csrf_token.expose_secret().to_owned();
        Self {
            _data: data,
            app: build_router(AppState::new(setup)),
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
    ) -> CapturedResponse {
        let mutation = matches!(method, Method::POST | Method::PATCH | Method::DELETE);
        let mut request = Request::builder().method(method).uri(uri);
        if authenticated {
            request = request.header(COOKIE, &self.cookie);
        }
        if mutation && authenticated {
            request = request
                .header("x-csrf-token", &self.csrf)
                .header(ORIGIN, "http://organization-openapi.test")
                .header(HOST, "organization-openapi.test");
        }
        if body.is_some() {
            request = request.header(CONTENT_TYPE, "application/json");
        }
        let response = self
            .app
            .clone()
            .oneshot(
                request
                    .body(body.map_or_else(Body::empty, |body| Body::from(body.to_string())))
                    .expect("organization OpenAPI request should build"),
            )
            .await
            .expect("organization OpenAPI request should complete");
        CapturedResponse::from_response(response).await
    }
}

struct CapturedResponse {
    status: StatusCode,
    headers: axum::http::HeaderMap,
    body: Vec<u8>,
}

impl CapturedResponse {
    async fn from_response(response: axum::response::Response) -> Self {
        let (parts, body) = response.into_parts();
        Self {
            status: parts.status,
            headers: parts.headers,
            body: body
                .collect()
                .await
                .expect("organization OpenAPI body should collect")
                .to_bytes()
                .to_vec(),
        }
    }

    fn json(&self) -> Value {
        serde_json::from_slice(&self.body).expect("organization response should be JSON")
    }
}

fn assert_observed(
    document: &Value,
    path: &str,
    method: &str,
    response: &CapturedResponse,
    status: u16,
) {
    assert_eq!(response.status.as_u16(), status);
    assert!(
        document["paths"][path][method]["responses"]
            .get(status.to_string())
            .is_some(),
        "observed {method} {path} status {status} must be documented"
    );
    assert_eq!(
        response
            .headers
            .get(CACHE_CONTROL)
            .and_then(|value| value.to_str().ok()),
        Some("no-store")
    );
}

fn assert_category(value: &Value) {
    let object = value.as_object().expect("category should be an object");
    assert_eq!(
        object.keys().cloned().collect::<BTreeSet<_>>(),
        BTreeSet::from([
            "categoryId".to_owned(),
            "position".to_owned(),
            "title".to_owned(),
        ])
    );
    assert!(value["categoryId"].is_string());
    assert!(value["title"].is_string());
    assert!(value["position"].is_i64() || value["position"].is_u64());
}

fn assert_error_envelope(value: &Value) {
    assert!(value["error"]["code"].is_string());
    assert!(value["error"]["message"].is_string());
    assert!(value["error"]["requestId"].is_string());
}

fn load_openapi() -> Value {
    let artifact = fs::read_to_string(OPENAPI_PATH)
        .unwrap_or_else(|error| panic!("organization OpenAPI should exist: {error}"));
    serde_json::from_str(&artifact).expect("organization OpenAPI should be valid JSON")
}

fn documented_operations(document: &Value) -> BTreeSet<(String, String)> {
    let mut operations = BTreeSet::new();
    for (path, item) in document["paths"]
        .as_object()
        .expect("OpenAPI paths should be an object")
    {
        for method in METHODS {
            if item.get(method).is_some() {
                operations.insert((method.to_ascii_uppercase(), path.clone()));
            }
        }
    }
    operations
}

fn assert_statuses(document: &Value, path: &str, method: &str, expected: &[u16]) {
    let actual = document["paths"][path][method]["responses"]
        .as_object()
        .expect("operation responses should be an object")
        .keys()
        .map(|status| {
            status
                .parse::<u16>()
                .expect("response status should be numeric")
        })
        .collect::<BTreeSet<_>>();
    assert_eq!(actual, expected.iter().copied().collect());
}

fn assert_required(document: &Value, schema: &str, expected: &[&str]) {
    let actual = document["components"]["schemas"][schema]["required"]
        .as_array()
        .expect("schema required fields should be an array")
        .iter()
        .map(|field| field.as_str().expect("required field should be a string"))
        .collect::<BTreeSet<_>>();
    assert_eq!(actual, expected.iter().copied().collect());
}

fn assert_all_local_refs_resolve(document: &Value, value: &Value) {
    match value {
        Value::Object(object) => {
            if let Some(reference) = object.get("$ref").and_then(Value::as_str) {
                let pointer = reference
                    .strip_prefix('#')
                    .expect("only local OpenAPI references are allowed");
                assert!(
                    document.pointer(pointer).is_some(),
                    "OpenAPI reference should resolve: {reference}"
                );
            }
            for nested in object.values() {
                assert_all_local_refs_resolve(document, nested);
            }
        }
        Value::Array(array) => {
            for nested in array {
                assert_all_local_refs_resolve(document, nested);
            }
        }
        _ => {}
    }
}
