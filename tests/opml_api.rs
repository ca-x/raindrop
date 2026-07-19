#[allow(dead_code)]
mod support;

use axum::{
    Router,
    body::Body,
    http::{
        Method, Request, StatusCode,
        header::{CONTENT_DISPOSITION, CONTENT_TYPE, COOKIE, HOST, ORIGIN},
    },
};
use http_body_util::BodyExt;
use raindrop::{
    app::{AppState, build_router},
    auth::build_session_cookie,
    db::{
        DatabaseConfig, connect,
        entities::{category, feed, subscription},
        migrate,
    },
    setup::SetupService,
};
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, DatabaseConnection, EntityTrait,
    PaginatorTrait, QueryFilter,
};
use secrecy::{ExposeSecret, SecretString};
use serde_json::Value;
use tempfile::TempDir;
use time::OffsetDateTime;
use tower::ServiceExt;
use uuid::Uuid;

use support::database::{USER_A_ID, USER_B_ID, insert_user};

struct OpmlApiFixture {
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

impl OpmlApiFixture {
    async fn new() -> Self {
        let data = tempfile::tempdir().expect("temporary directory should be created");
        let database_url = format!(
            "sqlite://{}?mode=rwc",
            data.path().join("opml-api.db").display()
        );
        let database = connect(&DatabaseConfig::new(SecretString::from(database_url)))
            .await
            .expect("OPML API database should connect");
        migrate(&database)
            .await
            .expect("OPML API database should migrate");
        insert_user(&database, USER_A_ID, "opml-a").await;
        insert_user(&database, USER_B_ID, "opml-b").await;
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
        let app = build_router(AppState::new(setup));

        Self {
            _data: data,
            app,
            database,
            user_a_cookie: session_cookie(&user_a_session),
            user_a_csrf: user_a_session.csrf_token.expose_secret().to_owned(),
            user_b_cookie: session_cookie(&user_b_session),
            user_b_csrf: user_b_session.csrf_token.expose_secret().to_owned(),
        }
    }

    async fn import(
        &self,
        mode: &str,
        document: impl Into<Body>,
        user: UserKind,
        with_csrf: bool,
    ) -> axum::response::Response {
        let (cookie, csrf) = self.credentials(user);
        let mut request = Request::builder()
            .method(Method::POST)
            .uri(format!("/api/v1/imports/opml?mode={mode}"))
            .header(COOKIE, cookie)
            .header(CONTENT_TYPE, "application/xml")
            .header(ORIGIN, "http://opml.test")
            .header(HOST, "opml.test");
        if with_csrf {
            request = request.header("x-csrf-token", csrf);
        }
        self.app
            .clone()
            .oneshot(
                request
                    .body(document.into())
                    .expect("OPML request should build"),
            )
            .await
            .expect("OPML request should complete")
    }

    async fn export(&self, user: UserKind) -> axum::response::Response {
        let (cookie, _) = self.credentials(user);
        self.app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri("/api/v1/exports/opml")
                    .header(COOKIE, cookie)
                    .body(Body::empty())
                    .expect("OPML export request should build"),
            )
            .await
            .expect("OPML export request should complete")
    }

    fn credentials(&self, user: UserKind) -> (&str, &str) {
        match user {
            UserKind::A => (&self.user_a_cookie, &self.user_a_csrf),
            UserKind::B => (&self.user_b_cookie, &self.user_b_csrf),
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

async fn response_bytes(response: axum::response::Response) -> axum::body::Bytes {
    response
        .into_body()
        .collect()
        .await
        .expect("response body should collect")
        .to_bytes()
}

async fn response_json(response: axum::response::Response) -> Value {
    serde_json::from_slice(&response_bytes(response).await)
        .expect("response should contain valid JSON")
}

fn sample_opml() -> &'static str {
    r#"<?xml version="1.0" encoding="UTF-8"?>
<opml version="2.0">
  <head><title>Reader export</title></head>
  <body>
    <outline text="Technology">
      <outline text="IT &amp; Home" xmlUrl="https://www.ithome.com/rss/"/>
      <outline text="Duplicate" xmlUrl="https://www.ithome.com/rss/"/>
    </outline>
    <outline text="Example" xmlUrl="https://example.com/feed.xml"/>
    <outline text="Broken" xmlUrl="javascript:alert(1)"/>
  </body>
</opml>"#
}

#[tokio::test]
async fn preview_commit_export_round_trip_is_duplicate_safe_and_user_scoped() {
    let fixture = OpmlApiFixture::new().await;

    let preview = fixture
        .import("preview", sample_opml(), UserKind::A, true)
        .await;
    assert_eq!(preview.status(), StatusCode::OK);
    let preview = response_json(preview).await;
    assert_eq!(preview["mode"], "PREVIEW");
    assert_eq!(preview["outlineCount"], 5);
    assert_eq!(preview["validCount"], 2);
    assert_eq!(preview["newCount"], 2);
    assert_eq!(preview["duplicateCount"], 1);
    assert_eq!(preview["invalidCount"], 1);
    assert_eq!(preview["categoryCount"], 1);
    assert_eq!(
        subscription::Entity::find()
            .filter(subscription::Column::UserId.eq(USER_A_ID))
            .count(&fixture.database)
            .await
            .expect("subscriptions should count"),
        0
    );

    let committed = fixture
        .import("commit", sample_opml(), UserKind::A, true)
        .await;
    assert_eq!(committed.status(), StatusCode::OK);
    let committed = response_json(committed).await;
    assert_eq!(committed["mode"], "COMMIT");
    assert_eq!(committed["importedCount"], 2);
    assert_eq!(committed["duplicateCount"], 1);
    assert_eq!(committed["createdCategoryCount"], 1);

    let repeated = fixture
        .import("commit", sample_opml(), UserKind::A, true)
        .await;
    assert_eq!(repeated.status(), StatusCode::OK);
    let repeated = response_json(repeated).await;
    assert_eq!(repeated["importedCount"], 0);
    assert_eq!(repeated["duplicateCount"], 3);

    assert_eq!(
        subscription::Entity::find()
            .filter(subscription::Column::UserId.eq(USER_A_ID))
            .count(&fixture.database)
            .await
            .expect("user A subscriptions should count"),
        2
    );
    assert_eq!(
        subscription::Entity::find()
            .filter(subscription::Column::UserId.eq(USER_B_ID))
            .count(&fixture.database)
            .await
            .expect("user B subscriptions should count"),
        0
    );

    let exported = fixture.export(UserKind::A).await;
    assert_eq!(exported.status(), StatusCode::OK);
    assert_eq!(
        exported.headers().get(CONTENT_TYPE).unwrap(),
        "application/xml; charset=utf-8"
    );
    assert_eq!(
        exported.headers().get(CONTENT_DISPOSITION).unwrap(),
        "attachment; filename=\"raindrop.opml\""
    );
    let exported = response_bytes(exported).await;
    let exported_text = std::str::from_utf8(&exported).expect("export should be UTF-8");
    assert!(exported_text.contains("Technology"));
    assert!(exported_text.contains("IT &amp; Home"));
    assert!(exported_text.contains("https://www.ithome.com/rss/"));

    let exported_preview = fixture.import("preview", exported, UserKind::A, true).await;
    assert_eq!(exported_preview.status(), StatusCode::OK);
    let exported_preview = response_json(exported_preview).await;
    assert_eq!(exported_preview["newCount"], 0);
    assert_eq!(exported_preview["duplicateCount"], 2);

    let user_b_export = fixture.export(UserKind::B).await;
    let user_b_export = response_bytes(user_b_export).await;
    assert!(
        !std::str::from_utf8(&user_b_export)
            .expect("user B export should be UTF-8")
            .contains("ithome")
    );
}

#[tokio::test]
async fn imports_require_authentication_and_csrf_and_reject_malformed_xml() {
    let fixture = OpmlApiFixture::new().await;
    let missing_csrf = fixture
        .import("commit", sample_opml(), UserKind::A, false)
        .await;
    assert_eq!(missing_csrf.status(), StatusCode::FORBIDDEN);

    let unauthenticated = fixture
        .app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/api/v1/imports/opml?mode=preview")
                .header(CONTENT_TYPE, "application/xml")
                .body(Body::from(sample_opml()))
                .expect("unauthenticated request should build"),
        )
        .await
        .expect("unauthenticated request should complete");
    assert_eq!(unauthenticated.status(), StatusCode::UNAUTHORIZED);

    let malformed = fixture
        .import(
            "preview",
            "<opml><body><outline xmlUrl=\"https://example.com/rss\"></body>",
            UserKind::A,
            true,
        )
        .await;
    assert_eq!(malformed.status(), StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn opml_route_accepts_documents_above_the_global_json_body_limit() {
    let fixture = OpmlApiFixture::new().await;
    let padding = "x".repeat(70 * 1024);
    let document = format!(
        "<?xml version=\"1.0\"?><opml version=\"2.0\"><head><!--{padding}--></head><body><outline text=\"IT Home\" xmlUrl=\"https://www.ithome.com/rss/\"/></body></opml>"
    );
    let response = fixture.import("preview", document, UserKind::A, true).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response_json(response).await["newCount"], 1);
}

#[tokio::test]
async fn category_limit_failure_rolls_back_new_feed_and_subscription_rows() {
    let fixture = OpmlApiFixture::new().await;
    let now = OffsetDateTime::now_utc();
    for index in 0..250_u128 {
        category::ActiveModel {
            id: Set(Uuid::from_u128(0x9000_0000_0000_4000_8000_0000_0000_0000 + index).to_string()),
            user_id: Set(USER_A_ID.to_owned()),
            title: Set(format!("Category {index}")),
            normalized_title: Set(format!("category {index}")),
            position: Set(i64::try_from(index + 1).expect("category position should fit") * 1_024),
            created_at: Set(now),
            updated_at: Set(now),
        }
        .insert(&fixture.database)
        .await
        .expect("category should insert");
    }
    let feed_count_before = feed::Entity::find()
        .count(&fixture.database)
        .await
        .expect("feeds should count");
    let response = fixture
        .import(
            "commit",
            r#"<opml version="2.0"><body><outline text="Overflow"><outline text="New" xmlUrl="https://rollback.example/rss.xml"/></outline></body></opml>"#,
            UserKind::A,
            true,
        )
        .await;
    assert_eq!(response.status(), StatusCode::CONFLICT);
    assert_eq!(
        feed::Entity::find()
            .count(&fixture.database)
            .await
            .expect("feeds should count after rollback"),
        feed_count_before
    );
    assert_eq!(
        subscription::Entity::find()
            .filter(subscription::Column::UserId.eq(USER_A_ID))
            .count(&fixture.database)
            .await
            .expect("subscriptions should count after rollback"),
        0
    );
}
