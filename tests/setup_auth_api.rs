use axum::{
    body::Body,
    http::{
        Method, Request, StatusCode,
        header::{CONTENT_TYPE, COOKIE, SET_COOKIE},
    },
};
use http_body_util::BodyExt;
use raindrop::{
    app::{AppState, build_router},
    db::{DatabaseConfig, connect, entities::user},
    setup::SetupService,
};
use sea_orm::EntityTrait;
use secrecy::SecretString;
use serde_json::{Value, json};
use tempfile::tempdir;
use tower::ServiceExt;

#[tokio::test]
async fn setup_routes_require_the_terminal_token_without_leaking_it_or_database_secrets() {
    let data = tempdir().expect("temporary directory should be created");
    let setup_token = "rd_setup_test_only_terminal_token";
    let app = build_router(AppState::new(SetupService::required(
        data.path(),
        SecretString::from(setup_token.to_owned()),
        None,
    )));

    let response = app
        .clone()
        .oneshot(api_request(Method::GET, "/api/v1/bootstrap", None, None))
        .await
        .expect("request should complete");
    assert_eq!(response.status(), StatusCode::OK);
    let body = response_json(response).await;
    assert_eq!(body["status"], "SETUP_REQUIRED");
    assert_eq!(body["version"], env!("CARGO_PKG_VERSION"));
    assert!(!body.to_string().contains(setup_token));

    let response = app
        .clone()
        .oneshot(api_request(
            Method::POST,
            "/api/v1/setup/complete",
            Some(json!({
                "databaseUrl": "sqlite://ignored.db?mode=rwc",
                "username": "Reader",
                "password": "correct horse battery staple"
            })),
            None,
        ))
        .await
        .expect("request should complete");
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    let body = response_json(response).await;
    assert_eq!(body["error"]["code"], "SETUP_TOKEN_REQUIRED");
    assert!(body["error"]["requestId"].is_string());
    assert!(!body.to_string().contains(setup_token));

    let response = app
        .oneshot(api_request(
            Method::POST,
            "/api/v1/setup/database-check",
            Some(json!({
                "databaseUrl": "ftp://reader:super-secret@database.example/raindrop"
            })),
            Some(setup_token),
        ))
        .await
        .expect("request should complete");
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let body = response_json(response).await;
    assert_eq!(body["error"]["code"], "VALIDATION_ERROR");
    assert!(body["error"]["fields"]["databaseUrl"].is_string());
    assert!(!body.to_string().contains("super-secret"));
}

#[tokio::test]
async fn setup_completion_writes_private_config_creates_one_admin_and_closes_setup() {
    let data = tempdir().expect("temporary directory should be created");
    let setup_token = "rd_setup_completion_test_token";
    let database_url = format!(
        "sqlite://{}?mode=rwc",
        data.path().join("raindrop.db").display()
    );
    let app = build_router(AppState::new(SetupService::required(
        data.path(),
        SecretString::from(setup_token.to_owned()),
        None,
    )));
    let setup_body = json!({
        "databaseUrl": database_url,
        "username": "Reader",
        "password": "correct horse battery staple",
        "email": "reader@example.com"
    });

    let response = app
        .clone()
        .oneshot(api_request(
            Method::POST,
            "/api/v1/setup/complete",
            Some(setup_body.clone()),
            Some(setup_token),
        ))
        .await
        .expect("request should complete");
    assert_eq!(response.status(), StatusCode::OK);
    let body = response_json(response).await;
    assert_eq!(body["status"], "READY");
    assert_eq!(body["user"]["username"], "Reader");
    assert!(!body.to_string().contains("correct horse battery staple"));

    let config_path = data.path().join("config.toml");
    let config = std::fs::read_to_string(&config_path).expect("config should be written");
    assert!(config.contains("database_url"));
    assert!(config.contains("raindrop.db"));
    assert!(!config.contains("correct horse battery staple"));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&config_path)
            .expect("config metadata should be readable")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600);
    }

    let database = connect(&DatabaseConfig::new(SecretString::from(
        database_url.clone(),
    )))
    .await
    .expect("configured database should connect");
    assert_eq!(
        user::Entity::find()
            .all(&database)
            .await
            .expect("users should load")
            .len(),
        1
    );

    let response = app
        .oneshot(api_request(
            Method::POST,
            "/api/v1/setup/complete",
            Some(setup_body),
            Some(setup_token),
        ))
        .await
        .expect("request should complete");
    assert_eq!(response.status(), StatusCode::CONFLICT);
    let body = response_json(response).await;
    assert_eq!(body["error"]["code"], "SETUP_ALREADY_COMPLETE");
}

#[tokio::test]
async fn login_session_and_csrf_logout_form_a_revocable_browser_session() {
    let data = tempdir().expect("temporary directory should be created");
    let setup_token = "rd_setup_auth_flow_test_token";
    let database_url = format!(
        "sqlite://{}?mode=rwc",
        data.path().join("auth-flow.db").display()
    );
    let app = build_router(AppState::new(SetupService::required(
        data.path(),
        SecretString::from(setup_token.to_owned()),
        None,
    )));
    let response = app
        .clone()
        .oneshot(api_request(
            Method::POST,
            "/api/v1/setup/complete",
            Some(json!({
                "databaseUrl": database_url,
                "username": "Reader",
                "password": "correct horse battery staple",
                "email": "reader@example.com"
            })),
            Some(setup_token),
        ))
        .await
        .expect("setup request should complete");
    assert_eq!(response.status(), StatusCode::OK);

    let response = app
        .clone()
        .oneshot(api_request(
            Method::POST,
            "/api/v1/auth/login",
            Some(json!({
                "login": "reader@example.com",
                "password": "correct horse battery staple"
            })),
            None,
        ))
        .await
        .expect("login request should complete");
    assert_eq!(response.status(), StatusCode::OK);
    let set_cookie = response
        .headers()
        .get(SET_COOKIE)
        .expect("login should set a cookie")
        .to_str()
        .expect("cookie should be text")
        .to_owned();
    assert!(set_cookie.contains("HttpOnly"));
    assert!(set_cookie.contains("SameSite=Lax"));
    let request_cookie = set_cookie
        .split(';')
        .next()
        .expect("cookie should contain name and value")
        .to_owned();
    let login_body = response_json(response).await;
    assert_eq!(login_body["user"]["username"], "Reader");
    let login_csrf_token = login_body["csrfToken"]
        .as_str()
        .expect("login should return a CSRF token")
        .to_owned();
    assert_eq!(login_csrf_token.len(), 43);

    let response = app
        .clone()
        .oneshot(auth_request(
            Method::GET,
            "/api/v1/auth/session",
            &request_cookie,
            None,
        ))
        .await
        .expect("session request should complete");
    assert_eq!(response.status(), StatusCode::OK);
    let session_body = response_json(response).await;
    let csrf_token = session_body["csrfToken"]
        .as_str()
        .expect("session should return a CSRF token")
        .to_owned();
    assert_eq!(session_body["user"]["username"], "Reader");
    assert_eq!(csrf_token, login_csrf_token);

    let response = app
        .clone()
        .oneshot(auth_request(
            Method::POST,
            "/api/v1/auth/logout",
            &request_cookie,
            Some(&csrf_token),
        ))
        .await
        .expect("logout request should complete");
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    let cleared = response
        .headers()
        .get(SET_COOKIE)
        .expect("logout should clear the cookie")
        .to_str()
        .expect("cookie should be text");
    assert!(cleared.contains("Max-Age=0"));

    let response = app
        .oneshot(auth_request(
            Method::GET,
            "/api/v1/auth/session",
            &request_cookie,
            None,
        ))
        .await
        .expect("session request should complete");
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    let body = response_json(response).await;
    assert_eq!(body["error"]["code"], "AUTHENTICATION_REQUIRED");
}

#[tokio::test]
async fn malformed_json_uses_the_uniform_validation_error_contract() {
    let data = tempdir().expect("temporary directory should be created");
    let app = build_router(AppState::new(SetupService::required(
        data.path(),
        SecretString::from("rd_setup_json_test_token".to_owned()),
        None,
    )));
    let response = app
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/api/v1/auth/login")
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from("{"))
                .expect("request should build"),
        )
        .await
        .expect("request should complete");
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let body = response_json(response).await;
    assert_eq!(body["error"]["code"], "VALIDATION_ERROR");
    assert!(body["error"]["requestId"].is_string());
    assert!(!body.to_string().contains("line 1 column"));
}

#[tokio::test]
async fn repeated_login_attempts_are_rate_limited_with_a_stable_error_contract() {
    let data = tempdir().expect("temporary directory should be created");
    let setup_token = "rd_setup_rate_limit_test_token";
    let database_url = format!(
        "sqlite://{}?mode=rwc",
        data.path().join("rate-limit.db").display()
    );
    let app = build_router(AppState::new(SetupService::required(
        data.path(),
        SecretString::from(setup_token.to_owned()),
        None,
    )));
    let response = app
        .clone()
        .oneshot(api_request(
            Method::POST,
            "/api/v1/setup/complete",
            Some(json!({
                "databaseUrl": database_url,
                "username": "Reader",
                "password": "correct horse battery staple"
            })),
            Some(setup_token),
        ))
        .await
        .expect("setup request should complete");
    assert_eq!(response.status(), StatusCode::OK);

    for _ in 0..10 {
        let response = app
            .clone()
            .oneshot(api_request(
                Method::POST,
                "/api/v1/auth/login",
                Some(json!({
                    "login": "reader",
                    "password": "wrong password value"
                })),
                None,
            ))
            .await
            .expect("login request should complete");
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    let response = app
        .oneshot(api_request(
            Method::POST,
            "/api/v1/auth/login",
            Some(json!({
                "login": "reader",
                "password": "wrong password value"
            })),
            None,
        ))
        .await
        .expect("login request should complete");
    assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
    let body = response_json(response).await;
    assert_eq!(body["error"]["code"], "RATE_LIMITED");
}

fn api_request(
    method: Method,
    uri: &str,
    body: Option<Value>,
    setup_token: Option<&str>,
) -> Request<Body> {
    let mut request = Request::builder().method(method).uri(uri);
    if body.is_some() {
        request = request.header(CONTENT_TYPE, "application/json");
    }
    if let Some(setup_token) = setup_token {
        request = request.header("x-setup-token", setup_token);
    }
    request
        .body(body.map_or_else(Body::empty, |value| Body::from(value.to_string())))
        .expect("request should build")
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

fn auth_request(
    method: Method,
    uri: &str,
    cookie: &str,
    csrf_token: Option<&str>,
) -> Request<Body> {
    let mut request = Request::builder()
        .method(method)
        .uri(uri)
        .header(COOKIE, cookie);
    if let Some(csrf_token) = csrf_token {
        request = request.header("x-csrf-token", csrf_token);
    }
    request.body(Body::empty()).expect("request should build")
}
