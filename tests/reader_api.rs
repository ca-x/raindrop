#[allow(dead_code)]
mod support;

use axum::{
    Router,
    body::Body,
    http::{
        Method, Request, StatusCode,
        header::{CACHE_CONTROL, CONTENT_TYPE, COOKIE, HOST, ORIGIN, PRAGMA},
    },
};
use http_body_util::BodyExt;
use raindrop::{
    app::{AppState, build_router},
    auth::build_session_cookie,
    db::{
        DatabaseConfig, connect,
        entities::{category, entry, entry_state, feed, rss_counter, session, subscription, user},
        migrate,
    },
    setup::SetupService,
};
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, DatabaseConnection, EntityTrait,
    IntoActiveModel, QueryFilter,
};
use secrecy::{ExposeSecret, SecretString};
use serde_json::{Value, json};
use tempfile::TempDir;
use time::OffsetDateTime;
use tower::ServiceExt;

use support::database::{
    ENTRY_A_ID, ENTRY_B_ID, FEED_ID, HASH_A, HASH_B, HASH_C, SUBSCRIPTION_A_ID, SUBSCRIPTION_B_ID,
    USER_A_ID, USER_B_ID, entry_model, insert_feed, insert_user, subscription_model,
};

const CROSS_TENANT_FEED_ID: &str = "00000000-0000-4000-8000-000000000102";
const CROSS_TENANT_SUBSCRIPTION_ID: &str = "00000000-0000-4000-8000-000000000203";
const CROSS_TENANT_ENTRY_ID: &str = "00000000-0000-4000-8000-000000000303";
const CATEGORY_A_ID: &str = "00000000-0000-4000-8000-000000000501";
const CATEGORY_A_OTHER_ID: &str = "00000000-0000-4000-8000-000000000502";
const CATEGORY_B_ID: &str = "00000000-0000-4000-8000-000000000503";

struct ReaderFixture {
    _data: TempDir,
    app: Router,
    database: DatabaseConnection,
    user_a_cookie: String,
    user_a_csrf: String,
    user_b_cookie: String,
    user_b_csrf: String,
}

#[derive(Clone, Copy)]
enum UserKind {
    A,
    B,
}

impl ReaderFixture {
    async fn new() -> Self {
        let data = tempfile::tempdir().expect("temporary directory should be created");
        let database_url = format!(
            "sqlite://{}?mode=rwc",
            data.path().join("reader-api.db").display()
        );
        let database = connect(&DatabaseConfig::new(SecretString::from(database_url)))
            .await
            .expect("reader database should connect");
        migrate(&database)
            .await
            .expect("reader database should migrate");

        let now = OffsetDateTime::now_utc();
        insert_user(&database, USER_A_ID, "reader-a").await;
        insert_user(&database, USER_B_ID, "reader-b").await;
        insert_feed(&database, now).await;

        for (id, user_id, title, position) in [
            (CATEGORY_A_ID, USER_A_ID, "Technology", 1024),
            (CATEGORY_A_OTHER_ID, USER_A_ID, "Science", 2048),
            (CATEGORY_B_ID, USER_B_ID, "Private", 1024),
        ] {
            category::ActiveModel {
                id: Set(id.to_owned()),
                user_id: Set(user_id.to_owned()),
                title: Set(title.to_owned()),
                normalized_title: Set(title.to_lowercase()),
                position: Set(position),
                created_at: Set(now),
                updated_at: Set(now),
            }
            .insert(&database)
            .await
            .expect("reader category should insert");
        }

        let mut subscription_a = subscription_model(SUBSCRIPTION_A_ID, USER_A_ID, now);
        subscription_a.category_id = Set(Some(CATEGORY_A_ID.to_owned()));
        subscription_a.start_sequence = Set(0);
        subscription_a
            .insert(&database)
            .await
            .expect("user A subscription should insert");
        let mut subscription_b = subscription_model(SUBSCRIPTION_B_ID, USER_B_ID, now);
        subscription_b.category_id = Set(Some(CATEGORY_B_ID.to_owned()));
        subscription_b.start_sequence = Set(1);
        subscription_b
            .insert(&database)
            .await
            .expect("user B subscription should insert");

        entry_model(
            ENTRY_A_ID,
            1,
            "entry-a",
            HASH_A,
            Some(1_784_246_400_000_000),
            now,
        )
        .insert(&database)
        .await
        .expect("entry A should insert");
        entry_model(
            ENTRY_B_ID,
            2,
            "entry-b",
            HASH_B,
            Some(1_784_246_500_000_000),
            now,
        )
        .insert(&database)
        .await
        .expect("entry B should insert");
        entry_state::ActiveModel {
            user_id: Set(USER_B_ID.to_owned()),
            entry_id: Set(ENTRY_B_ID.to_owned()),
            feed_id: Set(FEED_ID.to_owned()),
            feed_sequence: Set(2),
            read_override: Set(Some(true)),
            is_starred: Set(true),
            starred_at: Set(Some(now)),
            revision: Set(1),
            updated_at: Set(now),
        }
        .insert(&database)
        .await
        .expect("sparse reader state should insert");
        rss_counter::ActiveModel {
            key: Set("INGEST_GENERATION".to_owned()),
            value: Set(1),
        }
        .update(&database)
        .await
        .expect("ingestion generation should update");

        let setup = SetupService::ready(data.path(), None, database.clone());
        let user_a_session = setup
            .sessions()
            .create(USER_A_ID)
            .await
            .expect("user A session should create");
        let user_b_session = setup
            .sessions()
            .create(USER_B_ID)
            .await
            .expect("user B session should create");
        let user_a_cookie = session_cookie(&user_a_session);
        let user_a_csrf = user_a_session.csrf_token.expose_secret().to_owned();
        let user_b_cookie = session_cookie(&user_b_session);
        let user_b_csrf = user_b_session.csrf_token.expose_secret().to_owned();
        let app = build_router(AppState::new(setup));

        Self {
            _data: data,
            app,
            database,
            user_a_cookie,
            user_a_csrf,
            user_b_cookie,
            user_b_csrf,
        }
    }

    async fn request_with_csrf(
        &self,
        method: Method,
        uri: &str,
        body: Value,
        user: UserKind,
        origin: Option<&str>,
        host: Option<&str>,
    ) -> axum::response::Response {
        let (cookie, csrf) = match user {
            UserKind::A => (&self.user_a_cookie, &self.user_a_csrf),
            UserKind::B => (&self.user_b_cookie, &self.user_b_csrf),
        };
        let mut request = Request::builder()
            .method(method)
            .uri(uri)
            .header(COOKIE, cookie)
            .header("x-csrf-token", csrf)
            .header(CONTENT_TYPE, "application/json");
        if let Some(origin) = origin {
            request = request.header(ORIGIN, origin);
        }
        if let Some(host) = host {
            request = request.header(HOST, host);
        }
        let request = request
            .body(Body::from(body.to_string()))
            .expect("reader state request should build");
        self.app
            .clone()
            .oneshot(request)
            .await
            .expect("reader state request should complete")
    }

    async fn request_state_body(
        &self,
        body: &str,
        content_type: Option<&str>,
        user: UserKind,
    ) -> axum::response::Response {
        let (cookie, csrf) = match user {
            UserKind::A => (&self.user_a_cookie, &self.user_a_csrf),
            UserKind::B => (&self.user_b_cookie, &self.user_b_csrf),
        };
        let mut request = Request::builder()
            .method(Method::PATCH)
            .uri(format!("/api/v1/entries/{ENTRY_A_ID}/state"))
            .header(COOKIE, cookie)
            .header("x-csrf-token", csrf)
            .header(ORIGIN, "http://reader.test")
            .header(HOST, "reader.test");
        if let Some(content_type) = content_type {
            request = request.header(CONTENT_TYPE, content_type);
        }
        let request = request
            .body(Body::from(body.to_owned()))
            .expect("reader state body request should build");
        self.app
            .clone()
            .oneshot(request)
            .await
            .expect("reader state body request should complete")
    }

    async fn request_mark_read_body(
        &self,
        body: &str,
        content_type: Option<&str>,
        include_csrf: bool,
    ) -> axum::response::Response {
        let mut request = Request::builder()
            .method(Method::POST)
            .uri("/api/v1/entries/mark-read")
            .header(COOKIE, &self.user_a_cookie)
            .header(ORIGIN, "http://reader.test")
            .header(HOST, "reader.test");
        if include_csrf {
            request = request.header("x-csrf-token", &self.user_a_csrf);
        }
        if let Some(content_type) = content_type {
            request = request.header(CONTENT_TYPE, content_type);
        }
        let request = request
            .body(Body::from(body.to_owned()))
            .expect("bulk read body request should build");
        self.app
            .clone()
            .oneshot(request)
            .await
            .expect("bulk read body request should complete")
    }

    async fn insert_cross_tenant_entry(&self) {
        let now = OffsetDateTime::now_utc();
        let mut cross_feed = feed::Entity::find_by_id(FEED_ID)
            .one(&self.database)
            .await
            .expect("source feed should query")
            .expect("source feed should exist")
            .into_active_model();
        cross_feed.id = Set(CROSS_TENANT_FEED_ID.to_owned());
        cross_feed.source_url = Set("https://cross-tenant.example/feed.xml".to_owned());
        cross_feed.normalized_url = Set("https://cross-tenant.example/feed.xml".to_owned());
        cross_feed.normalized_url_hash = Set(HASH_C.to_owned());
        cross_feed.fetch_url = Set("https://cross-tenant.example/feed.xml".to_owned());
        cross_feed.validator_url = Set(Some("https://cross-tenant.example/feed.xml".to_owned()));
        cross_feed
            .insert(&self.database)
            .await
            .expect("cross-tenant feed should insert");

        let mut subscription = subscription_model(CROSS_TENANT_SUBSCRIPTION_ID, USER_B_ID, now);
        subscription.feed_id = Set(CROSS_TENANT_FEED_ID.to_owned());
        subscription.start_sequence = Set(0);
        subscription
            .insert(&self.database)
            .await
            .expect("cross-tenant subscription should insert");

        let mut cross_entry = entry_model(
            CROSS_TENANT_ENTRY_ID,
            1,
            "cross-tenant-entry",
            HASH_C,
            None,
            now,
        );
        cross_entry.feed_id = Set(CROSS_TENANT_FEED_ID.to_owned());
        cross_entry
            .insert(&self.database)
            .await
            .expect("cross-tenant entry should insert");
    }

    async fn request_state_without_session(&self) -> axum::response::Response {
        let request = Request::builder()
            .method(Method::PATCH)
            .uri("/api/v1/entries/not-a-uuid/state")
            .header("x-csrf-token", "not-a-token")
            .header(CONTENT_TYPE, "application/json")
            .header(ORIGIN, "http://reader.test")
            .header(HOST, "reader.test")
            .body(Body::from("{}"))
            .expect("reader state unauthenticated request should build");
        self.app
            .clone()
            .oneshot(request)
            .await
            .expect("reader state unauthenticated request should complete")
    }

    async fn expire_user_a_session(&self) {
        let stored = session::Entity::find()
            .filter(session::Column::UserId.eq(USER_A_ID))
            .one(&self.database)
            .await
            .expect("user A session should query")
            .expect("user A session should exist");
        let mut active = stored.into_active_model();
        active.expires_at = Set(OffsetDateTime::now_utc() - time::Duration::seconds(1));
        active
            .update(&self.database)
            .await
            .expect("user A session should expire");
    }

    async fn disable_user_a(&self) {
        let stored = user::Entity::find_by_id(USER_A_ID)
            .one(&self.database)
            .await
            .expect("user A should query")
            .expect("user A should exist");
        let mut active = stored.into_active_model();
        active.is_disabled = Set(true);
        active
            .update(&self.database)
            .await
            .expect("user A should disable");
    }

    async fn request_state_with_csrf_headers(
        &self,
        csrf_headers: &[&str],
    ) -> axum::response::Response {
        let mut request = Request::builder()
            .method(Method::PATCH)
            .uri("/api/v1/entries/not-a-uuid/state")
            .header(COOKIE, &self.user_a_cookie)
            .header(CONTENT_TYPE, "application/json")
            .header(ORIGIN, "http://reader.test")
            .header(HOST, "reader.test");
        for csrf in csrf_headers {
            request = request.header("x-csrf-token", *csrf);
        }
        let request = request
            .body(Body::from("{}"))
            .expect("reader state CSRF request should build");
        self.app
            .clone()
            .oneshot(request)
            .await
            .expect("reader state CSRF request should complete")
    }

    async fn request_state_with_origin_headers(
        &self,
        origin_headers: &[&str],
        host: Option<&str>,
    ) -> axum::response::Response {
        let mut request = Request::builder()
            .method(Method::PATCH)
            .uri(format!("/api/v1/entries/{ENTRY_A_ID}/state"))
            .header(COOKIE, &self.user_a_cookie)
            .header("x-csrf-token", &self.user_a_csrf)
            .header(CONTENT_TYPE, "application/json");
        for origin in origin_headers {
            request = request.header(ORIGIN, *origin);
        }
        if let Some(host) = host {
            request = request.header(HOST, host);
        }
        let request = request
            .body(Body::from(json!({ "isRead": true }).to_string()))
            .expect("reader state origin request should build");
        self.app
            .clone()
            .oneshot(request)
            .await
            .expect("reader state origin request should complete")
    }

    async fn close_database(&self) {
        self.database
            .clone()
            .close()
            .await
            .expect("reader database should close");
    }

    async fn request(
        &self,
        method: Method,
        uri: &str,
        body: Option<Value>,
        user: UserKind,
    ) -> axum::response::Response {
        let cookie = match user {
            UserKind::A => &self.user_a_cookie,
            UserKind::B => &self.user_b_cookie,
        };
        let request = Request::builder()
            .method(method)
            .uri(uri)
            .header(COOKIE, cookie)
            .body(body.map_or_else(Body::empty, |value| Body::from(value.to_string())))
            .expect("reader request should build");
        self.app
            .clone()
            .oneshot(request)
            .await
            .expect("reader request should complete")
    }

    async fn request_unauthenticated(&self, method: Method, uri: &str) -> axum::response::Response {
        let request = Request::builder()
            .method(method)
            .uri(uri)
            .body(Body::empty())
            .expect("unauthenticated reader request should build");
        self.app
            .clone()
            .oneshot(request)
            .await
            .expect("unauthenticated reader request should complete")
    }

    async fn replace_entry_storage(
        &self,
        sanitized_content: Option<&str>,
        enclosure_json: Option<&str>,
    ) {
        let mut entry = entry::Entity::find_by_id(ENTRY_A_ID)
            .one(&self.database)
            .await
            .expect("entry should query")
            .expect("entry A should exist")
            .into_active_model();
        if let Some(sanitized_content) = sanitized_content {
            entry.sanitized_content = Set(sanitized_content.to_owned());
        }
        if let Some(enclosure_json) = enclosure_json {
            entry.enclosure_json = Set(Some(enclosure_json.to_owned()));
        }
        entry
            .update(&self.database)
            .await
            .expect("entry storage should update");
    }

    async fn seed_search_projection(&self) {
        for (entry_id, title, author, summary, search_text) in [
            (
                ENTRY_A_ID,
                "Rust Dispatch",
                "Alice",
                "Portable storage",
                "rust dispatch alice portable storage rendered content 100% _ literal γεια common",
            ),
            (
                ENTRY_B_ID,
                "RSS Database",
                "Bob",
                "Second article",
                "rss database bob second article feed parser common",
            ),
        ] {
            let stored = entry::Entity::find_by_id(entry_id)
                .one(&self.database)
                .await
                .expect("search entry should query")
                .expect("search entry should exist");
            let mut active = stored.into_active_model();
            active.title = Set(Some(title.to_owned()));
            active.author = Set(Some(author.to_owned()));
            active.summary = Set(Some(summary.to_owned()));
            active.search_text = Set(search_text.to_owned());
            active
                .update(&self.database)
                .await
                .expect("search projection should update");
        }
    }
}

fn session_cookie(session: &raindrop::auth::CreatedSession) -> String {
    build_session_cookie(session, false)
        .to_string()
        .split(';')
        .next()
        .expect("session cookie should contain a pair")
        .to_owned()
}

async fn response_json(response: axum::response::Response) -> Value {
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("response body should collect")
        .to_bytes();
    serde_json::from_slice(&bytes).expect("response should contain JSON")
}

fn assert_sensitive_cache_headers(response: &axum::response::Response) {
    assert_eq!(response.headers().get(CACHE_CONTROL).unwrap(), "no-store");
    assert_eq!(response.headers().get(PRAGMA).unwrap(), "no-cache");
}

#[tokio::test]
async fn reader_state_patch_updates_only_supplied_fields() {
    let fixture = ReaderFixture::new().await;
    let response = fixture
        .request_with_csrf(
            Method::PATCH,
            &format!("/api/v1/entries/{ENTRY_A_ID}/state"),
            json!({ "isStarred": true }),
            UserKind::A,
            Some("http://reader.test"),
            Some("reader.test"),
        )
        .await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_sensitive_cache_headers(&response);
    let body = response_json(response).await;
    assert_eq!(body["entryId"], ENTRY_A_ID);
    assert_eq!(body["isRead"], false);
    assert_eq!(body["isStarred"], true);
}

#[tokio::test]
async fn reader_state_rejects_invalid_bodies() {
    let fixture = ReaderFixture::new().await;
    for (body, content_type) in [
        ("{}", Some("application/json")),
        (r#"{"isRead":null}"#, Some("application/json")),
        (r#"{"isRead":1}"#, Some("application/json")),
        (r#"{"isRead":"true"}"#, Some("application/json")),
        (
            r#"{"isStarred":true,"unexpected":false}"#,
            Some("application/json"),
        ),
        (r#"{"isRead":true"#, Some("application/json")),
        (r#"{"isRead":true}"#, Some("text/plain")),
    ] {
        let response = fixture
            .request_state_body(body, content_type, UserKind::A)
            .await;
        assert_eq!(
            response.status(),
            StatusCode::UNPROCESSABLE_ENTITY,
            "unexpected status for body {body:?} and content type {content_type:?}"
        );
        let body = response_json(response).await;
        assert_eq!(body["error"]["code"], "VALIDATION_ERROR");
    }
}

#[tokio::test]
async fn reader_state_hides_missing_and_cross_tenant_entries() {
    let fixture = ReaderFixture::new().await;
    fixture.insert_cross_tenant_entry().await;

    let response = fixture
        .request_with_csrf(
            Method::PATCH,
            "/api/v1/entries/not-a-uuid/state",
            json!({ "isRead": true }),
            UserKind::A,
            Some("http://reader.test"),
            Some("reader.test"),
        )
        .await;
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let body = response_json(response).await;
    assert_eq!(body["error"]["code"], "VALIDATION_ERROR");

    let mut envelopes = Vec::new();
    for (entry_id, user) in [
        ("00000000-0000-4000-8000-000000000399", UserKind::A),
        (CROSS_TENANT_ENTRY_ID, UserKind::A),
        (ENTRY_A_ID, UserKind::B),
    ] {
        let response = fixture
            .request_with_csrf(
                Method::PATCH,
                &format!("/api/v1/entries/{entry_id}/state"),
                json!({ "isRead": true }),
                user,
                Some("http://reader.test"),
                Some("reader.test"),
            )
            .await;
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        let body = response_json(response).await;
        envelopes.push((
            body["error"]["code"].clone(),
            body["error"]["message"].clone(),
        ));
    }
    assert!(envelopes.windows(2).all(|pair| pair[0] == pair[1]));
}

#[tokio::test]
async fn reader_state_requires_active_session() {
    let fixture = ReaderFixture::new().await;
    let response = fixture.request_state_without_session().await;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    let body = response_json(response).await;
    assert_eq!(body["error"]["code"], "AUTHENTICATION_REQUIRED");

    let fixture = ReaderFixture::new().await;
    fixture.expire_user_a_session().await;
    let response = fixture
        .request_with_csrf(
            Method::PATCH,
            &format!("/api/v1/entries/{ENTRY_A_ID}/state"),
            json!({ "isRead": true }),
            UserKind::A,
            Some("http://reader.test"),
            Some("reader.test"),
        )
        .await;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    let body = response_json(response).await;
    assert_eq!(body["error"]["code"], "AUTHENTICATION_REQUIRED");

    let fixture = ReaderFixture::new().await;
    fixture.disable_user_a().await;
    let response = fixture
        .request_with_csrf(
            Method::PATCH,
            &format!("/api/v1/entries/{ENTRY_A_ID}/state"),
            json!({ "isRead": true }),
            UserKind::A,
            Some("http://reader.test"),
            Some("reader.test"),
        )
        .await;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    let body = response_json(response).await;
    assert_eq!(body["error"]["code"], "AUTHENTICATION_REQUIRED");
}

#[tokio::test]
async fn reader_state_requires_valid_csrf() {
    let fixture = ReaderFixture::new().await;
    let valid_csrf = fixture.user_a_csrf.as_str();
    for csrf_headers in [
        Vec::new(),
        vec![valid_csrf, valid_csrf],
        vec!["not-a-token"],
        vec!["AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"],
    ] {
        let response = fixture.request_state_with_csrf_headers(&csrf_headers).await;
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        let body = response_json(response).await;
        assert_eq!(body["error"]["code"], "FORBIDDEN");
    }
}

#[tokio::test]
async fn reader_state_enforces_same_origin() {
    let fixture = ReaderFixture::new().await;
    let response = fixture
        .request_state_with_origin_headers(&["http://reader.test"], Some("reader.test"))
        .await;
    assert_eq!(response.status(), StatusCode::OK);

    for (origins, host) in [
        (vec!["http://attacker.test"], Some("reader.test")),
        (vec!["http://reader.test:8080"], Some("reader.test")),
        (vec!["not-an-origin"], Some("reader.test")),
        (
            vec!["http://reader.test", "http://reader.test"],
            Some("reader.test"),
        ),
        (vec!["http://reader.test"], None),
    ] {
        let response = fixture
            .request_state_with_origin_headers(&origins, host)
            .await;
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        let body = response_json(response).await;
        assert_eq!(body["error"]["code"], "FORBIDDEN");
    }
}

#[tokio::test]
async fn reader_state_responses_disable_caching() {
    let fixture = ReaderFixture::new().await;
    let cases = [
        (
            StatusCode::OK,
            fixture
                .request_with_csrf(
                    Method::PATCH,
                    &format!("/api/v1/entries/{ENTRY_A_ID}/state"),
                    json!({ "isStarred": true }),
                    UserKind::A,
                    Some("http://reader.test"),
                    Some("reader.test"),
                )
                .await,
        ),
        (
            StatusCode::UNPROCESSABLE_ENTITY,
            fixture
                .request_state_body("{}", Some("application/json"), UserKind::A)
                .await,
        ),
        (
            StatusCode::UNAUTHORIZED,
            fixture.request_state_without_session().await,
        ),
        (
            StatusCode::FORBIDDEN,
            fixture.request_state_with_csrf_headers(&[]).await,
        ),
        (
            StatusCode::NOT_FOUND,
            fixture
                .request_with_csrf(
                    Method::PATCH,
                    "/api/v1/entries/00000000-0000-4000-8000-000000000399/state",
                    json!({ "isRead": true }),
                    UserKind::A,
                    Some("http://reader.test"),
                    Some("reader.test"),
                )
                .await,
        ),
        (
            StatusCode::METHOD_NOT_ALLOWED,
            fixture
                .request(
                    Method::POST,
                    &format!("/api/v1/entries/{ENTRY_A_ID}/state"),
                    Some(json!({ "isRead": true })),
                    UserKind::A,
                )
                .await,
        ),
    ];
    for (expected_status, response) in cases {
        assert_eq!(response.status(), expected_status);
        assert_sensitive_cache_headers(&response);
    }

    let broken_fixture = ReaderFixture::new().await;
    broken_fixture.close_database().await;
    let response = broken_fixture
        .request_with_csrf(
            Method::PATCH,
            &format!("/api/v1/entries/{ENTRY_A_ID}/state"),
            json!({ "isRead": true }),
            UserKind::A,
            Some("http://reader.test"),
            Some("reader.test"),
        )
        .await;
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert_sensitive_cache_headers(&response);
}

#[tokio::test]
async fn reader_list_defaults_to_unread_and_returns_a_user_bound_cursor() {
    let fixture = ReaderFixture::new().await;
    let response = fixture
        .request(Method::GET, "/api/v1/entries?limit=1", None, UserKind::A)
        .await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_sensitive_cache_headers(&response);
    let body = response_json(response).await;
    assert_eq!(body["items"].as_array().unwrap().len(), 1);
    assert_eq!(body["items"][0]["isRead"], false);
    assert!(body["nextCursor"].is_string());
    assert_eq!(body["snapshotGeneration"], 1);

    let cursor = body["nextCursor"].as_str().unwrap();
    let response = fixture
        .request(
            Method::GET,
            &format!("/api/v1/entries?limit=1&cursor={cursor}"),
            None,
            UserKind::B,
        )
        .await;
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn reader_category_filter_is_user_scoped_and_cursor_bound() {
    let fixture = ReaderFixture::new().await;
    let response = fixture
        .request(
            Method::GET,
            &format!("/api/v1/entries?state=ALL&limit=1&categoryId={CATEGORY_A_ID}"),
            None,
            UserKind::A,
        )
        .await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = response_json(response).await;
    assert_eq!(body["items"].as_array().unwrap().len(), 1);
    assert!(body["nextCursor"].is_string());
    let cursor = body["nextCursor"].as_str().unwrap();

    let replay = fixture
        .request(
            Method::GET,
            &format!(
                "/api/v1/entries?state=ALL&limit=1&categoryId={CATEGORY_A_OTHER_ID}&cursor={cursor}"
            ),
            None,
            UserKind::A,
        )
        .await;
    assert_eq!(replay.status(), StatusCode::UNPROCESSABLE_ENTITY);

    let cross_user = fixture
        .request(
            Method::GET,
            &format!("/api/v1/entries?state=ALL&categoryId={CATEGORY_B_ID}"),
            None,
            UserKind::A,
        )
        .await;
    assert_eq!(cross_user.status(), StatusCode::OK);
    assert_eq!(response_json(cross_user).await["items"], json!([]));

    let ambiguous = fixture
        .request(
            Method::GET,
            &format!("/api/v1/entries?state=ALL&feedId={FEED_ID}&categoryId={CATEGORY_A_ID}"),
            None,
            UserKind::A,
        )
        .await;
    assert_eq!(ambiguous.status(), StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn reader_feed_search_matches_portable_projection_and_binds_the_cursor() {
    let fixture = ReaderFixture::new().await;
    fixture.seed_search_projection().await;

    for (query, expected_entry) in [
        ("rust", ENTRY_A_ID),
        ("alice%20storage", ENTRY_A_ID),
        ("100%25%20_", ENTRY_A_ID),
        ("%CE%93%CE%95%CE%99%CE%91", ENTRY_A_ID),
        ("database%20bob", ENTRY_B_ID),
    ] {
        let response = fixture
            .request(
                Method::GET,
                &format!("/api/v1/entries?state=ALL&feedId={FEED_ID}&search={query}"),
                None,
                UserKind::A,
            )
            .await;
        assert_eq!(response.status(), StatusCode::OK, "search query {query}");
        let body = response_json(response).await;
        assert_eq!(body["items"].as_array().unwrap().len(), 1, "{query}");
        assert_eq!(body["items"][0]["entryId"], expected_entry, "{query}");
    }

    let first = fixture
        .request(
            Method::GET,
            &format!("/api/v1/entries?state=ALL&feedId={FEED_ID}&search=common&limit=1"),
            None,
            UserKind::A,
        )
        .await;
    assert_eq!(first.status(), StatusCode::OK);
    let first = response_json(first).await;
    let cursor = first["nextCursor"]
        .as_str()
        .expect("common search should have another page");
    let replay = fixture
        .request(
            Method::GET,
            &format!(
                "/api/v1/entries?state=ALL&feedId={FEED_ID}&search=rust&limit=1&cursor={cursor}"
            ),
            None,
            UserKind::A,
        )
        .await;
    assert_eq!(replay.status(), StatusCode::UNPROCESSABLE_ENTITY);

    let cross_user = fixture
        .request(
            Method::GET,
            &format!("/api/v1/entries?state=ALL&feedId={FEED_ID}&search=rust"),
            None,
            UserKind::B,
        )
        .await;
    assert_eq!(cross_user.status(), StatusCode::OK);
    assert_eq!(response_json(cross_user).await["items"], json!([]));
}

#[tokio::test]
async fn reader_feed_search_rejects_unbounded_or_non_feed_queries() {
    let fixture = ReaderFixture::new().await;
    let too_long = "x".repeat(129);
    for uri in [
        "/api/v1/entries?state=ALL&search=rust".to_owned(),
        format!("/api/v1/entries?state=ALL&categoryId={CATEGORY_A_ID}&search=rust"),
        format!("/api/v1/entries?state=ALL&feedId={FEED_ID}&search=%20"),
        format!(
            "/api/v1/entries?state=ALL&feedId={FEED_ID}&search=one%20two%20three%20four%20five%20six%20seven%20eight%20nine"
        ),
        format!("/api/v1/entries?state=ALL&feedId={FEED_ID}&search={too_long}"),
    ] {
        let response = fixture.request(Method::GET, &uri, None, UserKind::A).await;
        assert_eq!(
            response.status(),
            StatusCode::UNPROCESSABLE_ENTITY,
            "unexpected status for {uri}"
        );
        let body = response_json(response).await;
        assert_eq!(body["error"]["code"], "VALIDATION_ERROR");
        assert!(body["error"]["fields"]["search"].is_string());
    }
}

#[tokio::test]
async fn reader_bulk_mark_read_advances_the_confirmed_snapshot() {
    let fixture = ReaderFixture::new().await;
    let response = fixture
        .request_with_csrf(
            Method::POST,
            "/api/v1/entries/mark-read",
            json!({ "snapshotGeneration": 1, "categoryId": CATEGORY_A_ID }),
            UserKind::A,
            Some("http://reader.test"),
            Some("reader.test"),
        )
        .await;
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    assert_sensitive_cache_headers(&response);
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("bulk read response should collect")
        .to_bytes();
    assert!(bytes.is_empty());

    let subscription = subscription::Entity::find_by_id(SUBSCRIPTION_A_ID)
        .one(&fixture.database)
        .await
        .expect("Subscription should query")
        .expect("Subscription should exist");
    assert_eq!(subscription.read_through_sequence, 2);
    assert_eq!(subscription.state_revision, 1);

    let unread = fixture
        .request(Method::GET, "/api/v1/entries", None, UserKind::A)
        .await;
    assert_eq!(unread.status(), StatusCode::OK);
    assert_eq!(response_json(unread).await["items"], json!([]));
}

#[tokio::test]
async fn reader_bulk_mark_read_rejects_invalid_or_untrusted_requests() {
    let fixture = ReaderFixture::new().await;
    for (body, content_type) in [
        ("{}", Some("application/json")),
        (r#"{"snapshotGeneration":null}"#, Some("application/json")),
        (r#"{"snapshotGeneration":-1}"#, Some("application/json")),
        (r#"{"snapshotGeneration":2}"#, Some("application/json")),
        (
            r#"{"snapshotGeneration":1,"feedId":"not-a-uuid"}"#,
            Some("application/json"),
        ),
        (
            r#"{"snapshotGeneration":1,"categoryId":"not-a-uuid"}"#,
            Some("application/json"),
        ),
        (
            r#"{"snapshotGeneration":1,"feedId":"00000000-0000-4000-8000-000000000101","categoryId":"00000000-0000-4000-8000-000000000501"}"#,
            Some("application/json"),
        ),
        (
            r#"{"snapshotGeneration":1,"unexpected":true}"#,
            Some("application/json"),
        ),
        (r#"{"snapshotGeneration":1}"#, Some("text/plain")),
    ] {
        let response = fixture
            .request_mark_read_body(body, content_type, true)
            .await;
        assert_eq!(
            response.status(),
            StatusCode::UNPROCESSABLE_ENTITY,
            "unexpected status for {body:?}"
        );
        assert_sensitive_cache_headers(&response);
        assert_eq!(
            response_json(response).await["error"]["code"],
            "VALIDATION_ERROR"
        );
    }

    let forbidden = fixture
        .request_mark_read_body(
            r#"{"snapshotGeneration":1}"#,
            Some("application/json"),
            false,
        )
        .await;
    assert_eq!(forbidden.status(), StatusCode::FORBIDDEN);

    let unauthenticated = fixture
        .request_unauthenticated(Method::POST, "/api/v1/entries/mark-read")
        .await;
    assert_eq!(unauthenticated.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn reader_detail_returns_only_sanitized_visible_content() {
    let fixture = ReaderFixture::new().await;
    let response = fixture
        .request(
            Method::GET,
            &format!("/api/v1/entries/{ENTRY_A_ID}"),
            None,
            UserKind::A,
        )
        .await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = response_json(response).await;
    assert_eq!(body["contentHtml"], "<p>Safe content</p>");
    assert!(body["inertImages"].is_array());
    assert!(body["enclosures"].is_array());
    assert!(!body.to_string().contains("<script"));
}

#[tokio::test]
async fn guessed_or_invisible_entry_ids_share_the_same_not_found_contract() {
    let fixture = ReaderFixture::new().await;
    let mut envelopes = Vec::new();
    for entry_id in [ENTRY_A_ID, "00000000-0000-4000-8000-000000000399"] {
        let response = fixture
            .request(
                Method::GET,
                &format!("/api/v1/entries/{entry_id}"),
                None,
                UserKind::B,
            )
            .await;
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        let body = response_json(response).await;
        envelopes.push((
            body["error"]["code"].clone(),
            body["error"]["message"].clone(),
        ));
    }
    assert_eq!(envelopes[0], envelopes[1]);
}

#[tokio::test]
async fn reader_list_rejects_invalid_query_contract() {
    let fixture = ReaderFixture::new().await;
    for uri in [
        "/api/v1/entries?limit=0",
        "/api/v1/entries?limit=101",
        "/api/v1/entries?state=unread",
        "/api/v1/entries?feedId=not-a-uuid",
        "/api/v1/entries?cursor=not-a-cursor",
        "/api/v1/entries?limit=1&limit=2",
        "/api/v1/entries?state=ALL&state=UNREAD",
        "/api/v1/entries?feedId=00000000-0000-4000-8000-000000000101&feedId=00000000-0000-4000-8000-000000000102",
        "/api/v1/entries?cursor=first&cursor=second",
        "/api/v1/entries?categoryId=not-a-uuid",
    ] {
        let response = fixture.request(Method::GET, uri, None, UserKind::A).await;
        assert_eq!(
            response.status(),
            StatusCode::UNPROCESSABLE_ENTITY,
            "unexpected status for {uri}"
        );
        let body = response_json(response).await;
        assert_eq!(
            body["error"]["code"], "VALIDATION_ERROR",
            "unexpected envelope for {uri}"
        );
    }
}

#[tokio::test]
async fn reader_routes_require_authentication() {
    let fixture = ReaderFixture::new().await;
    for uri in [
        "/api/v1/entries",
        "/api/v1/entries/00000000-0000-4000-8000-000000000301",
    ] {
        let response = fixture.request_unauthenticated(Method::GET, uri).await;
        assert_eq!(
            response.status(),
            StatusCode::UNAUTHORIZED,
            "unexpected status for {uri}"
        );
        let body = response_json(response).await;
        assert_eq!(body["error"]["code"], "AUTHENTICATION_REQUIRED");
    }
}

#[tokio::test]
async fn reader_routes_redact_corrupt_persisted_content() {
    let cases = [
        (
            Some("corrupt-sanitized-content-sentinel-18d7"),
            None,
            "corrupt-sanitized-content-sentinel-18d7",
        ),
        (
            None,
            Some("corrupt-enclosure-sentinel-6a2f"),
            "corrupt-enclosure-sentinel-6a2f",
        ),
    ];
    for (sanitized_content, enclosure_json, sentinel) in cases {
        let fixture = ReaderFixture::new().await;
        fixture
            .replace_entry_storage(sanitized_content, enclosure_json)
            .await;
        let response = fixture
            .request(
                Method::GET,
                &format!("/api/v1/entries/{ENTRY_A_ID}"),
                None,
                UserKind::A,
            )
            .await;
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
        let body = response_json(response).await;
        assert_eq!(body["error"]["code"], "INTERNAL_ERROR");
        assert!(!body.to_string().contains(sentinel));
    }
}

#[tokio::test]
async fn reader_unknown_paths_return_json_not_found() {
    let fixture = ReaderFixture::new().await;
    for uri in [
        "/api/v1/entries/",
        "/api/v1/entries/00000000-0000-4000-8000-000000000301/unexpected",
    ] {
        let response = fixture.request(Method::GET, uri, None, UserKind::A).await;
        assert_eq!(
            response.status(),
            StatusCode::NOT_FOUND,
            "unexpected status for {uri}"
        );
        let body = response_json(response).await;
        assert_eq!(body["error"]["code"], "NOT_FOUND");
    }
}

#[tokio::test]
async fn reader_known_paths_reject_wrong_methods() {
    let fixture = ReaderFixture::new().await;
    for (method, uri) in [
        (Method::POST, "/api/v1/entries"),
        (Method::GET, "/api/v1/entries/mark-read"),
        (
            Method::PUT,
            "/api/v1/entries/00000000-0000-4000-8000-000000000301",
        ),
        (
            Method::POST,
            "/api/v1/entries/00000000-0000-4000-8000-000000000301/state",
        ),
    ] {
        let response = fixture.request(method, uri, None, UserKind::A).await;
        assert_eq!(
            response.status(),
            StatusCode::METHOD_NOT_ALLOWED,
            "unexpected status for {uri}"
        );
        let body = response_json(response).await;
        assert_eq!(body["error"]["code"], "METHOD_NOT_ALLOWED");
    }
}

#[tokio::test]
async fn reader_responses_disable_caching() {
    let fixture = ReaderFixture::new().await;
    let authenticated_cases = [
        (Method::GET, "/api/v1/entries"),
        (
            Method::GET,
            "/api/v1/entries/00000000-0000-4000-8000-000000000301",
        ),
        (Method::GET, "/api/v1/entries?limit=0"),
        (
            Method::GET,
            "/api/v1/entries/00000000-0000-4000-8000-000000000399",
        ),
        (Method::GET, "/api/v1/entries/"),
        (
            Method::GET,
            "/api/v1/entries/00000000-0000-4000-8000-000000000301/unexpected",
        ),
        (Method::POST, "/api/v1/entries"),
        (
            Method::PUT,
            "/api/v1/entries/00000000-0000-4000-8000-000000000301",
        ),
    ];
    for (method, uri) in authenticated_cases {
        let response = fixture.request(method, uri, None, UserKind::A).await;
        assert!(
            response.headers().contains_key(PRAGMA),
            "missing Pragma for {uri} with status {}",
            response.status()
        );
        assert_sensitive_cache_headers(&response);
    }

    for uri in [
        "/api/v1/entries",
        "/api/v1/entries/00000000-0000-4000-8000-000000000301",
    ] {
        let response = fixture.request_unauthenticated(Method::GET, uri).await;
        assert_sensitive_cache_headers(&response);
    }

    let corrupt_fixture = ReaderFixture::new().await;
    corrupt_fixture
        .replace_entry_storage(Some("corrupt-cache-case-sentinel-26b1"), None)
        .await;
    let response = corrupt_fixture
        .request(
            Method::GET,
            "/api/v1/entries/00000000-0000-4000-8000-000000000301",
            None,
            UserKind::A,
        )
        .await;
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert_sensitive_cache_headers(&response);
}
