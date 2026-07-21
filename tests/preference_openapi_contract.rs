#[allow(dead_code)]
mod support;

use std::{collections::BTreeSet, fs};

use axum::{
    Router,
    body::Body,
    http::{
        Method, Request, StatusCode,
        header::{
            ACCEPT_LANGUAGE, CACHE_CONTROL, CONTENT_TYPE, COOKIE, HOST, ORIGIN, PRAGMA, RETRY_AFTER,
        },
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

const OPENAPI_PATH: &str = "docs/openapi/preferences-v1.json";
const OPENAPI_V2_PATH: &str = "docs/openapi/preferences-v2.json";
const PREFERENCES_PATH: &str = "/api/v1/preferences";
const METHODS: [&str; 8] = [
    "get", "put", "post", "delete", "options", "head", "patch", "trace",
];

#[test]
fn preferences_openapi_freezes_the_public_surface_and_strict_schemas() {
    let document = load_openapi();
    assert_eq!(document["openapi"], "3.1.0");
    assert_eq!(
        documented_operations(&document),
        BTreeSet::from([
            ("GET".to_owned(), PREFERENCES_PATH.to_owned()),
            ("PATCH".to_owned(), PREFERENCES_PATH.to_owned()),
        ])
    );
    assert_statuses(&document, "get", &[200, 401, 500]);
    assert_statuses(&document, "patch", &[200, 401, 403, 422, 429, 500]);
    assert_eq!(
        document["paths"][PREFERENCES_PATH]["get"]["security"],
        json!([{ "sessionCookie": [] }])
    );
    assert_eq!(
        document["paths"][PREFERENCES_PATH]["patch"]["security"],
        json!([{ "sessionCookie": [] }])
    );
    assert_eq!(
        document["paths"][PREFERENCES_PATH]["patch"]["parameters"],
        json!([
            { "$ref": "#/components/parameters/AcceptLanguage" },
            { "$ref": "#/components/parameters/CsrfToken" }
        ])
    );

    let response = &document["components"]["schemas"]["UserPreferences"];
    assert_eq!(response["additionalProperties"], false);
    assert_required(
        &document,
        "UserPreferences",
        &["locale", "themeMode", "layoutDensity", "readingFontScale"],
    );
    assert_exact_properties(
        response,
        &["locale", "themeMode", "layoutDensity", "readingFontScale"],
    );

    let patch = &document["components"]["schemas"]["PatchUserPreferencesRequest"];
    assert_eq!(patch["additionalProperties"], false);
    assert_eq!(patch["minProperties"], 1);
    assert!(patch.get("required").is_none());
    assert_exact_properties(
        patch,
        &["locale", "themeMode", "layoutDensity", "readingFontScale"],
    );
    for schema in [response, patch] {
        assert_eq!(
            schema["properties"]["locale"]["enum"],
            json!(["zh-CN", "en"])
        );
        assert_eq!(
            schema["properties"]["themeMode"]["enum"],
            json!(["SYSTEM", "LIGHT", "DARK"])
        );
        assert_eq!(
            schema["properties"]["layoutDensity"]["enum"],
            json!(["COMPACT", "BALANCED", "SPACIOUS"])
        );
        assert_eq!(schema["properties"]["readingFontScale"]["minimum"], 85);
        assert_eq!(schema["properties"]["readingFontScale"]["maximum"], 130);
    }

    for response_name in ["Preferences", "Error", "RateLimited"] {
        let headers = &document["components"]["responses"][response_name]["headers"];
        assert_eq!(
            headers["Cache-Control"]["$ref"],
            "#/components/headers/CacheControl"
        );
        assert_eq!(headers["Pragma"]["$ref"], "#/components/headers/Pragma");
    }
    assert_eq!(
        document["components"]["responses"]["RateLimited"]["headers"]["Retry-After"]["$ref"],
        "#/components/headers/RetryAfter"
    );
    assert_required(&document, "ApiError", &["code", "message", "requestId"]);
    assert_all_local_refs_resolve(&document, &document);

    let serialized = serde_json::to_string(&document)
        .expect("preferences OpenAPI should serialize")
        .to_ascii_lowercase();
    for forbidden in [
        "userid",
        "user_id",
        "createdat",
        "created_at",
        "updatedat",
        "updated_at",
        "databaseurl",
    ] {
        assert!(
            !serialized.contains(forbidden),
            "preferences OpenAPI leaks internal field {forbidden}"
        );
    }
}

#[test]
fn preferences_v2_adds_reader_fields_on_a_versioned_path() {
    let document: Value = serde_json::from_str(
        &fs::read_to_string(OPENAPI_V2_PATH).expect("preferences v2 OpenAPI should exist"),
    )
    .expect("preferences v2 OpenAPI should be valid JSON");
    let path = "/api/v2/preferences";
    assert!(document["paths"][path]["get"].is_object());
    assert!(document["paths"][path]["patch"].is_object());
    assert_required(
        &document,
        "UserPreferences",
        &[
            "locale",
            "themeMode",
            "layoutDensity",
            "readingFontScale",
            "readingFontFamily",
            "readingCustomFontId",
            "readingColorScheme",
            "linkOpenMode",
        ],
    );
    assert_exact_properties(
        &document["components"]["schemas"]["UserPreferences"],
        &[
            "locale",
            "themeMode",
            "layoutDensity",
            "readingFontScale",
            "readingFontFamily",
            "readingCustomFontId",
            "readingColorScheme",
            "linkOpenMode",
        ],
    );
    assert_all_local_refs_resolve(&document, &document);
}

#[tokio::test]
async fn preferences_openapi_matches_real_router_responses() {
    let fixture = ContractFixture::new().await;
    let document = load_openapi();

    let unauthenticated = fixture.request(Method::GET, None, false, false).await;
    assert_observed(&document, "get", &unauthenticated, 401);
    assert_error_envelope(&unauthenticated.json());

    let initial = fixture.request(Method::GET, None, true, false).await;
    assert_observed(&document, "get", &initial, 200);
    assert_preferences(&initial.json());
    assert_eq!(initial.json()["locale"], "zh-CN");

    let missing_csrf = fixture
        .request(
            Method::PATCH,
            Some(json!({ "themeMode": "DARK" })),
            true,
            false,
        )
        .await;
    assert_observed(&document, "patch", &missing_csrf, 403);
    assert_error_envelope(&missing_csrf.json());

    let invalid = fixture
        .request(Method::PATCH, Some(json!({})), true, true)
        .await;
    assert_observed(&document, "patch", &invalid, 422);
    assert_error_envelope(&invalid.json());

    let updated = fixture
        .request(
            Method::PATCH,
            Some(json!({
                "locale": "en",
                "themeMode": "LIGHT",
                "layoutDensity": "SPACIOUS",
                "readingFontScale": 85
            })),
            true,
            true,
        )
        .await;
    assert_observed(&document, "patch", &updated, 200);
    assert_preferences(&updated.json());

    for _ in 0..28 {
        let admitted = fixture
            .request(
                Method::PATCH,
                Some(json!({ "themeMode": "LIGHT" })),
                true,
                true,
            )
            .await;
        assert_eq!(admitted.status, StatusCode::OK);
    }
    let limited = fixture
        .request(
            Method::PATCH,
            Some(json!({ "themeMode": "DARK" })),
            true,
            true,
        )
        .await;
    assert_observed(&document, "patch", &limited, 429);
    assert_error_envelope(&limited.json());
    assert!(limited.headers.get(RETRY_AFTER).is_some());
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
        let database_url = format!(
            "sqlite://{}?mode=rwc",
            data.path().join("preferences-openapi.db").display()
        );
        let database = connect(&DatabaseConfig::new(SecretString::from(database_url)))
            .await
            .expect("preferences OpenAPI database should connect");
        migrate(&database)
            .await
            .expect("preferences OpenAPI database should migrate");
        insert_user(&database, USER_A_ID, "preferences-openapi").await;
        let setup = SetupService::ready(data.path(), None, database);
        let session = setup
            .sessions()
            .create(USER_A_ID)
            .await
            .expect("preferences OpenAPI session should create");
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
        body: Option<Value>,
        authenticated: bool,
        valid_csrf: bool,
    ) -> CapturedResponse {
        let mut request = Request::builder()
            .method(method)
            .uri(PREFERENCES_PATH)
            .header(ACCEPT_LANGUAGE, "zh-Hans, en;q=0.8");
        if authenticated {
            request = request.header(COOKIE, &self.cookie);
        }
        if valid_csrf {
            request = request
                .header("x-csrf-token", &self.csrf)
                .header(ORIGIN, "http://preferences-openapi.test")
                .header(HOST, "preferences-openapi.test");
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
                    .expect("preferences OpenAPI request should build"),
            )
            .await
            .expect("preferences OpenAPI request should complete");
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
                .expect("preferences OpenAPI body should collect")
                .to_bytes()
                .to_vec(),
        }
    }

    fn json(&self) -> Value {
        serde_json::from_slice(&self.body).expect("preferences response should be JSON")
    }
}

fn assert_observed(document: &Value, method: &str, response: &CapturedResponse, status: u16) {
    assert_eq!(response.status.as_u16(), status);
    assert!(
        document["paths"][PREFERENCES_PATH][method]["responses"]
            .get(status.to_string())
            .is_some(),
        "observed {method} preference status {status} must be documented"
    );
    assert_eq!(response.headers.get(CACHE_CONTROL).unwrap(), "no-store");
    assert_eq!(response.headers.get(PRAGMA).unwrap(), "no-cache");
}

fn assert_preferences(value: &Value) {
    let object = value
        .as_object()
        .expect("preferences response should be an object");
    assert_eq!(
        object.keys().cloned().collect::<BTreeSet<_>>(),
        BTreeSet::from([
            "layoutDensity".to_owned(),
            "locale".to_owned(),
            "readingFontScale".to_owned(),
            "themeMode".to_owned(),
        ])
    );
    assert!(matches!(value["locale"].as_str(), Some("zh-CN" | "en")));
    assert!(matches!(
        value["themeMode"].as_str(),
        Some("SYSTEM" | "LIGHT" | "DARK")
    ));
    assert!(matches!(
        value["layoutDensity"].as_str(),
        Some("COMPACT" | "BALANCED" | "SPACIOUS")
    ));
    assert!((85..=130).contains(&value["readingFontScale"].as_i64().unwrap()));
}

fn assert_error_envelope(value: &Value) {
    assert!(value["error"]["code"].is_string());
    assert!(value["error"]["message"].is_string());
    assert!(value["error"]["requestId"].is_string());
}

fn load_openapi() -> Value {
    let artifact = fs::read_to_string(OPENAPI_PATH)
        .unwrap_or_else(|error| panic!("preferences OpenAPI should exist: {error}"));
    serde_json::from_str(&artifact).expect("preferences OpenAPI should be valid JSON")
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

fn assert_statuses(document: &Value, method: &str, expected: &[u16]) {
    let actual = document["paths"][PREFERENCES_PATH][method]["responses"]
        .as_object()
        .expect("operation responses should be an object")
        .keys()
        .map(|status| status.parse::<u16>().expect("status should be numeric"))
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

fn assert_exact_properties(schema: &Value, expected: &[&str]) {
    let actual = schema["properties"]
        .as_object()
        .expect("schema properties should be an object")
        .keys()
        .map(String::as_str)
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
