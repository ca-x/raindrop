#[allow(dead_code)]
mod support;

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
    db::{DatabaseConfig, connect, entities::category, migrate},
    organization::{CategoryRepository, CreateCategory},
    setup::SetupService,
};
use sea_orm::{ActiveModelTrait, ActiveValue::Set, DatabaseConnection};
use secrecy::{ExposeSecret, SecretString};
use serde_json::{Value, json};
use support::database::{USER_A_ID, USER_B_ID, insert_user};
use tempfile::TempDir;
use time::OffsetDateTime;
use tower::ServiceExt;

struct CategoryFixture {
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

impl CategoryFixture {
    async fn new() -> Self {
        let data = tempfile::tempdir().expect("temporary directory should be created");
        let url = format!(
            "sqlite://{}?mode=rwc",
            data.path().join("category-api.db").display()
        );
        let database = connect(&DatabaseConfig::new(SecretString::from(url)))
            .await
            .expect("category API database should connect");
        migrate(&database)
            .await
            .expect("category API database should migrate");
        insert_user(&database, USER_A_ID, "category-api-a").await;
        insert_user(&database, USER_B_ID, "category-api-b").await;
        let setup = SetupService::ready(data.path(), None, database.clone());
        let session_a = setup
            .sessions()
            .create(USER_A_ID)
            .await
            .expect("user A session should create");
        let session_b = setup
            .sessions()
            .create(USER_B_ID)
            .await
            .expect("user B session should create");
        let user_a_cookie = session_cookie(&session_a, false);
        let user_b_cookie = session_cookie(&session_b, false);
        let user_a_csrf = session_a.csrf_token.expose_secret().to_owned();
        let user_b_csrf = session_b.csrf_token.expose_secret().to_owned();
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

    async fn request(
        &self,
        method: Method,
        uri: &str,
        body: Option<Value>,
        user: Option<UserKind>,
        include_csrf: bool,
    ) -> CapturedResponse {
        let mut request = Request::builder().method(method).uri(uri);
        if let Some(user) = user {
            let (cookie, csrf) = match user {
                UserKind::A => (&self.user_a_cookie, &self.user_a_csrf),
                UserKind::B => (&self.user_b_cookie, &self.user_b_csrf),
            };
            request = request.header(COOKIE, cookie);
            if include_csrf {
                request = request
                    .header("x-csrf-token", csrf)
                    .header(ORIGIN, "http://categories.test")
                    .header(HOST, "categories.test");
            }
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
                    .expect("category request should build"),
            )
            .await
            .expect("category request should complete");
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
                .expect("category response body should collect")
                .to_bytes()
                .to_vec(),
        }
    }

    fn json(&self) -> Value {
        serde_json::from_slice(&self.body).expect("category response should be JSON")
    }
}

#[tokio::test]
async fn category_crud_is_user_scoped_and_uncached() {
    let fixture = CategoryFixture::new().await;
    let empty = fixture
        .request(
            Method::GET,
            "/api/v1/categories",
            None,
            Some(UserKind::A),
            false,
        )
        .await;
    assert_eq!(empty.status, StatusCode::OK);
    assert_eq!(empty.json(), json!({ "items": [] }));
    assert_sensitive_cache_headers(&empty);

    let created = fixture
        .request(
            Method::POST,
            "/api/v1/categories",
            Some(json!({ "title": " Technology " })),
            Some(UserKind::A),
            true,
        )
        .await;
    assert_eq!(created.status, StatusCode::CREATED);
    assert_sensitive_cache_headers(&created);
    let category = created.json();
    let category_id = category["categoryId"]
        .as_str()
        .expect("created category ID should be a string");
    assert_eq!(category["title"], "Technology");
    assert_eq!(category["position"], 1024);
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
            Some(UserKind::A),
            true,
        )
        .await;
    assert_eq!(updated.status, StatusCode::OK);
    assert_eq!(updated.json()["title"], "Science");
    assert_eq!(updated.json()["position"], 512);

    let user_b = fixture
        .request(
            Method::GET,
            "/api/v1/categories",
            None,
            Some(UserKind::B),
            false,
        )
        .await;
    assert_eq!(user_b.json(), json!({ "items": [] }));

    let deleted = fixture
        .request(
            Method::DELETE,
            &format!("/api/v1/categories/{category_id}"),
            None,
            Some(UserKind::A),
            true,
        )
        .await;
    assert_eq!(deleted.status, StatusCode::NO_CONTENT);
    assert!(deleted.body.is_empty());
}

#[tokio::test]
async fn authentication_and_csrf_precede_body_validation() {
    let fixture = CategoryFixture::new().await;
    let unauthenticated = fixture
        .request(
            Method::POST,
            "/api/v1/categories",
            Some(json!({ "unexpected": true })),
            None,
            false,
        )
        .await;
    assert_error(
        &unauthenticated,
        StatusCode::UNAUTHORIZED,
        "AUTHENTICATION_REQUIRED",
    );

    let no_csrf = fixture
        .request(
            Method::POST,
            "/api/v1/categories",
            Some(json!({ "unexpected": true })),
            Some(UserKind::A),
            false,
        )
        .await;
    assert_error(&no_csrf, StatusCode::FORBIDDEN, "FORBIDDEN");

    let invalid_body = fixture
        .request(
            Method::POST,
            "/api/v1/categories",
            Some(json!({ "unexpected": true })),
            Some(UserKind::A),
            true,
        )
        .await;
    assert_error(
        &invalid_body,
        StatusCode::UNPROCESSABLE_ENTITY,
        "VALIDATION_ERROR",
    );
}

#[tokio::test]
async fn validation_conflict_and_cross_user_ids_have_stable_errors() {
    let fixture = CategoryFixture::new().await;
    for (body, field) in [
        (json!({ "title": "" }), "title"),
        (json!({ "position": -1 }), "position"),
        (json!({}), ""),
    ] {
        let response = fixture
            .request(
                if body.get("title").is_some() {
                    Method::POST
                } else {
                    Method::PATCH
                },
                if body.get("title").is_some() {
                    "/api/v1/categories"
                } else {
                    "/api/v1/categories/00000000-0000-4000-8000-000000000599"
                },
                Some(body),
                Some(UserKind::A),
                true,
            )
            .await;
        assert_error(
            &response,
            StatusCode::UNPROCESSABLE_ENTITY,
            "VALIDATION_ERROR",
        );
        if !field.is_empty() {
            assert!(response.json()["error"]["fields"].get(field).is_some());
        }
    }

    let null_patch = fixture
        .request(
            Method::PATCH,
            "/api/v1/categories/00000000-0000-4000-8000-000000000599",
            Some(json!({ "title": null, "position": 5 })),
            Some(UserKind::A),
            true,
        )
        .await;
    assert_error(
        &null_patch,
        StatusCode::UNPROCESSABLE_ENTITY,
        "VALIDATION_ERROR",
    );

    let repository = CategoryRepository::new(fixture.database.clone());
    let user_b_category = repository
        .create(
            USER_B_ID,
            CreateCategory {
                title: "User B".to_owned(),
            },
        )
        .await
        .expect("user B category should create");
    for method in [Method::PATCH, Method::DELETE] {
        let response = fixture
            .request(
                method.clone(),
                &format!("/api/v1/categories/{}", user_b_category.category_id),
                (method == Method::PATCH).then(|| json!({ "title": "Stolen" })),
                Some(UserKind::A),
                true,
            )
            .await;
        assert_error(&response, StatusCode::NOT_FOUND, "NOT_FOUND");
    }

    let first = fixture
        .request(
            Method::POST,
            "/api/v1/categories",
            Some(json!({ "title": "Duplicate" })),
            Some(UserKind::A),
            true,
        )
        .await;
    assert_eq!(first.status, StatusCode::CREATED);
    let duplicate = fixture
        .request(
            Method::POST,
            "/api/v1/categories",
            Some(json!({ "title": "DUPLICATE" })),
            Some(UserKind::A),
            true,
        )
        .await;
    assert_error(&duplicate, StatusCode::CONFLICT, "CONFLICT");
}

#[tokio::test]
async fn category_namespace_has_json_fallback_and_method_contracts() {
    let fixture = CategoryFixture::new().await;
    let trailing = fixture
        .request(
            Method::GET,
            "/api/v1/categories/",
            None,
            Some(UserKind::A),
            false,
        )
        .await;
    assert_error(&trailing, StatusCode::NOT_FOUND, "NOT_FOUND");
    let method = fixture
        .request(
            Method::PUT,
            "/api/v1/categories",
            None,
            Some(UserKind::A),
            false,
        )
        .await;
    assert_error(
        &method,
        StatusCode::METHOD_NOT_ALLOWED,
        "METHOD_NOT_ALLOWED",
    );
    let malformed_id = fixture
        .request(
            Method::DELETE,
            "/api/v1/categories/not-a-uuid",
            None,
            Some(UserKind::A),
            true,
        )
        .await;
    assert_error(
        &malformed_id,
        StatusCode::UNPROCESSABLE_ENTITY,
        "VALIDATION_ERROR",
    );
}

#[tokio::test]
async fn persistent_quota_and_mutation_rate_limit_have_distinct_contracts() {
    let quota_fixture = CategoryFixture::new().await;
    let now = OffsetDateTime::now_utc();
    for index in 0..250_u16 {
        category::ActiveModel {
            id: Set(format!("20000000-0000-4000-8000-{index:012}")),
            user_id: Set(USER_A_ID.to_owned()),
            title: Set(format!("Quota {index}")),
            normalized_title: Set(format!("quota {index}")),
            position: Set(i64::from(index) * 1024),
            created_at: Set(now),
            updated_at: Set(now),
        }
        .insert(&quota_fixture.database)
        .await
        .expect("category quota fixture should insert");
    }
    let quota = quota_fixture
        .request(
            Method::POST,
            "/api/v1/categories",
            Some(json!({ "title": "Over quota" })),
            Some(UserKind::A),
            true,
        )
        .await;
    assert_error(&quota, StatusCode::CONFLICT, "CATEGORY_LIMIT_REACHED");

    let rate_fixture = CategoryFixture::new().await;
    for _ in 0..30 {
        let invalid = rate_fixture
            .request(
                Method::POST,
                "/api/v1/categories",
                Some(json!({ "title": "" })),
                Some(UserKind::A),
                true,
            )
            .await;
        assert_error(
            &invalid,
            StatusCode::UNPROCESSABLE_ENTITY,
            "VALIDATION_ERROR",
        );
    }
    let limited = rate_fixture
        .request(
            Method::POST,
            "/api/v1/categories",
            Some(json!({ "title": "Later" })),
            Some(UserKind::A),
            true,
        )
        .await;
    assert_error(&limited, StatusCode::TOO_MANY_REQUESTS, "RATE_LIMITED");
    assert!(limited.headers.get(RETRY_AFTER).is_some());
    assert!(limited.json()["error"]["fields"]["retryAt"].is_string());
}

fn session_cookie(session: &raindrop::auth::CreatedSession, secure: bool) -> String {
    build_session_cookie(session, secure)
        .to_string()
        .split(';')
        .next()
        .expect("session cookie should contain a pair")
        .to_owned()
}

fn assert_error(response: &CapturedResponse, status: StatusCode, code: &str) {
    assert_eq!(response.status, status);
    assert_eq!(response.json()["error"]["code"], code);
    assert_sensitive_cache_headers(response);
}

fn assert_sensitive_cache_headers(response: &CapturedResponse) {
    assert_eq!(
        response
            .headers
            .get(CACHE_CONTROL)
            .and_then(|value| value.to_str().ok()),
        Some("no-store")
    );
    assert_eq!(
        response
            .headers
            .get(PRAGMA)
            .and_then(|value| value.to_str().ok()),
        Some("no-cache")
    );
}
