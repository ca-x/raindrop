use axum::{
    Router,
    body::Body,
    http::{
        HeaderMap, Method, Request,
        header::{CONTENT_TYPE, COOKIE, HOST, ORIGIN},
    },
};
use http_body_util::BodyExt;
use raindrop::{
    app::{AppState, build_router},
    auth::build_session_cookie,
    db::{DatabaseConfig, connect, entities::rss_counter, migrate},
    setup::SetupService,
};
use sea_orm::{ActiveModelTrait, ActiveValue::Set, DatabaseConnection};
use secrecy::{ExposeSecret, SecretString};
use serde_json::Value;
use tempfile::TempDir;
use time::OffsetDateTime;
use tower::ServiceExt;

use crate::support::database::{
    ENTRY_A_ID, HASH_A, SUBSCRIPTION_A_ID, USER_A_ID, entry_model, insert_feed, insert_user,
    subscription_model,
};

use super::document::MARK_READ_PATH;

pub(crate) struct ContractFixture {
    _data: TempDir,
    app: Router,
    database: DatabaseConnection,
    cookie: String,
    csrf: String,
}

impl ContractFixture {
    pub(crate) async fn new() -> Self {
        let data = tempfile::tempdir().expect("temporary directory should be created");
        let database_url = format!(
            "sqlite://{}?mode=rwc",
            data.path().join("reader-openapi.db").display()
        );
        let database = connect(&DatabaseConfig::new(SecretString::from(database_url)))
            .await
            .expect("reader OpenAPI database should connect");
        migrate(&database)
            .await
            .expect("reader OpenAPI database should migrate");

        let now = OffsetDateTime::now_utc();
        insert_user(&database, USER_A_ID, "reader-openapi").await;
        insert_feed(&database, now).await;
        let mut subscription = subscription_model(SUBSCRIPTION_A_ID, USER_A_ID, now);
        subscription.start_sequence = Set(0);
        subscription
            .insert(&database)
            .await
            .expect("reader OpenAPI subscription should insert");
        entry_model(
            ENTRY_A_ID,
            1,
            "reader-openapi-entry",
            HASH_A,
            Some(1_784_246_400_000_000),
            now,
        )
        .insert(&database)
        .await
        .expect("reader OpenAPI entry should insert");
        rss_counter::ActiveModel {
            key: Set("INGEST_GENERATION".to_owned()),
            value: Set(1),
        }
        .update(&database)
        .await
        .expect("reader OpenAPI generation should update");

        let setup = SetupService::ready(data.path(), None, database.clone());
        let session = setup
            .sessions()
            .create(USER_A_ID)
            .await
            .expect("reader OpenAPI session should create");
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

    pub(crate) async fn request(
        &self,
        method: Method,
        uri: &str,
        body: Option<Value>,
        authenticated: bool,
        valid_csrf: bool,
    ) -> CapturedResponse {
        let mut request = Request::builder().method(method.clone()).uri(uri);
        if authenticated {
            request = request.header(COOKIE, &self.cookie);
        }
        if method == Method::PATCH || (method == Method::POST && uri == MARK_READ_PATH) {
            request = request
                .header(
                    "x-csrf-token",
                    if valid_csrf {
                        &self.csrf
                    } else {
                        "invalid-csrf"
                    },
                )
                .header(ORIGIN, "http://reader-openapi.test")
                .header(HOST, "reader-openapi.test");
        }
        if body.is_some() {
            request = request.header(CONTENT_TYPE, "application/json");
        }
        let request = request
            .body(body.map_or_else(Body::empty, |value| Body::from(value.to_string())))
            .expect("reader OpenAPI request should build");
        let response = self
            .app
            .clone()
            .oneshot(request)
            .await
            .expect("reader OpenAPI request should complete");
        CapturedResponse::from_response(response).await
    }

    pub(crate) async fn close_database(&self) {
        self.database
            .clone()
            .close()
            .await
            .expect("reader OpenAPI database should close");
    }
}

pub(crate) struct CapturedResponse {
    pub(crate) status: axum::http::StatusCode,
    pub(crate) headers: HeaderMap,
    body: Vec<u8>,
}

impl CapturedResponse {
    async fn from_response(response: axum::response::Response) -> Self {
        let (parts, body) = response.into_parts();
        let body = body
            .collect()
            .await
            .expect("reader OpenAPI response should collect")
            .to_bytes()
            .to_vec();
        Self {
            status: parts.status,
            headers: parts.headers,
            body,
        }
    }

    pub(crate) fn json(&self) -> Value {
        serde_json::from_slice(&self.body).expect("reader response should contain JSON")
    }

    pub(crate) fn body_is_empty(&self) -> bool {
        self.body.is_empty()
    }
}
