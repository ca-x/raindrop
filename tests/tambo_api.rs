#[allow(dead_code)]
mod support;

use std::sync::{Arc, Mutex};

use axum::{
    Router,
    body::{Body, Bytes},
    extract::State,
    http::{HeaderMap, Method, Request, StatusCode, Uri, header},
    response::{IntoResponse, Response},
    routing::any,
};
use http_body_util::BodyExt;
use raindrop::{
    app::{AppState, build_router},
    auth::build_session_cookie,
    db::{DatabaseConfig, connect, migrate},
    plugins::{EmbeddedOfficialAiPlugin, PluginRegistryRepository},
    setup::SetupService,
    tambo::{TamboClient, TamboConfig},
};
use sea_orm::{ConnectionTrait, DatabaseConnection, DbBackend, Statement};
use secrecy::{ExposeSecret, SecretString};
use serde_json::{Value, json};
use support::database::{USER_A_ID, USER_B_ID, insert_user};
use tower::ServiceExt;

const SETTINGS: &str = "/api/v1/ai/tambo";
const PROXY: &str = "/api/v1/ai/tambo/api/v1/threads/runs";
const PROJECT_KEY: &str = "private-project-key-never-returned";

struct UpstreamRequest {
    headers: HeaderMap,
    uri: Uri,
    body: Value,
}

type Captured = Arc<Mutex<Vec<UpstreamRequest>>>;

async fn upstream(
    State(captured): State<Captured>,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let path = uri.path().to_owned();
    captured.lock().unwrap().push(UpstreamRequest {
        headers,
        uri,
        body: if body.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&body).unwrap()
        },
    });
    if path.ends_with("/error") {
        return (StatusCode::INTERNAL_SERVER_ERROR, PROJECT_KEY).into_response();
    }
    if path.ends_with("/redirect") {
        return (
            StatusCode::TEMPORARY_REDIRECT,
            [(header::LOCATION, "https://example.com/leak")],
        )
            .into_response();
    }
    if path.ends_with("/missing") {
        return StatusCode::FORBIDDEN.into_response();
    }
    if path.ends_with("/oversized") {
        return (
            [(header::CONTENT_TYPE, "text/event-stream")],
            vec![b'a'; 4 * 1024 * 1024 + 1],
        )
            .into_response();
    }
    Response::builder()
        .header(header::CONTENT_TYPE, "text/event-stream")
        .header(header::SET_COOKIE, "upstream-secret=private")
        .header("x-thread-id", "thread_1")
        .header("x-run-id", "run_1")
        .body(Body::from("data: {\"type\":\"RUN_STARTED\"}\n\n"))
        .unwrap()
}

struct Fixture {
    data: tempfile::TempDir,
    database: DatabaseConnection,
    app: Router,
    cookies: [String; 2],
    csrf: [String; 2],
    captured: Captured,
    server: tokio::task::JoinHandle<()>,
}

impl Fixture {
    async fn new(configured: bool) -> Self {
        let data = tempfile::tempdir().unwrap();
        let url = format!(
            "sqlite://{}?mode=rwc",
            data.path().join("tambo.db").display()
        );
        let database = connect(&DatabaseConfig::new(SecretString::from(url)))
            .await
            .unwrap();
        migrate(&database).await.unwrap();
        insert_user(&database, USER_A_ID, "tambo-user-a").await;
        insert_user(&database, USER_B_ID, "tambo-user-b").await;
        PluginRegistryRepository::new(database.clone())
            .sync_bundled(EmbeddedOfficialAiPlugin::load().unwrap().bundle())
            .await
            .unwrap();
        let captured = Captured::default();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/api", listener.local_addr().unwrap());
        let upstream = Router::new()
            .route("/api/{*path}", any(upstream))
            .with_state(captured.clone());
        let server = tokio::spawn(async move {
            axum::serve(listener, upstream).await.unwrap();
        });
        let client = configured.then(|| {
            Arc::new(
                TamboClient::new(TamboConfig::new(&url, SecretString::from(PROJECT_KEY)).unwrap())
                    .unwrap(),
            )
        });
        let setup = SetupService::ready(data.path(), None, database.clone());
        let session_a = setup.sessions().create(USER_A_ID).await.unwrap();
        let session_b = setup.sessions().create(USER_B_ID).await.unwrap();
        let cookie = |session: &raindrop::auth::CreatedSession| {
            build_session_cookie(session, false)
                .to_string()
                .split(';')
                .next()
                .unwrap()
                .to_owned()
        };
        Self {
            data,
            database,
            app: build_router(AppState::new(setup).with_tambo(client)),
            cookies: [cookie(&session_a), cookie(&session_b)],
            csrf: [
                session_a.csrf_token.expose_secret().to_owned(),
                session_b.csrf_token.expose_secret().to_owned(),
            ],
            captured,
            server,
        }
    }

    async fn request(
        &self,
        method: Method,
        path: &str,
        user: Option<usize>,
        csrf: bool,
        body: Value,
    ) -> Response {
        let mut request = Request::builder().method(method).uri(path);
        if let Some(user) = user {
            request = request.header(header::COOKIE, &self.cookies[user]);
            if csrf {
                request = request
                    .header(
                        if path == SETTINGS {
                            "x-csrf-token"
                        } else {
                            "x-api-key"
                        },
                        &self.csrf[user],
                    )
                    .header(header::ORIGIN, "http://tambo.test")
                    .header(header::HOST, "tambo.test");
            }
        }
        let body = if body.is_null() {
            Body::empty()
        } else {
            request = request.header(header::CONTENT_TYPE, "application/json");
            Body::from(body.to_string())
        };
        self.app
            .clone()
            .oneshot(request.body(body).unwrap())
            .await
            .unwrap()
    }

    async fn enable(&self, user: usize) {
        let response = self
            .request(
                Method::PUT,
                SETTINGS,
                Some(user),
                true,
                json!({"expectedRevision":null,"enabled":true}),
            )
            .await;
        assert_eq!(response.status(), StatusCode::OK);
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.server.abort();
    }
}

async fn document(response: Response) -> Value {
    serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap()
}

#[tokio::test]
async fn opt_in_is_authenticated_csrf_protected_revisioned_and_persistent() {
    let mut fixture = Fixture::new(true).await;
    assert_eq!(
        fixture
            .request(Method::GET, SETTINGS, None, false, Value::Null)
            .await
            .status(),
        StatusCode::UNAUTHORIZED
    );
    let initial = document(
        fixture
            .request(Method::GET, SETTINGS, Some(0), false, Value::Null)
            .await,
    )
    .await;
    assert_eq!(initial["enabled"], false);
    assert_eq!(initial["revision"], Value::Null);
    let body = json!({"expectedRevision":null,"enabled":true});
    for invalid in [
        json!({"enabled":true}),
        json!({"expectedRevision":null,"enabled":true,"userId":USER_B_ID}),
    ] {
        assert_eq!(
            fixture
                .request(Method::PUT, SETTINGS, Some(0), true, invalid)
                .await
                .status(),
            StatusCode::UNPROCESSABLE_ENTITY
        );
    }
    assert_eq!(
        fixture
            .request(Method::PUT, SETTINGS, Some(0), false, body.clone())
            .await
            .status(),
        StatusCode::FORBIDDEN
    );
    fixture.enable(0).await;
    assert_eq!(
        fixture
            .request(Method::PUT, SETTINGS, Some(0), true, body)
            .await
            .status(),
        StatusCode::CONFLICT
    );
    assert_eq!(
        document(
            fixture
                .request(Method::GET, SETTINGS, Some(1), false, Value::Null)
                .await
        )
        .await["enabled"],
        false
    );
    fixture.app = build_router(AppState::new(SetupService::ready(
        fixture.data.path(),
        None,
        fixture.database.clone(),
    )));
    let response = fixture
        .request(Method::GET, SETTINGS, Some(0), false, Value::Null)
        .await;
    assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
    let saved = document(response).await;
    assert_eq!(saved["configured"], false);
    assert_eq!(saved["enabled"], true);
    assert_eq!(saved["revision"], 0);
    let response = fixture
        .request(
            Method::PUT,
            SETTINGS,
            Some(0),
            true,
            json!({"expectedRevision":0,"enabled":false}),
        )
        .await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(document(response).await["revision"], 1);
}

#[tokio::test]
async fn missing_service_and_system_disable_preserve_settings_but_block_use() {
    let fixture = Fixture::new(false).await;
    assert_eq!(
        fixture
            .request(
                Method::PUT,
                SETTINGS,
                Some(0),
                true,
                json!({"expectedRevision":null,"enabled":true})
            )
            .await
            .status(),
        StatusCode::SERVICE_UNAVAILABLE
    );
    let fixture = Fixture::new(true).await;
    assert_eq!(
        fixture
            .request(Method::POST, PROXY, Some(0), true, json!({}))
            .await
            .status(),
        StatusCode::SERVICE_UNAVAILABLE
    );
    fixture.enable(0).await;
    for state in ["DISABLED", "QUARANTINED"] {
        fixture
            .database
            .execute(Statement::from_sql_and_values(
                DbBackend::Sqlite,
                "UPDATE plugin_installations SET system_state = ? WHERE plugin_key = ?",
                [state.into(), "raindrop.ai-content".into()],
            ))
            .await
            .unwrap();
        assert_eq!(
            fixture
                .request(Method::POST, PROXY, Some(0), true, json!({}))
                .await
                .status(),
            StatusCode::SERVICE_UNAVAILABLE
        );
        assert!(
            PluginRegistryRepository::new(fixture.database.clone())
                .get_tambo_preferences(USER_A_ID)
                .await
                .unwrap()
                .enabled
        );
    }
    assert!(fixture.captured.lock().unwrap().is_empty());
}

#[tokio::test]
async fn proxy_rebinds_identity_keeps_keys_server_side_and_streams_without_cookies() {
    let fixture = Fixture::new(true).await;
    fixture.enable(0).await;
    fixture.enable(1).await;
    assert_eq!(
        fixture
            .request(Method::POST, PROXY, Some(0), false, json!({}))
            .await
            .status(),
        StatusCode::FORBIDDEN
    );
    let path = format!("{PROXY}?userKey=victim&contextKey=victim&userToken=stolen&limit=5");
    let response = fixture.request(Method::POST, &path, Some(0), true,
        json!({"userKey":"victim","userToken":"stolen","thread":{"userKey":"victim"},"message":{"content":[]}})).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers()[header::CONTENT_TYPE],
        "text/event-stream"
    );
    assert_eq!(response.headers()["x-thread-id"], "thread_1");
    assert!(!response.headers().contains_key(header::SET_COOKIE));
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    assert!(String::from_utf8_lossy(&bytes).contains("RUN_STARTED"));
    let response = fixture
        .request(Method::POST, PROXY, Some(1), true, json!({}))
        .await;
    assert_eq!(response.status(), StatusCode::OK);
    drop(response);
    let captured = fixture.captured.lock().unwrap();
    assert_eq!(captured.len(), 2);
    for (index, user_id) in [USER_A_ID, USER_B_ID].iter().enumerate() {
        let request = &captured[index];
        assert_eq!(request.headers["x-api-key"], PROJECT_KEY);
        assert!(!request.headers.contains_key(header::COOKIE));
        assert!(!request.headers.contains_key(header::AUTHORIZATION));
        assert!(!request.headers.contains_key("x-csrf-token"));
        assert_eq!(
            request.body["thread"]["userKey"],
            format!("raindrop:{user_id}")
        );
        assert!(request.body.get("userToken").is_none());
        let query: Vec<_> =
            url::form_urlencoded::parse(request.uri.query().unwrap().as_bytes()).collect();
        assert_eq!(query.iter().filter(|(key, _)| key == "userKey").count(), 1);
        assert!(
            query
                .iter()
                .any(|(key, value)| key == "userKey" && value == &format!("raindrop:{user_id}"))
        );
        assert!(
            !query
                .iter()
                .any(|(key, _)| key == "userToken" || key == "contextKey")
        );
    }
}

#[tokio::test]
async fn proxy_rejects_legacy_admin_routes_redirects_and_upstream_error_details() {
    let fixture = Fixture::new(true).await;
    fixture.enable(0).await;
    for path in ["threads", "v1/projects", "v1/threads/id/suggestions"] {
        let response = fixture
            .request(
                Method::GET,
                &format!("{SETTINGS}/api/{path}"),
                Some(0),
                false,
                Value::Null,
            )
            .await;
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }
    assert!(fixture.captured.lock().unwrap().is_empty());
    for (id, status) in [
        ("error", StatusCode::BAD_GATEWAY),
        ("redirect", StatusCode::BAD_GATEWAY),
        ("missing", StatusCode::NOT_FOUND),
    ] {
        let response = fixture
            .request(
                Method::GET,
                &format!("{SETTINGS}/api/v1/threads/{id}"),
                Some(0),
                false,
                Value::Null,
            )
            .await;
        assert_eq!(response.status(), status);
        assert!(!document(response).await.to_string().contains(PROJECT_KEY));
    }
}

#[tokio::test]
async fn proxy_holds_concurrency_until_stream_drop_and_bounds_response_bytes() {
    let fixture = Fixture::new(true).await;
    fixture.enable(0).await;
    let path = format!("{SETTINGS}/api/v1/threads/thread_1");
    let first = fixture
        .request(Method::GET, &path, Some(0), false, Value::Null)
        .await;
    let second = fixture
        .request(Method::GET, &path, Some(0), false, Value::Null)
        .await;
    assert_eq!(first.status(), StatusCode::OK);
    assert_eq!(second.status(), StatusCode::OK);
    assert_eq!(
        fixture
            .request(Method::GET, &path, Some(0), false, Value::Null)
            .await
            .status(),
        StatusCode::TOO_MANY_REQUESTS
    );
    drop(first);
    let recovered = fixture
        .request(Method::GET, &path, Some(0), false, Value::Null)
        .await;
    assert_eq!(recovered.status(), StatusCode::OK);
    drop((second, recovered));
    let response = fixture
        .request(
            Method::GET,
            &format!("{SETTINGS}/api/v1/threads/oversized"),
            Some(0),
            false,
            Value::Null,
        )
        .await;
    assert_eq!(response.status(), StatusCode::OK);
    assert!(response.into_body().collect().await.is_err());
}

#[tokio::test]
async fn unsupported_persisted_settings_schema_fails_closed_without_rewriting_data() {
    let fixture = Fixture::new(true).await;
    let registry = PluginRegistryRepository::new(fixture.database.clone());
    let value = br#"{"schemaVersion":2,"enabled":true}"#.to_vec();
    registry
        .put_kv(
            "raindrop.ai-content",
            USER_A_ID,
            "interaction.tambo.v1",
            None,
            value.clone(),
        )
        .await
        .unwrap();
    let response = fixture
        .request(Method::GET, SETTINGS, Some(0), false, Value::Null)
        .await;
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(
        registry
            .get_kv("raindrop.ai-content", USER_A_ID, "interaction.tambo.v1")
            .await
            .unwrap()
            .unwrap()
            .value(),
        value
    );
    assert_eq!(
        fixture
            .request(Method::POST, PROXY, Some(0), true, json!({}))
            .await
            .status(),
        StatusCode::INTERNAL_SERVER_ERROR
    );
    assert!(fixture.captured.lock().unwrap().is_empty());
}
