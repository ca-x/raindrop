#[allow(dead_code)]
mod support;

use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use async_trait::async_trait;
use axum::{
    Router,
    body::Body,
    http::{
        Method, Request, StatusCode,
        header::{
            CACHE_CONTROL, CONTENT_TYPE, COOKIE, HOST, LOCATION, ORIGIN, PRAGMA, RETRY_AFTER,
        },
    },
};
use http_body_util::BodyExt;
use raindrop::{
    app::{AppState, build_router},
    auth::build_session_cookie,
    db::entities::{feed, feed_refresh_run},
    db::{DatabaseConfig, connect, migrate},
    feeds::{
        FeedExecutor, FeedFetchError, FeedRepository, FeedRuntime, FeedTransport, FeedUrlPolicy,
        FetchOutcome, FetchRequest,
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

use support::database::{FEED_ID, USER_A_ID, USER_B_ID, insert_feed, insert_user};

#[derive(Clone)]
struct BlockedTransport {
    calls: Arc<AtomicUsize>,
}

#[async_trait]
impl FeedTransport for BlockedTransport {
    async fn fetch(&self, _request: FetchRequest) -> Result<FetchOutcome, FeedFetchError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        std::future::pending().await
    }
}

struct SubscriptionApiFixture {
    _data: TempDir,
    app: Router,
    database: DatabaseConnection,
    user_a_cookie: String,
    user_a_csrf: String,
    user_b_cookie: String,
    user_b_csrf: String,
    transport_calls: Arc<AtomicUsize>,
}

#[derive(Clone, Copy)]
enum UserKind {
    A,
    B,
}

impl SubscriptionApiFixture {
    async fn new() -> Self {
        Self::with_blocked_transport().await
    }

    async fn with_blocked_transport() -> Self {
        let data = tempfile::tempdir().expect("temporary directory should be created");
        let database_url = format!(
            "sqlite://{}?mode=rwc",
            data.path().join("subscription-api.db").display()
        );
        let database = connect(&DatabaseConfig::new(SecretString::from(database_url)))
            .await
            .expect("subscription API database should connect");
        migrate(&database)
            .await
            .expect("subscription API database should migrate");
        insert_user(&database, USER_A_ID, "subscription-a").await;
        insert_user(&database, USER_B_ID, "subscription-b").await;

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
        let transport_calls = Arc::new(AtomicUsize::new(0));
        let transport = BlockedTransport {
            calls: transport_calls.clone(),
        };
        let (_runtime, handle) = FeedRuntime::new(setup.clone(), move |database| {
            Ok(Arc::new(FeedExecutor::new(
                FeedRepository::new(database),
                FeedUrlPolicy::new(false),
                transport.clone(),
            )))
        });
        let app = build_router(AppState::with_feed_runtime(setup, handle));

        Self {
            _data: data,
            app,
            database,
            user_a_cookie: session_cookie(&user_a_session),
            user_a_csrf: user_a_session.csrf_token.expose_secret().to_owned(),
            user_b_cookie: session_cookie(&user_b_session),
            user_b_csrf: user_b_session.csrf_token.expose_secret().to_owned(),
            transport_calls,
        }
    }

    async fn get(&self, uri: &str, user: UserKind) -> axum::response::Response {
        let cookie = self.credentials(user).0;
        let request = Request::builder()
            .method(Method::GET)
            .uri(uri)
            .header(COOKIE, cookie)
            .body(Body::empty())
            .expect("subscription GET request should build");
        self.app
            .clone()
            .oneshot(request)
            .await
            .expect("subscription GET request should complete")
    }

    async fn request_authenticated(
        &self,
        method: Method,
        uri: &str,
        user: UserKind,
    ) -> axum::response::Response {
        let cookie = self.credentials(user).0;
        let request = Request::builder()
            .method(method)
            .uri(uri)
            .header(COOKIE, cookie)
            .body(Body::empty())
            .expect("authenticated subscription request should build");
        self.app
            .clone()
            .oneshot(request)
            .await
            .expect("authenticated subscription request should complete")
    }

    async fn post_with_csrf(
        &self,
        uri: &str,
        body: Value,
        user: UserKind,
    ) -> axum::response::Response {
        let (cookie, csrf) = self.credentials(user);
        let request = Request::builder()
            .method(Method::POST)
            .uri(uri)
            .header(COOKIE, cookie)
            .header("x-csrf-token", csrf)
            .header(CONTENT_TYPE, "application/json")
            .header(ORIGIN, "http://subscriptions.test")
            .header(HOST, "subscriptions.test")
            .body(Body::from(body.to_string()))
            .expect("subscription POST request should build");
        self.app
            .clone()
            .oneshot(request)
            .await
            .expect("subscription POST request should complete")
    }

    async fn request_unauthenticated(
        &self,
        method: Method,
        uri: &str,
        body: Option<Value>,
    ) -> axum::response::Response {
        let mut request = Request::builder().method(method).uri(uri);
        if body.is_some() {
            request = request.header(CONTENT_TYPE, "application/json");
        }
        let request = request
            .body(body.map_or_else(Body::empty, |value| Body::from(value.to_string())))
            .expect("unauthenticated subscription request should build");
        self.app
            .clone()
            .oneshot(request)
            .await
            .expect("unauthenticated subscription request should complete")
    }

    #[allow(clippy::too_many_arguments)]
    async fn request_mutation(
        &self,
        method: Method,
        uri: &str,
        body: Option<&str>,
        user: UserKind,
        csrf_headers: &[&str],
        origin_headers: &[&str],
        host: Option<&str>,
    ) -> axum::response::Response {
        let cookie = self.credentials(user).0;
        let mut request = Request::builder()
            .method(method)
            .uri(uri)
            .header(COOKIE, cookie);
        for csrf in csrf_headers {
            request = request.header("x-csrf-token", *csrf);
        }
        for origin in origin_headers {
            request = request.header(ORIGIN, *origin);
        }
        if let Some(host) = host {
            request = request.header(HOST, host);
        }
        if body.is_some() {
            request = request.header(CONTENT_TYPE, "application/json");
        }
        let request = request
            .body(body.map_or_else(Body::empty, |value| Body::from(value.to_owned())))
            .expect("subscription mutation request should build");
        self.app
            .clone()
            .oneshot(request)
            .await
            .expect("subscription mutation request should complete")
    }

    fn credentials(&self, user: UserKind) -> (&str, &str) {
        match user {
            UserKind::A => (&self.user_a_cookie, &self.user_a_csrf),
            UserKind::B => (&self.user_b_cookie, &self.user_b_csrf),
        }
    }

    fn transport_calls(&self) -> usize {
        self.transport_calls.load(Ordering::SeqCst)
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

async fn response_body_bytes(response: axum::response::Response) -> axum::body::Bytes {
    response
        .into_body()
        .collect()
        .await
        .expect("response body should collect")
        .to_bytes()
}

fn assert_sensitive_cache_headers(response: &axum::response::Response) {
    assert_eq!(response.headers().get(CACHE_CONTROL).unwrap(), "no-store");
    assert_eq!(response.headers().get(PRAGMA).unwrap(), "no-cache");
}

#[tokio::test]
async fn subscription_list_is_empty_for_a_new_user() {
    let fixture = SubscriptionApiFixture::new().await;
    let response = fixture.get("/api/v1/subscriptions", UserKind::A).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_sensitive_cache_headers(&response);
    assert_eq!(response_json(response).await["items"], json!([]));
}

#[tokio::test]
async fn subscription_create_returns_before_blocked_transport_and_sets_location() {
    let fixture = SubscriptionApiFixture::with_blocked_transport().await;
    let response = fixture
        .post_with_csrf(
            "/api/v1/subscriptions",
            json!({ "url": "https://feed.example/rss.xml" }),
            UserKind::A,
        )
        .await;
    assert_eq!(response.status(), StatusCode::CREATED);
    assert!(response.headers().contains_key(LOCATION));
    let body = response_json(response).await;
    assert_eq!(body["created"], true);
    assert_eq!(body["subscription"]["refresh"]["state"], "PENDING");
    assert_eq!(fixture.transport_calls(), 0);
}

#[tokio::test]
async fn subscription_routes_require_active_session() {
    let fixture = SubscriptionApiFixture::new().await;
    for (method, uri, body) in [
        (Method::GET, "/api/v1/subscriptions", None),
        (
            Method::GET,
            "/api/v1/subscriptions/00000000-0000-4000-8000-000000000299",
            None,
        ),
        (
            Method::POST,
            "/api/v1/subscriptions",
            Some(json!({ "url": "https://feed.example/rss.xml" })),
        ),
        (
            Method::POST,
            "/api/v1/subscriptions/00000000-0000-4000-8000-000000000299/refresh",
            Some(json!({ "requestId": "00000000-0000-4000-8000-000000000701" })),
        ),
        (
            Method::DELETE,
            "/api/v1/subscriptions/00000000-0000-4000-8000-000000000299",
            None,
        ),
    ] {
        let response = fixture.request_unauthenticated(method, uri, body).await;
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        assert_sensitive_cache_headers(&response);
        assert_eq!(
            response_json(response).await["error"]["code"],
            "AUTHENTICATION_REQUIRED"
        );
    }
}

#[tokio::test]
async fn subscription_mutations_require_valid_csrf() {
    let fixture = SubscriptionApiFixture::new().await;
    let (_, valid_csrf) = fixture.credentials(UserKind::A);
    for (method, uri, body) in [
        (
            Method::POST,
            "/api/v1/subscriptions",
            Some(r#"{"url":"https://feed.example/rss.xml"}"#),
        ),
        (
            Method::POST,
            "/api/v1/subscriptions/not-a-uuid/refresh",
            Some(r#"{"requestId":"not-a-uuid"}"#),
        ),
        (Method::DELETE, "/api/v1/subscriptions/not-a-uuid", None),
    ] {
        for csrf_headers in [Vec::<&str>::new(), vec!["invalid-csrf"]] {
            let response = fixture
                .request_mutation(
                    method.clone(),
                    uri,
                    body,
                    UserKind::A,
                    &csrf_headers,
                    &["http://subscriptions.test"],
                    Some("subscriptions.test"),
                )
                .await;
            assert_eq!(response.status(), StatusCode::FORBIDDEN);
            assert_sensitive_cache_headers(&response);
            assert_eq!(response_json(response).await["error"]["code"], "FORBIDDEN");
        }
    }

    let response = fixture
        .request_mutation(
            Method::POST,
            "/api/v1/subscriptions",
            Some(r#"{"url":"https://feed.example/rss.xml"}"#),
            UserKind::A,
            &[valid_csrf, valid_csrf],
            &["http://subscriptions.test"],
            Some("subscriptions.test"),
        )
        .await;
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn subscription_mutations_enforce_same_origin() {
    let fixture = SubscriptionApiFixture::new().await;
    let (_, csrf) = fixture.credentials(UserKind::A);
    for (origins, host) in [
        (vec!["https://evil.example"], Some("subscriptions.test")),
        (vec!["http://subscriptions.test"], None),
        (
            vec!["http://subscriptions.test", "http://subscriptions.test"],
            Some("subscriptions.test"),
        ),
    ] {
        let response = fixture
            .request_mutation(
                Method::POST,
                "/api/v1/subscriptions",
                Some(r#"{"url":"not-a-url"}"#),
                UserKind::A,
                &[csrf],
                &origins,
                host,
            )
            .await;
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        assert_sensitive_cache_headers(&response);
        assert_eq!(response_json(response).await["error"]["code"], "FORBIDDEN");
    }

    let response = fixture
        .request_mutation(
            Method::POST,
            "/api/v1/subscriptions",
            Some(r#"{"url":"not-a-url"}"#),
            UserKind::A,
            &[csrf],
            &["http://subscriptions.test"],
            Some("subscriptions.test"),
        )
        .await;
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn subscription_requests_reject_invalid_query_path_body_and_url() {
    let fixture = SubscriptionApiFixture::new().await;
    for uri in [
        "/api/v1/subscriptions?limit=0",
        "/api/v1/subscriptions?limit=101",
        "/api/v1/subscriptions?unknown=true",
        "/api/v1/subscriptions?cursor=not-a-cursor",
    ] {
        let response = fixture.get(uri, UserKind::A).await;
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY, "{uri}");
        assert_eq!(
            response_json(response).await["error"]["code"],
            "VALIDATION_ERROR"
        );
    }

    let response = fixture
        .get("/api/v1/subscriptions/not-a-uuid", UserKind::A)
        .await;
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);

    let (_, csrf) = fixture.credentials(UserKind::A);
    for body in [
        "{}",
        r#"{"url":null}"#,
        r#"{"url":1}"#,
        r#"{"url":"https://feed.example/rss.xml","extra":true}"#,
        r#"{"url":"https://feed.example/rss.xml""#,
        r#"{"URL":"https://feed.example/rss.xml"}"#,
    ] {
        let response = fixture
            .request_mutation(
                Method::POST,
                "/api/v1/subscriptions",
                Some(body),
                UserKind::A,
                &[csrf],
                &["http://subscriptions.test"],
                Some("subscriptions.test"),
            )
            .await;
        assert_eq!(
            response.status(),
            StatusCode::UNPROCESSABLE_ENTITY,
            "{body}"
        );
    }

    for url in [
        "",
        "not-a-url",
        "http://feed.example/rss.xml",
        "https://user:password@feed.example/rss.xml",
    ] {
        let response = fixture
            .post_with_csrf("/api/v1/subscriptions", json!({ "url": url }), UserKind::A)
            .await;
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY, "{url}");
        assert_eq!(
            response_json(response).await["error"]["code"],
            "VALIDATION_ERROR"
        );
    }

    for (uri, body) in [
        (
            "/api/v1/subscriptions/not-a-uuid/refresh",
            r#"{"requestId":"00000000-0000-4000-8000-000000000701"}"#,
        ),
        (
            "/api/v1/subscriptions/00000000-0000-4000-8000-000000000299/refresh",
            r#"{"requestId":"not-a-uuid"}"#,
        ),
        (
            "/api/v1/subscriptions/00000000-0000-4000-8000-000000000299/refresh",
            r#"{"request_id":"00000000-0000-4000-8000-000000000701"}"#,
        ),
        (
            "/api/v1/subscriptions/00000000-0000-4000-8000-000000000299/refresh",
            r#"{"requestId":"00000000-0000-4000-8000-000000000701","extra":true}"#,
        ),
    ] {
        let response = fixture
            .request_mutation(
                Method::POST,
                uri,
                Some(body),
                UserKind::A,
                &[csrf],
                &["http://subscriptions.test"],
                Some("subscriptions.test"),
            )
            .await;
        assert_eq!(
            response.status(),
            StatusCode::UNPROCESSABLE_ENTITY,
            "{body}"
        );
    }

    let response = fixture
        .request_mutation(
            Method::DELETE,
            "/api/v1/subscriptions/not-a-uuid",
            None,
            UserKind::A,
            &[csrf],
            &["http://subscriptions.test"],
            Some("subscriptions.test"),
        )
        .await;
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn subscription_detail_hides_missing_and_cross_tenant() {
    let fixture = SubscriptionApiFixture::new().await;
    let created = fixture
        .post_with_csrf(
            "/api/v1/subscriptions",
            json!({ "url": "https://feed.example/rss.xml" }),
            UserKind::B,
        )
        .await;
    assert_eq!(created.status(), StatusCode::CREATED);
    let created_body = response_json(created).await;
    let subscription_id = created_body["subscription"]["subscriptionId"]
        .as_str()
        .expect("created subscription should expose its identifier");

    let mut hidden_envelopes = Vec::new();
    for subscription_id in ["00000000-0000-4000-8000-000000000299", subscription_id] {
        let response = fixture
            .get(
                &format!("/api/v1/subscriptions/{subscription_id}"),
                UserKind::A,
            )
            .await;
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        assert_sensitive_cache_headers(&response);
        let body = response_json(response).await;
        hidden_envelopes.push((
            body["error"]["code"].clone(),
            body["error"]["message"].clone(),
        ));
    }
    assert_eq!(hidden_envelopes[0], hidden_envelopes[1]);

    let response = fixture
        .get(
            &format!("/api/v1/subscriptions/{subscription_id}"),
            UserKind::B,
        )
        .await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = response_json(response).await;
    assert_eq!(body["subscriptionId"], subscription_id);
    assert_eq!(
        body.as_object()
            .expect("subscription should be an object")
            .keys()
            .map(String::as_str)
            .collect::<std::collections::BTreeSet<_>>(),
        std::collections::BTreeSet::from([
            "feedId",
            "refresh",
            "siteUrl",
            "subscriptionId",
            "title",
            "unreadCount",
        ])
    );
    assert_eq!(
        body["refresh"]
            .as_object()
            .expect("refresh should be an object")
            .keys()
            .map(String::as_str)
            .collect::<std::collections::BTreeSet<_>>(),
        std::collections::BTreeSet::from([
            "completedAt",
            "droppedCount",
            "errorCode",
            "generation",
            "newCount",
            "operationId",
            "queuedAt",
            "retryAt",
            "startedAt",
            "state",
            "updatedCount",
        ])
    );
}

#[tokio::test]
async fn subscription_delete_is_idempotent_and_non_enumerating() {
    let fixture = SubscriptionApiFixture::new().await;
    let created = fixture
        .post_with_csrf(
            "/api/v1/subscriptions",
            json!({ "url": "https://feed.example/rss.xml" }),
            UserKind::B,
        )
        .await;
    let created_body = response_json(created).await;
    let subscription_id = created_body["subscription"]["subscriptionId"]
        .as_str()
        .expect("created subscription should expose its identifier")
        .to_owned();

    let (_, user_a_csrf) = fixture.credentials(UserKind::A);
    for target in [
        "00000000-0000-4000-8000-000000000299",
        subscription_id.as_str(),
    ] {
        let response = fixture
            .request_mutation(
                Method::DELETE,
                &format!("/api/v1/subscriptions/{target}"),
                None,
                UserKind::A,
                &[user_a_csrf],
                &["http://subscriptions.test"],
                Some("subscriptions.test"),
            )
            .await;
        assert_eq!(response.status(), StatusCode::NO_CONTENT);
        assert_sensitive_cache_headers(&response);
        assert!(response_body_bytes(response).await.is_empty());
    }
    let response = fixture
        .get(
            &format!("/api/v1/subscriptions/{subscription_id}"),
            UserKind::B,
        )
        .await;
    assert_eq!(response.status(), StatusCode::OK);

    let (_, user_b_csrf) = fixture.credentials(UserKind::B);
    for _ in 0..2 {
        let response = fixture
            .request_mutation(
                Method::DELETE,
                &format!("/api/v1/subscriptions/{subscription_id}"),
                None,
                UserKind::B,
                &[user_b_csrf],
                &["http://subscriptions.test"],
                Some("subscriptions.test"),
            )
            .await;
        assert_eq!(response.status(), StatusCode::NO_CONTENT);
        assert_sensitive_cache_headers(&response);
        assert!(response_body_bytes(response).await.is_empty());
    }
    let response = fixture
        .get(
            &format!("/api/v1/subscriptions/{subscription_id}"),
            UserKind::B,
        )
        .await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn subscription_refresh_is_exactly_idempotent_and_reports_active_conflict() {
    const REQUEST_A: &str = "00000000-0000-4000-8000-000000000701";
    const REQUEST_B: &str = "00000000-0000-4000-8000-000000000702";

    let fixture = SubscriptionApiFixture::new().await;
    let created = fixture
        .post_with_csrf(
            "/api/v1/subscriptions",
            json!({ "url": "https://feed.example/rss.xml" }),
            UserKind::A,
        )
        .await;
    let created_body = response_json(created).await;
    let subscription_id = created_body["subscription"]["subscriptionId"]
        .as_str()
        .expect("created subscription should expose its identifier")
        .to_owned();
    let feed_id = created_body["subscription"]["feedId"]
        .as_str()
        .expect("created subscription should expose its feed identifier")
        .to_owned();
    let subscribe_operation = created_body["subscription"]["refresh"]["operationId"]
        .as_str()
        .expect("new subscription should expose its queued refresh")
        .to_owned();

    let subscribe_run = feed_refresh_run::Entity::find_by_id(&subscribe_operation)
        .one(&fixture.database)
        .await
        .expect("subscribe run should query")
        .expect("subscribe run should exist");
    let mut subscribe_run = subscribe_run.into_active_model();
    subscribe_run.status = Set("SUCCESS".to_owned());
    subscribe_run.completed_at = Set(Some(datetime!(2026-07-17 11:59:59 UTC)));
    subscribe_run
        .update(&fixture.database)
        .await
        .expect("subscribe run should become terminal");

    let response = fixture
        .post_with_csrf(
            &format!("/api/v1/subscriptions/{subscription_id}/refresh"),
            json!({ "requestId": REQUEST_A }),
            UserKind::A,
        )
        .await;
    assert_eq!(response.status(), StatusCode::ACCEPTED);
    assert_sensitive_cache_headers(&response);
    let accepted = response_json(response).await;
    let operation_id = accepted["operationId"]
        .as_str()
        .expect("accepted refresh should expose its operation")
        .to_owned();
    assert_eq!(accepted["state"], "PENDING");

    let replay = fixture
        .post_with_csrf(
            &format!("/api/v1/subscriptions/{subscription_id}/refresh"),
            json!({ "requestId": REQUEST_A }),
            UserKind::A,
        )
        .await;
    assert_eq!(replay.status(), StatusCode::ACCEPTED);
    assert_eq!(response_json(replay).await["operationId"], operation_id);

    let conflict = fixture
        .post_with_csrf(
            &format!("/api/v1/subscriptions/{subscription_id}/refresh"),
            json!({ "requestId": REQUEST_B }),
            UserKind::A,
        )
        .await;
    assert_eq!(conflict.status(), StatusCode::CONFLICT);
    let conflict_body = response_json(conflict).await;
    assert_eq!(conflict_body["error"]["code"], "REFRESH_IN_PROGRESS");
    assert_eq!(
        conflict_body["error"]["fields"]["operationId"],
        operation_id
    );
    assert_eq!(
        conflict_body["error"]["fields"]
            .as_object()
            .expect("conflict fields should be an object")
            .len(),
        1
    );

    let manual_run = feed_refresh_run::Entity::find_by_id(&operation_id)
        .one(&fixture.database)
        .await
        .expect("manual run should query")
        .expect("manual run should exist");
    let mut manual_run = manual_run.into_active_model();
    manual_run.status = Set("ERROR".to_owned());
    manual_run.commit_generation = Set(Some(7));
    manual_run.new_count = Set(3);
    manual_run.updated_count = Set(2);
    manual_run.dropped_count = Set(1);
    manual_run.error_code = Set(Some("INTERNAL_PROVIDER_DETAIL".to_owned()));
    manual_run.queued_at = Set(datetime!(2026-07-17 12:00:00.123456 UTC));
    manual_run.started_at = Set(Some(datetime!(2026-07-17 12:00:01.234567 UTC)));
    manual_run.retry_at = Set(Some(datetime!(2036-07-17 12:00:30.345678 UTC)));
    manual_run.completed_at = Set(Some(datetime!(2026-07-17 12:00:02.345678 UTC)));
    manual_run
        .update(&fixture.database)
        .await
        .expect("manual run should become terminal");
    let stored_feed = feed::Entity::find_by_id(&feed_id)
        .one(&fixture.database)
        .await
        .expect("feed should query")
        .expect("feed should exist");
    let mut stored_feed = stored_feed.into_active_model();
    stored_feed.last_attempt_at = Set(Some(datetime!(2036-07-17 12:00:00 UTC)));
    stored_feed
        .update(&fixture.database)
        .await
        .expect("feed should enter cooldown");

    let terminal_replay = fixture
        .post_with_csrf(
            &format!("/api/v1/subscriptions/{subscription_id}/refresh"),
            json!({ "requestId": REQUEST_A }),
            UserKind::A,
        )
        .await;
    assert_eq!(terminal_replay.status(), StatusCode::OK);
    let terminal = response_json(terminal_replay).await;
    assert_eq!(terminal["operationId"], operation_id);
    assert_eq!(terminal["state"], "BACKING_OFF");
    assert_eq!(terminal["newCount"], 3);
    assert_eq!(terminal["updatedCount"], 2);
    assert_eq!(terminal["droppedCount"], 1);
    assert_eq!(terminal["generation"], 7);
    assert_eq!(terminal["errorCode"], "REFRESH_FAILED");
    assert_eq!(terminal["queuedAt"], "2026-07-17T12:00:00.123456Z");
    assert_eq!(terminal["startedAt"], "2026-07-17T12:00:01.234567Z");
    assert_eq!(terminal["completedAt"], "2026-07-17T12:00:02.345678Z");
    assert_eq!(terminal["retryAt"], "2036-07-17T12:00:30.345678Z");

    let manual_run = feed_refresh_run::Entity::find_by_id(&operation_id)
        .one(&fixture.database)
        .await
        .expect("manual run should query")
        .expect("manual run should exist");
    let mut manual_run = manual_run.into_active_model();
    manual_run.trigger_kind = Set("SUBSCRIBE".to_owned());
    manual_run
        .update(&fixture.database)
        .await
        .expect("manual run semantics should be corrupted for the conflict fixture");
    let semantic_conflict = fixture
        .post_with_csrf(
            &format!("/api/v1/subscriptions/{subscription_id}/refresh"),
            json!({ "requestId": REQUEST_A }),
            UserKind::A,
        )
        .await;
    assert_eq!(semantic_conflict.status(), StatusCode::CONFLICT);
    assert_eq!(
        response_json(semantic_conflict).await["error"]["code"],
        "CONFLICT"
    );
}

#[tokio::test]
async fn subscription_rate_limits_are_user_scoped_and_return_retry_after() {
    let fixture = SubscriptionApiFixture::new().await;
    for attempt in 0..30 {
        let response = fixture
            .post_with_csrf(
                "/api/v1/subscriptions",
                json!({ "url": "https://feed.example/rss.xml" }),
                UserKind::A,
            )
            .await;
        assert!(
            matches!(response.status(), StatusCode::OK | StatusCode::CREATED),
            "attempt {attempt} should be admitted"
        );
    }

    let limited = fixture
        .post_with_csrf(
            "/api/v1/subscriptions",
            json!({ "url": "https://feed.example/rss.xml" }),
            UserKind::A,
        )
        .await;
    assert_eq!(limited.status(), StatusCode::TOO_MANY_REQUESTS);
    assert_sensitive_cache_headers(&limited);
    let retry_after = limited
        .headers()
        .get(RETRY_AFTER)
        .expect("rate limit should include Retry-After")
        .to_str()
        .expect("Retry-After should be ASCII")
        .parse::<u64>()
        .expect("Retry-After should be integer seconds");
    assert!(retry_after >= 1);
    let limited_body = response_json(limited).await;
    assert_eq!(limited_body["error"]["code"], "RATE_LIMITED");
    assert_eq!(limited_body["error"]["message"], "Too many requests");
    assert!(
        limited_body["error"]["fields"]["retryAt"]
            .as_str()
            .is_some_and(|value| value.ends_with('Z'))
    );

    let user_b = fixture
        .post_with_csrf(
            "/api/v1/subscriptions",
            json!({ "url": "https://feed.example/rss.xml" }),
            UserKind::B,
        )
        .await;
    assert_eq!(user_b.status(), StatusCode::CREATED);

    let invalid_url = fixture
        .post_with_csrf(
            "/api/v1/subscriptions",
            json!({ "url": "not-a-url" }),
            UserKind::A,
        )
        .await;
    assert_eq!(invalid_url.status(), StatusCode::UNPROCESSABLE_ENTITY);

    let (_, csrf) = fixture.credentials(UserKind::A);
    let invalid_csrf = fixture
        .request_mutation(
            Method::POST,
            "/api/v1/subscriptions",
            Some(r#"{"url":"https://feed.example/other.xml"}"#),
            UserKind::A,
            &["invalid-csrf"],
            &["http://subscriptions.test"],
            Some("subscriptions.test"),
        )
        .await;
    assert_eq!(invalid_csrf.status(), StatusCode::FORBIDDEN);

    let unauthenticated = fixture
        .request_unauthenticated(
            Method::POST,
            "/api/v1/subscriptions",
            Some(json!({ "url": "https://feed.example/other.xml" })),
        )
        .await;
    assert_eq!(unauthenticated.status(), StatusCode::UNAUTHORIZED);
    assert!(!csrf.is_empty());
}

#[tokio::test]
async fn subscription_responses_disable_caching_for_200_201_202_204_401_403_404_405_409_422_429_500()
 {
    let fixture = SubscriptionApiFixture::new().await;
    let response = fixture.get("/api/v1/subscriptions", UserKind::A).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_sensitive_cache_headers(&response);

    let fixture = SubscriptionApiFixture::new().await;
    let created = fixture
        .post_with_csrf(
            "/api/v1/subscriptions",
            json!({ "url": "https://feed.example/rss.xml" }),
            UserKind::A,
        )
        .await;
    assert_eq!(created.status(), StatusCode::CREATED);
    assert_sensitive_cache_headers(&created);
    let created_body = response_json(created).await;
    let subscription_id = created_body["subscription"]["subscriptionId"]
        .as_str()
        .expect("created subscription should expose its identifier")
        .to_owned();
    let operation_id = created_body["subscription"]["refresh"]["operationId"]
        .as_str()
        .expect("created subscription should expose its refresh")
        .to_owned();
    let subscribe_run = feed_refresh_run::Entity::find_by_id(operation_id)
        .one(&fixture.database)
        .await
        .expect("subscribe run should query")
        .expect("subscribe run should exist");
    let mut subscribe_run = subscribe_run.into_active_model();
    subscribe_run.status = Set("SUCCESS".to_owned());
    subscribe_run.completed_at = Set(Some(datetime!(2026-07-17 12:00:00 UTC)));
    subscribe_run
        .update(&fixture.database)
        .await
        .expect("subscribe run should become terminal");
    let accepted = fixture
        .post_with_csrf(
            &format!("/api/v1/subscriptions/{subscription_id}/refresh"),
            json!({ "requestId": "00000000-0000-4000-8000-000000000711" }),
            UserKind::A,
        )
        .await;
    assert_eq!(accepted.status(), StatusCode::ACCEPTED);
    assert_sensitive_cache_headers(&accepted);

    let fixture = SubscriptionApiFixture::new().await;
    let (_, csrf) = fixture.credentials(UserKind::A);
    let deleted = fixture
        .request_mutation(
            Method::DELETE,
            "/api/v1/subscriptions/00000000-0000-4000-8000-000000000299",
            None,
            UserKind::A,
            &[csrf],
            &["http://subscriptions.test"],
            Some("subscriptions.test"),
        )
        .await;
    assert_eq!(deleted.status(), StatusCode::NO_CONTENT);
    assert_sensitive_cache_headers(&deleted);

    let fixture = SubscriptionApiFixture::new().await;
    let unauthorized = fixture
        .request_unauthenticated(Method::GET, "/api/v1/subscriptions", None)
        .await;
    assert_eq!(unauthorized.status(), StatusCode::UNAUTHORIZED);
    assert_sensitive_cache_headers(&unauthorized);

    let forbidden = fixture
        .request_mutation(
            Method::POST,
            "/api/v1/subscriptions",
            Some(r#"{"url":"https://feed.example/rss.xml"}"#),
            UserKind::A,
            &[],
            &["http://subscriptions.test"],
            Some("subscriptions.test"),
        )
        .await;
    assert_eq!(forbidden.status(), StatusCode::FORBIDDEN);
    assert_sensitive_cache_headers(&forbidden);

    let not_found = fixture
        .get(
            "/api/v1/subscriptions/00000000-0000-4000-8000-000000000299",
            UserKind::A,
        )
        .await;
    assert_eq!(not_found.status(), StatusCode::NOT_FOUND);
    assert_sensitive_cache_headers(&not_found);

    let method_not_allowed = fixture
        .request_authenticated(Method::PUT, "/api/v1/subscriptions", UserKind::A)
        .await;
    assert_eq!(method_not_allowed.status(), StatusCode::METHOD_NOT_ALLOWED);
    assert_sensitive_cache_headers(&method_not_allowed);

    let created = fixture
        .post_with_csrf(
            "/api/v1/subscriptions",
            json!({ "url": "https://conflict.example/rss.xml" }),
            UserKind::A,
        )
        .await;
    let created_body = response_json(created).await;
    let subscription_id = created_body["subscription"]["subscriptionId"]
        .as_str()
        .expect("created subscription should expose its identifier");
    let conflict = fixture
        .post_with_csrf(
            &format!("/api/v1/subscriptions/{subscription_id}/refresh"),
            json!({ "requestId": "00000000-0000-4000-8000-000000000712" }),
            UserKind::A,
        )
        .await;
    assert_eq!(conflict.status(), StatusCode::CONFLICT);
    assert_sensitive_cache_headers(&conflict);

    let validation = fixture
        .get("/api/v1/subscriptions?limit=0", UserKind::A)
        .await;
    assert_eq!(validation.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert_sensitive_cache_headers(&validation);

    let fixture = SubscriptionApiFixture::new().await;
    for _ in 0..30 {
        let response = fixture
            .post_with_csrf(
                "/api/v1/subscriptions",
                json!({ "url": "https://limited.example/rss.xml" }),
                UserKind::A,
            )
            .await;
        assert!(matches!(
            response.status(),
            StatusCode::OK | StatusCode::CREATED
        ));
    }
    let rate_limited = fixture
        .post_with_csrf(
            "/api/v1/subscriptions",
            json!({ "url": "https://limited.example/rss.xml" }),
            UserKind::A,
        )
        .await;
    assert_eq!(rate_limited.status(), StatusCode::TOO_MANY_REQUESTS);
    assert_sensitive_cache_headers(&rate_limited);

    let fixture = SubscriptionApiFixture::new().await;
    fixture
        .database
        .clone()
        .close()
        .await
        .expect("subscription database should close");
    let internal = fixture.get("/api/v1/subscriptions", UserKind::A).await;
    assert_eq!(internal.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert_sensitive_cache_headers(&internal);
}

#[tokio::test]
async fn subscription_unknown_and_trailing_paths_never_return_embedded_html() {
    let fixture = SubscriptionApiFixture::new().await;
    for uri in [
        "/api/v1/subscriptions/",
        "/api/v1/subscriptions/unknown/extra",
        "/api/v1/subscriptions/00000000-0000-4000-8000-000000000299/refresh/extra",
    ] {
        let response = fixture
            .request_unauthenticated(Method::GET, uri, None)
            .await;
        assert_eq!(response.status(), StatusCode::NOT_FOUND, "{uri}");
        assert_sensitive_cache_headers(&response);
        assert_eq!(
            response.headers().get(CONTENT_TYPE).unwrap(),
            "application/json"
        );
        let bytes = response_body_bytes(response).await;
        assert!(!String::from_utf8_lossy(&bytes).contains("<html"));
        assert_eq!(
            serde_json::from_slice::<Value>(&bytes).expect("fallback should return JSON")["error"]
                ["code"],
            "NOT_FOUND"
        );
    }

    let response = fixture
        .request_authenticated(Method::PUT, "/api/v1/subscriptions", UserKind::A)
        .await;
    assert_eq!(response.status(), StatusCode::METHOD_NOT_ALLOWED);
    assert_sensitive_cache_headers(&response);
    assert_eq!(
        response.headers().get(CONTENT_TYPE).unwrap(),
        "application/json"
    );
    let bytes = response_body_bytes(response).await;
    assert!(!String::from_utf8_lossy(&bytes).contains("<html"));
    assert_eq!(
        serde_json::from_slice::<Value>(&bytes).expect("method fallback should return JSON")["error"]
            ["code"],
        "METHOD_NOT_ALLOWED"
    );
}

#[tokio::test]
async fn subscription_conflicts_use_stable_public_envelopes() {
    let fixture = SubscriptionApiFixture::new().await;
    insert_feed(&fixture.database, time::OffsetDateTime::now_utc()).await;
    let stored_feed = feed::Entity::find_by_id(FEED_ID)
        .one(&fixture.database)
        .await
        .expect("hash fixture feed should query")
        .expect("hash fixture feed should exist");
    let mut stored_feed = stored_feed.into_active_model();
    stored_feed.normalized_url_hash = Set(blake3::hash(b"https://hash.example/rss.xml")
        .to_hex()
        .to_string());
    stored_feed
        .update(&fixture.database)
        .await
        .expect("hash fixture should update");
    let hash_conflict = fixture
        .post_with_csrf(
            "/api/v1/subscriptions",
            json!({ "url": "https://hash.example/rss.xml" }),
            UserKind::A,
        )
        .await;
    assert_eq!(hash_conflict.status(), StatusCode::CONFLICT);
    let expected = response_json(hash_conflict).await;
    assert_eq!(expected["error"]["code"], "CONFLICT");
    assert!(expected["error"].get("fields").is_none());

    let fixture = SubscriptionApiFixture::new().await;
    let created = fixture
        .post_with_csrf(
            "/api/v1/subscriptions",
            json!({ "url": "https://disabled.example/rss.xml" }),
            UserKind::A,
        )
        .await;
    let created_body = response_json(created).await;
    let subscription_id = created_body["subscription"]["subscriptionId"]
        .as_str()
        .expect("created subscription should expose its identifier")
        .to_owned();
    let feed_id = created_body["subscription"]["feedId"]
        .as_str()
        .expect("created subscription should expose its feed identifier")
        .to_owned();
    let operation_id = created_body["subscription"]["refresh"]["operationId"]
        .as_str()
        .expect("created subscription should expose its refresh")
        .to_owned();
    let run = feed_refresh_run::Entity::find_by_id(operation_id)
        .one(&fixture.database)
        .await
        .expect("subscribe run should query")
        .expect("subscribe run should exist");
    let mut run = run.into_active_model();
    run.status = Set("SUCCESS".to_owned());
    run.completed_at = Set(Some(datetime!(2026-07-17 12:00:00 UTC)));
    run.update(&fixture.database)
        .await
        .expect("subscribe run should become terminal");
    let stored_feed = feed::Entity::find_by_id(feed_id)
        .one(&fixture.database)
        .await
        .expect("disabled fixture feed should query")
        .expect("disabled fixture feed should exist");
    let mut stored_feed = stored_feed.into_active_model();
    stored_feed.is_disabled = Set(true);
    stored_feed
        .update(&fixture.database)
        .await
        .expect("feed should disable");
    let disabled = fixture
        .post_with_csrf(
            &format!("/api/v1/subscriptions/{subscription_id}/refresh"),
            json!({ "requestId": "00000000-0000-4000-8000-000000000721" }),
            UserKind::A,
        )
        .await;
    assert_eq!(disabled.status(), StatusCode::CONFLICT);
    let disabled_body = response_json(disabled).await;
    assert_eq!(
        (
            &disabled_body["error"]["code"],
            &disabled_body["error"]["message"]
        ),
        (&expected["error"]["code"], &expected["error"]["message"])
    );

    let fixture = SubscriptionApiFixture::new().await;
    let created = fixture
        .post_with_csrf(
            "/api/v1/subscriptions",
            json!({ "url": "https://corrupt.example/rss.xml" }),
            UserKind::A,
        )
        .await;
    let created_body = response_json(created).await;
    let subscription_id = created_body["subscription"]["subscriptionId"]
        .as_str()
        .expect("created subscription should expose its identifier")
        .to_owned();
    let operation_id = created_body["subscription"]["refresh"]["operationId"]
        .as_str()
        .expect("created subscription should expose its refresh");
    let run = feed_refresh_run::Entity::find_by_id(operation_id)
        .one(&fixture.database)
        .await
        .expect("subscribe run should query")
        .expect("subscribe run should exist");
    let mut run = run.into_active_model();
    run.status = Set("CORRUPT_STATUS".to_owned());
    run.update(&fixture.database)
        .await
        .expect("refresh projection should be corrupted for the fixture");
    let corrupt = fixture
        .get(
            &format!("/api/v1/subscriptions/{subscription_id}"),
            UserKind::A,
        )
        .await;
    assert_eq!(corrupt.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(
        response_json(corrupt).await["error"]["code"],
        "INTERNAL_ERROR"
    );
}
