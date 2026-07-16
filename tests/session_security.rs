use axum::{
    Json, Router,
    body::Body,
    extract::FromRef,
    http::{Method, Request, StatusCode, header::COOKIE},
    routing::{get, post},
};
use raindrop::{
    auth::{
        CreateAdminInput, CsrfGuard, CurrentUser, PasswordService, SessionService,
        build_session_cookie, create_admin,
    },
    db::{DatabaseConfig, connect, entities::session, migrate},
};
use sea_orm::{ActiveModelTrait, EntityTrait, Set};
use secrecy::{ExposeSecret, SecretString};
use tempfile::tempdir;
use time::{Duration, OffsetDateTime};
use tower::ServiceExt;

#[tokio::test]
async fn session_creation_hashes_tokens_and_builds_secure_cookie() {
    let data = tempdir().expect("temporary directory should be created");
    let url = format!(
        "sqlite://{}?mode=rwc",
        data.path().join("sessions.db").display()
    );
    let database = connect(&DatabaseConfig::new(SecretString::from(url)))
        .await
        .expect("database should connect");
    migrate(&database).await.expect("database should migrate");
    let user = create_admin(
        &database,
        &PasswordService::default(),
        CreateAdminInput {
            username: "Reader".to_owned(),
            password: SecretString::from("correct horse battery staple".to_owned()),
            email: Some("reader@example.com".to_owned()),
        },
    )
    .await
    .expect("admin should be created");

    let sessions = SessionService::new(database.clone());
    let created = sessions
        .create(&user.id)
        .await
        .expect("session should be created");

    assert_eq!(created.cookie_token.expose_secret().len(), 43);
    assert_eq!(created.csrf_token.expose_secret().len(), 43);
    assert_ne!(
        created.cookie_token.expose_secret(),
        created.csrf_token.expose_secret()
    );

    let stored = session::Entity::find()
        .one(&database)
        .await
        .expect("session lookup should work")
        .expect("session should exist");
    assert_eq!(stored.token_hash.len(), 64);
    assert_eq!(stored.csrf_hash.len(), 64);
    assert!(
        !stored
            .token_hash
            .contains(created.cookie_token.expose_secret())
    );
    assert!(
        !stored
            .csrf_hash
            .contains(created.csrf_token.expose_secret())
    );

    let cookie = build_session_cookie(&created, true);
    let rendered = cookie.to_string();
    assert!(rendered.starts_with("raindrop_session="));
    assert!(rendered.contains("HttpOnly"));
    assert!(rendered.contains("SameSite=Lax"));
    assert!(rendered.contains("Secure"));
    assert!(rendered.contains("Path=/"));
    assert!(!rendered.contains("Domain="));
}

#[tokio::test]
async fn current_user_rejects_revoked_and_expired_sessions() {
    let (_data, database, user) = test_identity("session-auth.db").await;
    let sessions = SessionService::new(database.clone());
    let state = TestState {
        sessions: sessions.clone(),
    };
    let app = Router::new()
        .route(
            "/protected",
            get(|CurrentUser(user): CurrentUser| async move { Json(user) }),
        )
        .with_state(state);

    let revoked = sessions
        .create(&user.id)
        .await
        .expect("session should be created");
    sessions
        .revoke(&revoked.cookie_token)
        .await
        .expect("session should be revoked");
    let response = app
        .clone()
        .oneshot(authenticated_request(
            "/protected",
            revoked.cookie_token.expose_secret(),
        ))
        .await
        .expect("request should complete");
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    let expired = sessions
        .create(&user.id)
        .await
        .expect("session should be created");
    let stored = session::Entity::find()
        .one(&database)
        .await
        .expect("session lookup should work")
        .expect("session should exist");
    let mut active: session::ActiveModel = stored.into();
    active.expires_at = Set(OffsetDateTime::now_utc() - Duration::seconds(1));
    active
        .update(&database)
        .await
        .expect("session should be expired");

    let response = app
        .oneshot(authenticated_request(
            "/protected",
            expired.cookie_token.expose_secret(),
        ))
        .await
        .expect("request should complete");
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn csrf_guard_rejects_mismatches_and_cross_origin_requests() {
    let (_data, database, user) = test_identity("session-csrf.db").await;
    let sessions = SessionService::new(database);
    let created = sessions
        .create(&user.id)
        .await
        .expect("session should be created");
    let app = Router::new()
        .route(
            "/protected",
            post(|_guard: CsrfGuard| async { StatusCode::NO_CONTENT }),
        )
        .with_state(TestState { sessions });

    let mismatch = app
        .clone()
        .oneshot(csrf_request(
            created.cookie_token.expose_secret(),
            "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
            None,
        ))
        .await
        .expect("request should complete");
    assert_eq!(mismatch.status(), StatusCode::FORBIDDEN);

    let cross_origin = app
        .clone()
        .oneshot(csrf_request(
            created.cookie_token.expose_secret(),
            created.csrf_token.expose_secret(),
            Some("https://attacker.example"),
        ))
        .await
        .expect("request should complete");
    assert_eq!(cross_origin.status(), StatusCode::FORBIDDEN);

    let accepted = app
        .oneshot(csrf_request(
            created.cookie_token.expose_secret(),
            created.csrf_token.expose_secret(),
            Some("https://reader.example"),
        ))
        .await
        .expect("request should complete");
    assert_eq!(accepted.status(), StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn current_user_throttles_last_seen_writes_to_fifteen_minutes() {
    let (_data, database, user) = test_identity("session-last-seen.db").await;
    let sessions = SessionService::new(database.clone());
    let created = sessions
        .create(&user.id)
        .await
        .expect("session should be created");
    let app = Router::new()
        .route(
            "/protected",
            get(|CurrentUser(user): CurrentUser| async move { Json(user) }),
        )
        .with_state(TestState { sessions });

    let before = session::Entity::find()
        .one(&database)
        .await
        .expect("session lookup should work")
        .expect("session should exist");
    let response = app
        .clone()
        .oneshot(authenticated_request(
            "/protected",
            created.cookie_token.expose_secret(),
        ))
        .await
        .expect("request should complete");
    assert_eq!(response.status(), StatusCode::OK);
    let untouched = session::Entity::find_by_id(&before.token_hash)
        .one(&database)
        .await
        .expect("session lookup should work")
        .expect("session should exist");
    assert_eq!(untouched.last_seen_at, before.last_seen_at);

    let stale_time = OffsetDateTime::now_utc() - Duration::minutes(16);
    let mut active: session::ActiveModel = untouched.into();
    active.last_seen_at = Set(stale_time);
    active
        .update(&database)
        .await
        .expect("last seen should be made stale");
    let response = app
        .oneshot(authenticated_request(
            "/protected",
            created.cookie_token.expose_secret(),
        ))
        .await
        .expect("request should complete");
    assert_eq!(response.status(), StatusCode::OK);
    let refreshed = session::Entity::find_by_id(&before.token_hash)
        .one(&database)
        .await
        .expect("session lookup should work")
        .expect("session should exist");
    assert!(refreshed.last_seen_at > stale_time);
}

#[derive(Clone)]
struct TestState {
    sessions: SessionService,
}

impl FromRef<TestState> for SessionService {
    fn from_ref(state: &TestState) -> Self {
        state.sessions.clone()
    }
}

async fn test_identity(
    database_name: &str,
) -> (
    tempfile::TempDir,
    sea_orm::DatabaseConnection,
    raindrop::auth::User,
) {
    let data = tempdir().expect("temporary directory should be created");
    let url = format!(
        "sqlite://{}?mode=rwc",
        data.path().join(database_name).display()
    );
    let database = connect(&DatabaseConfig::new(SecretString::from(url)))
        .await
        .expect("database should connect");
    migrate(&database).await.expect("database should migrate");
    let user = create_admin(
        &database,
        &PasswordService::default(),
        CreateAdminInput {
            username: "Reader".to_owned(),
            password: SecretString::from("correct horse battery staple".to_owned()),
            email: Some("reader@example.com".to_owned()),
        },
    )
    .await
    .expect("admin should be created");
    (data, database, user)
}

fn authenticated_request(uri: &str, token: &str) -> Request<Body> {
    Request::builder()
        .uri(uri)
        .header(COOKIE, format!("raindrop_session={token}"))
        .body(Body::empty())
        .expect("request should build")
}

fn csrf_request(token: &str, csrf_token: &str, origin: Option<&str>) -> Request<Body> {
    let mut request = Request::builder()
        .method(Method::POST)
        .uri("/protected")
        .header(COOKIE, format!("raindrop_session={token}"))
        .header("x-csrf-token", csrf_token)
        .header("host", "reader.example");
    if let Some(origin) = origin {
        request = request.header("origin", origin);
    }
    request.body(Body::empty()).expect("request should build")
}
