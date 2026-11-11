#[allow(dead_code)]
mod support;

use axum::{
    Router,
    body::Body,
    http::{Method, Request, StatusCode},
};
use http_body_util::BodyExt;
use raindrop::{
    app::{AppState, build_router},
    auth::build_session_cookie,
    db::{DatabaseConfig, connect, migrate},
    setup::SetupService,
};
use sea_orm::{ConnectionTrait, DatabaseConnection, Statement};
use secrecy::{ExposeSecret, SecretString};
use serde_json::{Value, json};
use support::database::{
    ENTRY_A_ID, FEED_ID, HASH_A, SUBSCRIPTION_A_ID, USER_A_ID, USER_B_ID, insert_entry,
    insert_feed, insert_subscription, insert_user,
};
use tempfile::TempDir;
use tower::ServiceExt;

struct Fixture {
    _data: TempDir,
    database: DatabaseConnection,
    app: Router,
    cookies: [String; 2],
    csrf: [String; 2],
}

impl Fixture {
    async fn new() -> Self {
        Self::with_url(None).await
    }

    async fn with_url(url: Option<String>) -> Self {
        let data = tempfile::tempdir().unwrap();
        let external = url.is_some();
        let url = url.unwrap_or_else(|| {
            format!("sqlite://{}?mode=rwc", data.path().join("mcp.db").display())
        });
        let database = connect(&DatabaseConfig::new(SecretString::from(url)))
            .await
            .unwrap();
        if external {
            raindrop::db::rollback(&database).await.unwrap();
        }
        migrate(&database).await.unwrap();
        migrate(&database).await.unwrap();
        insert_user(&database, USER_A_ID, "mcp-a").await;
        insert_user(&database, USER_B_ID, "mcp-b").await;
        let setup = SetupService::ready(data.path(), None, database.clone());
        let mut cookies = Vec::new();
        let mut csrf = Vec::new();
        for user in [USER_A_ID, USER_B_ID] {
            let session = setup.sessions().create(user).await.unwrap();
            cookies.push(
                build_session_cookie(&session, false)
                    .to_string()
                    .split(';')
                    .next()
                    .unwrap()
                    .to_owned(),
            );
            csrf.push(session.csrf_token.expose_secret().to_owned());
        }
        Self {
            _data: data,
            database,
            app: build_router(AppState::new(setup)),
            cookies: cookies.try_into().unwrap(),
            csrf: csrf.try_into().unwrap(),
        }
    }

    async fn send(
        &self,
        method: Method,
        path: &str,
        headers: &[(&str, &str)],
        body: String,
    ) -> (StatusCode, Value) {
        let mut request = Request::builder().method(method).uri(path);
        for (name, value) in headers {
            request = request.header(*name, *value);
        }
        let response = self
            .app
            .clone()
            .oneshot(request.body(Body::from(body)).unwrap())
            .await
            .unwrap();
        let status = response.status();
        if path == "/mcp" && status == StatusCode::TOO_MANY_REQUESTS {
            let retry: u64 = response.headers()["retry-after"]
                .to_str()
                .unwrap()
                .parse()
                .unwrap();
            assert!((1..=60).contains(&retry));
        }
        assert_eq!(response.headers().get("cache-control").unwrap(), "no-store");
        let body = response.into_body().collect().await.unwrap().to_bytes();
        (status, serde_json::from_slice(&body).unwrap_or(Value::Null))
    }

    async fn manage(
        &self,
        method: Method,
        path: &str,
        user: usize,
        body: Value,
    ) -> (StatusCode, Value) {
        self.send(
            method,
            path,
            &[
                ("cookie", &self.cookies[user]),
                ("x-csrf-token", &self.csrf[user]),
                ("origin", "http://mcp.test"),
                ("host", "mcp.test"),
                ("content-type", "application/json"),
            ],
            body.to_string(),
        )
        .await
    }

    async fn token(&self, user: usize) -> Value {
        let (status, value) = self
            .manage(
                Method::POST,
                "/api/v1/mcp/tokens",
                user,
                json!({"name":"External AI","expiresInDays":30}),
            )
            .await;
        assert_eq!(status, StatusCode::OK);
        value
    }

    async fn rpc(&self, token: &str, body: Value) -> (StatusCode, Value) {
        self.send(
            Method::POST,
            "/mcp",
            &[
                ("authorization", &format!("Bearer {token}")),
                ("content-type", "application/json"),
                ("accept", "application/json, text/event-stream"),
                ("mcp-protocol-version", "2025-11-25"),
            ],
            body.to_string(),
        )
        .await
    }

    async fn sql(&self, sql: &str) {
        self.database
            .execute(Statement::from_string(
                self.database.get_database_backend(),
                sql,
            ))
            .await
            .unwrap();
    }
}

fn request(method: &str, params: Value) -> Value {
    json!({"jsonrpc":"2.0","id":1,"method":method,"params":params})
}

#[tokio::test]
async fn mcp_token_lifecycle_hash_only_and_owner_isolation() {
    let fixture = Fixture::new().await;
    assert_eq!(
        fixture
            .manage(Method::GET, "/api/v1/mcp/tokens", 0, Value::Null)
            .await
            .1,
        json!({"items":[]})
    );
    let created = fixture.token(0).await;
    let token = created["token"].as_str().unwrap();
    assert_eq!(token.len(), 49);
    let id = created["item"]["id"].as_str().unwrap();
    let row = fixture
        .database
        .query_one(Statement::from_string(
            fixture.database.get_database_backend(),
            "SELECT * FROM mcp_tokens",
        ))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        row.try_get::<String>("", "token_hash").unwrap(),
        blake3::hash(token.as_bytes()).to_hex().to_string()
    );
    assert_eq!(
        fixture
            .manage(Method::GET, "/api/v1/mcp/tokens", 1, Value::Null)
            .await
            .1,
        json!({"items":[]})
    );
    assert_eq!(
        fixture
            .manage(
                Method::DELETE,
                &format!("/api/v1/mcp/tokens/{id}"),
                1,
                Value::Null
            )
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        fixture.rpc(token, request("ping", json!({}))).await.0,
        StatusCode::OK
    );
    let listed = fixture
        .manage(Method::GET, "/api/v1/mcp/tokens", 0, Value::Null)
        .await
        .1;
    assert!(!listed.to_string().contains(token));
    assert!(listed["items"][0]["lastUsedAt"].is_string());
    assert!(listed["items"][0].get("token_hash").is_none());
    assert_eq!(listed["items"][0].as_object().unwrap().len(), 6);
    assert_eq!(
        fixture
            .manage(
                Method::DELETE,
                &format!("/api/v1/mcp/tokens/{id}"),
                0,
                Value::Null
            )
            .await
            .0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        fixture.rpc(token, request("ping", json!({}))).await.0,
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
async fn mcp_management_requires_session_csrf_and_valid_input() {
    let fixture = Fixture::new().await;
    assert_eq!(
        fixture
            .send(Method::GET, "/api/v1/mcp/tokens", &[], String::new())
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        fixture
            .send(
                Method::POST,
                "/api/v1/mcp/tokens",
                &[
                    ("cookie", &fixture.cookies[0]),
                    ("content-type", "application/json")
                ],
                json!({"name":"x","expiresInDays":30}).to_string()
            )
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    for body in [
        json!({"name":"","expiresInDays":30}),
        json!({"name":"x".repeat(81),"expiresInDays":30}),
        json!({"name":"x","expiresInDays":31}),
        json!({"name":"x","expiresInDays":30,"token":"bad"}),
        json!({"name":"x\u{0000}","expiresInDays":30}),
    ] {
        assert_eq!(
            fixture
                .manage(Method::POST, "/api/v1/mcp/tokens", 0, body)
                .await
                .0,
            StatusCode::UNPROCESSABLE_ENTITY
        );
    }
    for days in [30, 90, 365] {
        let (status, value) = fixture
            .manage(
                Method::POST,
                "/api/v1/mcp/tokens",
                0,
                json!({"name":"x","expiresInDays":days}),
            )
            .await;
        assert_eq!(status, StatusCode::OK);
        let start = time::OffsetDateTime::parse(
            value["item"]["createdAt"].as_str().unwrap(),
            &time::format_description::well_known::Rfc3339,
        )
        .unwrap();
        let end = time::OffsetDateTime::parse(
            value["item"]["expiresAt"].as_str().unwrap(),
            &time::format_description::well_known::Rfc3339,
        )
        .unwrap();
        assert_eq!((end - start).whole_days(), days);
    }
}

#[tokio::test]
async fn mcp_expired_disabled_and_non_bearer_auth_are_rejected() {
    let fixture = Fixture::new().await;
    let created = fixture.token(0).await;
    let token = created["token"].as_str().unwrap();
    for auth in ["", "Basic bad", "Bearer bad"] {
        assert_eq!(
            fixture.rpc(auth, request("ping", json!({}))).await.0,
            StatusCode::UNAUTHORIZED
        );
    }
    assert_eq!(
        fixture
            .send(
                Method::POST,
                "/mcp",
                &[("cookie", &fixture.cookies[0])],
                request("ping", json!({})).to_string()
            )
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        fixture
            .send(
                Method::POST,
                &format!("/mcp?token={token}"),
                &[],
                String::new()
            )
            .await
            .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    fixture.sql("UPDATE mcp_tokens SET expires_at = 0").await;
    assert_eq!(
        fixture.rpc(token, request("ping", json!({}))).await.0,
        StatusCode::UNAUTHORIZED
    );
    fixture
        .sql("UPDATE mcp_tokens SET expires_at = 9999999999; ")
        .await;
    fixture
        .sql(&format!(
            "UPDATE users SET is_disabled = TRUE WHERE id = '{USER_A_ID}'"
        ))
        .await;
    assert_eq!(
        fixture.rpc(token, request("ping", json!({}))).await.0,
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
async fn mcp_protocol_negotiation_notifications_errors_and_tools() {
    let fixture = Fixture::new().await;
    let created = fixture.token(0).await;
    let token = created["token"].as_str().unwrap();
    let init = fixture.rpc(token, request("initialize", json!({"protocolVersion":"2099-01-01","capabilities":{},"clientInfo":{"name":"test","version":"1"}}))).await;
    assert_eq!(init.1["result"]["protocolVersion"], "2025-11-25");
    assert_eq!(
        fixture
            .rpc(
                token,
                json!({"jsonrpc":"2.0","method":"notifications/initialized"})
            )
            .await
            .0,
        StatusCode::ACCEPTED
    );
    assert_eq!(
        fixture.rpc(token, request("ping", json!({}))).await.1["result"],
        json!({})
    );
    let tools = fixture.rpc(token, request("tools/list", json!({}))).await.1;
    assert_eq!(tools["result"]["tools"].as_array().unwrap().len(), 3);
    for method in [
        "resources/list",
        "entries/delete",
        "server/discover",
        "notifications/initialized",
    ] {
        assert_eq!(
            fixture.rpc(token, request(method, json!({}))).await.1["error"]["code"],
            -32601
        );
    }
    for body in [
        json!([]),
        json!({}),
        json!({"jsonrpc":"1.0","id":1,"method":"ping"}),
        json!({"jsonrpc":"2.0","id":false,"method":"ping"}),
        json!({"jsonrpc":"2.0","id":1,"method":"ping","params":[]}),
    ] {
        assert_eq!(fixture.rpc(token, body).await.1["error"]["code"], -32600);
    }
    assert_eq!(
        fixture.rpc(token, request("initialize", json!({}))).await.1["error"]["code"],
        -32602
    );
    assert_eq!(
        fixture
            .send(Method::GET, "/mcp", &[], String::new())
            .await
            .0,
        StatusCode::METHOD_NOT_ALLOWED
    );
}

#[tokio::test]
async fn mcp_http_validation_and_body_bounds() {
    let fixture = Fixture::new().await;
    let created = fixture.token(0).await;
    let auth = format!("Bearer {}", created["token"].as_str().unwrap());
    let base = [
        ("authorization", auth.as_str()),
        ("content-type", "application/json"),
        ("accept", "application/json, text/event-stream"),
        ("mcp-protocol-version", "2025-11-25"),
    ];
    for (name, value, expected) in [
        ("origin", "https://evil.test", StatusCode::FORBIDDEN),
        ("origin", "null", StatusCode::FORBIDDEN),
        (
            "content-type",
            "text/plain",
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
        ),
        ("accept", "application/json", StatusCode::NOT_ACCEPTABLE),
        (
            "mcp-protocol-version",
            "2026-07-28",
            StatusCode::BAD_REQUEST,
        ),
    ] {
        let mut headers = base
            .iter()
            .copied()
            .filter(|(key, _)| *key != name)
            .collect::<Vec<_>>();
        headers.push((name, value));
        assert_eq!(
            fixture
                .send(
                    Method::POST,
                    "/mcp",
                    &headers,
                    request("ping", json!({})).to_string()
                )
                .await
                .0,
            expected
        );
    }
    let mut duplicate = base.to_vec();
    duplicate.push(("authorization", auth.as_str()));
    assert_eq!(
        fixture
            .send(
                Method::POST,
                "/mcp",
                &duplicate,
                request("ping", json!({})).to_string()
            )
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        fixture
            .send(Method::POST, "/mcp", &base, "{".into())
            .await
            .1["error"]["code"],
        -32700
    );
    assert_eq!(
        fixture
            .send(Method::POST, "/mcp", &base, " ".repeat(65537))
            .await
            .0,
        StatusCode::PAYLOAD_TOO_LARGE
    );
}

#[tokio::test]
async fn mcp_tools_enforce_owner_and_argument_bounds() {
    let fixture = Fixture::new().await;
    let now = time::OffsetDateTime::now_utc();
    insert_feed(&fixture.database, now).await;
    insert_subscription(&fixture.database, SUBSCRIPTION_A_ID, USER_A_ID, now).await;
    insert_entry(
        &fixture.database,
        ENTRY_A_ID,
        2,
        "entry-a",
        HASH_A,
        None,
        now,
    )
    .await;
    fixture
        .sql("UPDATE rss_counters SET value = 1 WHERE key = 'INGEST_GENERATION'")
        .await;
    let token_a = fixture.token(0).await;
    let token_b = fixture.token(1).await;
    for (token, expected) in [(&token_a, 1), (&token_b, 0)] {
        for name in ["list_subscriptions", "list_entries"] {
            let result = fixture
                .rpc(
                    token["token"].as_str().unwrap(),
                    request("tools/call", json!({"name":name,"arguments":{}})),
                )
                .await
                .1;
            let value: Value =
                serde_json::from_str(result["result"]["content"][0]["text"].as_str().unwrap())
                    .unwrap();
            assert_eq!(value["items"].as_array().unwrap().len(), expected);
        }
    }
    let result = fixture
        .rpc(
            token_a["token"].as_str().unwrap(),
            request(
                "tools/call",
                json!({"name":"get_entry","arguments":{"entryId":ENTRY_A_ID}}),
            ),
        )
        .await
        .1;
    assert_eq!(result["result"]["isError"], false);
    assert!(
        result["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("Safe content")
    );
    assert!(!result.to_string().contains("<p>"));
    assert_eq!(
        fixture
            .rpc(
                token_b["token"].as_str().unwrap(),
                request(
                    "tools/call",
                    json!({"name":"get_entry","arguments":{"entryId":ENTRY_A_ID}})
                )
            )
            .await
            .1["result"]["isError"],
        true
    );
    for (name, args) in [
        ("get_entry", json!({"entryId":"bad"})),
        ("get_entry", json!({})),
        ("list_entries", json!({"limit":0})),
        ("list_entries", json!({"limit":51})),
        ("list_entries", json!({"limit":1.5})),
        ("list_entries", json!({"feedId":"bad"})),
        ("list_entries", json!({"state":"bad"})),
        ("list_entries", json!({"cursor":"x".repeat(1025)})),
        ("list_subscriptions", json!({"url":"https://example.org"})),
        ("shell", json!({"command":"pwd"})),
    ] {
        assert_eq!(
            fixture
                .rpc(
                    token_a["token"].as_str().unwrap(),
                    request("tools/call", json!({"name":name,"arguments":args}))
                )
                .await
                .1["error"]["code"],
            -32602
        );
    }
    let foreign = fixture
        .rpc(
            token_b["token"].as_str().unwrap(),
            request(
                "tools/call",
                json!({"name":"list_entries","arguments":{"feedId":FEED_ID}}),
            ),
        )
        .await
        .1;
    let value: Value =
        serde_json::from_str(foreign["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(value["items"], json!([]));
}

#[tokio::test]
async fn mcp_request_budget_is_bounded() {
    let fixture = Fixture::new().await;
    let token = fixture.token(0).await;
    for _ in 0..120 {
        assert_eq!(
            fixture
                .rpc(token["token"].as_str().unwrap(), request("ping", json!({})))
                .await
                .0,
            StatusCode::OK
        );
    }
    assert_eq!(
        fixture
            .rpc(token["token"].as_str().unwrap(), request("ping", json!({})))
            .await
            .0,
        StatusCode::TOO_MANY_REQUESTS
    );
    let other = fixture.token(1).await;
    assert_eq!(
        fixture
            .rpc(other["token"].as_str().unwrap(), request("ping", json!({})))
            .await
            .0,
        StatusCode::OK
    );
}

#[tokio::test]
async fn mcp_invalid_auth_cannot_starve_valid_users() {
    let fixture = Fixture::new().await;
    let token = fixture.token(0).await;
    for _ in 0..4100 {
        assert_eq!(
            fixture.rpc("invalid", request("ping", json!({}))).await.0,
            StatusCode::UNAUTHORIZED
        );
    }
    assert_eq!(
        fixture
            .rpc(token["token"].as_str().unwrap(), request("ping", json!({})))
            .await
            .0,
        StatusCode::OK
    );
}

#[tokio::test]
async fn mcp_emergency_revocation_is_not_limited_by_creation_budget() {
    let fixture = Fixture::new().await;
    let created = fixture.token(0).await;
    let original = created["item"]["id"].as_str().unwrap();
    for _ in 0..29 {
        assert_eq!(
            fixture
                .manage(
                    Method::POST,
                    "/api/v1/mcp/tokens",
                    0,
                    json!({"name":"","expiresInDays":30})
                )
                .await
                .0,
            StatusCode::UNPROCESSABLE_ENTITY
        );
    }
    assert_eq!(
        fixture.token_attempt(0).await,
        StatusCode::TOO_MANY_REQUESTS
    );
    for index in 1..50 {
        fixture.sql(&format!("INSERT INTO mcp_tokens (id,user_id,name,prefix,token_hash,created_at,expires_at) SELECT '{index}',user_id,name,prefix,'test-hash-{index}',created_at,expires_at FROM mcp_tokens WHERE id = '{original}'")).await;
    }
    let items = fixture
        .manage(Method::GET, "/api/v1/mcp/tokens", 0, Value::Null)
        .await
        .1;
    assert_eq!(items["items"].as_array().unwrap().len(), 50);
    for item in items["items"].as_array().unwrap() {
        let path = format!("/api/v1/mcp/tokens/{}", item["id"].as_str().unwrap());
        assert_eq!(
            fixture
                .manage(Method::DELETE, &path, 0, Value::Null)
                .await
                .0,
            StatusCode::NO_CONTENT
        );
    }
    assert_eq!(
        fixture
            .manage(Method::GET, "/api/v1/mcp/tokens", 0, Value::Null)
            .await
            .1,
        json!({"items":[]})
    );
}

#[tokio::test]
async fn mcp_token_cap_is_atomic_per_owner_and_revocation_frees_a_slot() {
    let fixture = Fixture::new().await;
    let created = fixture.token(0).await;
    let original = created["item"]["id"].as_str().unwrap();
    for index in 1..49 {
        fixture.sql(&format!("INSERT INTO mcp_tokens (id,user_id,name,prefix,token_hash,created_at,expires_at) SELECT '{index}',user_id,name,prefix,'test-hash-{index}',created_at,expires_at FROM mcp_tokens WHERE id = '{original}'")).await;
    }
    let (first, second) = tokio::join!(fixture.token_attempt(0), fixture.token_attempt(0));
    let mut statuses = [first, second];
    statuses.sort();
    assert_eq!(statuses, [StatusCode::OK, StatusCode::CONFLICT]);
    assert_eq!(
        fixture
            .manage(Method::GET, "/api/v1/mcp/tokens", 0, Value::Null)
            .await
            .1["items"]
            .as_array()
            .unwrap()
            .len(),
        50
    );
    assert_eq!(fixture.token_attempt(1).await, StatusCode::OK);
    assert_eq!(
        fixture
            .manage(
                Method::DELETE,
                &format!("/api/v1/mcp/tokens/{original}"),
                0,
                Value::Null
            )
            .await
            .0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(fixture.token_attempt(0).await, StatusCode::OK);
}

impl Fixture {
    async fn token_attempt(&self, user: usize) -> StatusCode {
        self.manage(
            Method::POST,
            "/api/v1/mcp/tokens",
            user,
            json!({"name":"x".repeat(80),"expiresInDays":30}),
        )
        .await
        .0
    }
}

#[tokio::test]
async fn postgres_mcp_token_backend_contract() {
    let Ok(url) = std::env::var("RAINDROP_TEST_POSTGRES_URL") else {
        eprintln!("PostgreSQL MCP contract skipped: test database URL is not configured");
        return;
    };
    backend_contract(url).await;
}

#[tokio::test]
async fn mysql_mcp_token_backend_contract() {
    let Ok(url) = std::env::var("RAINDROP_TEST_MYSQL_URL") else {
        eprintln!("MySQL MCP contract skipped: test database URL is not configured");
        return;
    };
    backend_contract(url).await;
}

async fn backend_contract(url: String) {
    let fixture = Fixture::with_url(Some(url)).await;
    let created = fixture.token(0).await;
    let token = created["token"].as_str().unwrap();
    for _ in 0..2 {
        assert_eq!(
            fixture.rpc(token, request("ping", json!({}))).await.0,
            StatusCode::OK
        );
    }
    assert_eq!(
        fixture
            .manage(Method::GET, "/api/v1/mcp/tokens", 1, Value::Null)
            .await
            .1,
        json!({"items":[]})
    );
    let path = format!(
        "/api/v1/mcp/tokens/{}",
        created["item"]["id"].as_str().unwrap()
    );
    assert_eq!(
        fixture
            .manage(Method::DELETE, &path, 1, Value::Null)
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        fixture
            .manage(Method::DELETE, &path, 0, Value::Null)
            .await
            .0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        fixture.rpc(token, request("ping", json!({}))).await.0,
        StatusCode::UNAUTHORIZED
    );
    raindrop::db::rollback(&fixture.database).await.unwrap();
    migrate(&fixture.database).await.unwrap();
}

#[tokio::test]
async fn mcp_entry_content_is_inert_bounded_and_marked_untrusted() {
    use sea_orm::{ActiveModelTrait, ActiveValue::Set};
    let fixture = Fixture::new().await;
    let now = time::OffsetDateTime::now_utc();
    insert_feed(&fixture.database, now).await;
    insert_subscription(&fixture.database, SUBSCRIPTION_A_ID, USER_A_ID, now).await;
    let mut entry =
        support::database::entry_model(ENTRY_A_ID, 2, "bounded-entry", HASH_A, None, now);
    entry.title = Set(Some("<script>bad()</script><b>Safe title</b>".into()));
    entry.sanitized_content = Set(format!(
        "rdsc:v1:{}",
        json!({"html":format!("<p>{}</p>","Untrusted article data. ".repeat(3000)),"inertImages":[]})
    ));
    entry.insert(&fixture.database).await.unwrap();
    let token = fixture.token(0).await;
    let result = fixture
        .rpc(
            token["token"].as_str().unwrap(),
            request(
                "tools/call",
                json!({"name":"get_entry","arguments":{"entryId":ENTRY_A_ID}}),
            ),
        )
        .await
        .1;
    let data: Value =
        serde_json::from_str(result["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(data["title"], "Safe title");
    assert_eq!(data["content"].as_str().unwrap().chars().count(), 32768);
    assert!(!data["content"].as_str().unwrap().contains('<'));
    let tools = fixture
        .rpc(
            token["token"].as_str().unwrap(),
            request("tools/list", json!({})),
        )
        .await
        .1;
    let entry_tool = tools["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .find(|tool| tool["name"] == "get_entry")
        .unwrap();
    assert!(
        entry_tool["description"]
            .as_str()
            .unwrap()
            .contains("untrusted")
    );
    assert_eq!(entry_tool["annotations"]["readOnlyHint"], true);
    assert_eq!(entry_tool["annotations"]["openWorldHint"], false);
}

#[tokio::test]
async fn mcp_missing_version_is_only_allowed_for_initialization() {
    let fixture = Fixture::new().await;
    let token = fixture.token(0).await;
    let auth = format!("Bearer {}", token["token"].as_str().unwrap());
    let headers = [
        ("authorization", auth.as_str()),
        ("content-type", "application/json"),
        ("accept", "application/json, text/event-stream"),
    ];
    assert_eq!(
        fixture
            .send(
                Method::POST,
                "/mcp",
                &headers,
                request("ping", json!({})).to_string()
            )
            .await
            .0,
        StatusCode::BAD_REQUEST
    );
    let init = request(
        "initialize",
        json!({"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"test","version":"1"}}),
    );
    assert_eq!(
        fixture
            .send(Method::POST, "/mcp", &headers, init.to_string())
            .await
            .1["result"]["protocolVersion"],
        "2025-11-25"
    );
    assert_eq!(
        fixture
            .send(
                Method::GET,
                "/mcp",
                &[("origin", "https://evil.test")],
                String::new()
            )
            .await
            .0,
        StatusCode::FORBIDDEN
    );
}
