#[allow(dead_code)]
mod support;

use axum::{
    Router,
    body::Body,
    http::{
        Method, Request, StatusCode,
        header::{CACHE_CONTROL, COOKIE, PRAGMA},
    },
};
use http_body_util::BodyExt;
use raindrop::{
    app::{AppState, build_router},
    auth::build_session_cookie,
    db::{
        DatabaseConfig, connect,
        entities::{entry, entry_state, rss_counter},
        migrate,
    },
    setup::SetupService,
};
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, DatabaseConnection, EntityTrait, IntoActiveModel,
};
use secrecy::SecretString;
use serde_json::Value;
use tempfile::TempDir;
use time::OffsetDateTime;
use tower::ServiceExt;

use support::database::{
    ENTRY_A_ID, ENTRY_B_ID, FEED_ID, HASH_A, HASH_B, SUBSCRIPTION_A_ID, SUBSCRIPTION_B_ID,
    USER_A_ID, USER_B_ID, entry_model, insert_feed, insert_user, subscription_model,
};

struct ReaderFixture {
    _data: TempDir,
    app: Router,
    database: DatabaseConnection,
    user_a_cookie: String,
    user_b_cookie: String,
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

        let mut subscription_a = subscription_model(SUBSCRIPTION_A_ID, USER_A_ID, now);
        subscription_a.start_sequence = Set(0);
        subscription_a
            .insert(&database)
            .await
            .expect("user A subscription should insert");
        let mut subscription_b = subscription_model(SUBSCRIPTION_B_ID, USER_B_ID, now);
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
        let user_b_cookie = session_cookie(&user_b_session);
        let app = build_router(AppState::new(setup));

        Self {
            _data: data,
            app,
            database,
            user_a_cookie,
            user_b_cookie,
        }
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
        "/api/v1/entries?categoryId=00000000-0000-4000-8000-000000000501",
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
        (
            Method::PUT,
            "/api/v1/entries/00000000-0000-4000-8000-000000000301",
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
