use std::net::SocketAddr;

use axum::{
    body::Body,
    extract::connect_info::ConnectInfo,
    http::{
        Method, Request, StatusCode,
        header::{CACHE_CONTROL, CONTENT_TYPE, COOKIE, PRAGMA, SET_COOKIE},
    },
};
use http_body_util::BodyExt;
use raindrop::{
    app::{AppState, build_router},
    db::{DatabaseConfig, connect, entities::user, migrate},
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
    assert_eq!(body["setupMode"], "FULL");
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
async fn configured_empty_database_exposes_only_admin_setup_and_completes_it() {
    let data = tempdir().expect("temporary directory should be created");
    let setup_token = "rd_setup_admin_only_test_token";
    let database_url = format!(
        "sqlite://{}?mode=rwc",
        data.path().join("managed.db").display()
    );
    let database = connect(&DatabaseConfig::new(SecretString::from(
        database_url.clone(),
    )))
    .await
    .expect("database should connect");
    migrate(&database).await.expect("database should migrate");
    let app = build_router(AppState::new(SetupService::admin_only(
        data.path(),
        SecretString::from(setup_token.to_owned()),
        None,
        database.clone(),
    )));

    let response = app
        .clone()
        .oneshot(api_request(Method::GET, "/api/v1/bootstrap", None, None))
        .await
        .expect("bootstrap should complete");
    assert_eq!(response.status(), StatusCode::OK);
    let body = response_json(response).await;
    assert_eq!(body["status"], "SETUP_REQUIRED");
    assert_eq!(body["setupMode"], "ADMIN_ONLY");
    assert!(body.get("databaseUrl").is_none());

    for path in ["/api/v1/setup/database-check", "/api/v1/setup/complete"] {
        let body = if path.ends_with("database-check") {
            json!({ "databaseUrl": "sqlite://attacker.db?mode=rwc" })
        } else {
            json!({
                "databaseUrl": "sqlite://attacker.db?mode=rwc",
                "username": "Attacker",
                "password": "correct horse battery staple"
            })
        };
        let response = app
            .clone()
            .oneshot(api_request(
                Method::POST,
                path,
                Some(body),
                Some(setup_token),
            ))
            .await
            .expect("restricted setup request should complete");
        assert_eq!(response.status(), StatusCode::CONFLICT);
    }

    let response = app
        .clone()
        .oneshot(api_request(
            Method::POST,
            "/api/v1/setup/admin",
            Some(json!({
                "username": "Reader",
                "password": "correct horse battery staple",
                "email": " reader@example.com "
            })),
            Some(setup_token),
        ))
        .await
        .expect("administrator request should complete");
    assert_eq!(response.status(), StatusCode::OK);
    assert_sensitive_cache_headers(&response);
    let body = response_json(response).await;
    assert_eq!(body["status"], "READY");
    assert_eq!(body["user"]["email"], "reader@example.com");
    assert!(!data.path().join("config.toml").exists());

    let response = app
        .oneshot(api_request(Method::GET, "/api/v1/bootstrap", None, None))
        .await
        .expect("bootstrap should complete");
    let body = response_json(response).await;
    assert_eq!(body["status"], "READY");
    assert!(body.get("setupMode").is_none());
    assert_eq!(
        user::Entity::find()
            .all(&database)
            .await
            .expect("users should load")
            .len(),
        1
    );
}

#[tokio::test]
async fn full_setup_creates_and_secures_a_missing_data_directory_before_sqlite_connect() {
    let root = tempdir().expect("temporary directory should be created");
    let data_dir = root.path().join("missing").join("data");
    let setup_token = "rd_setup_missing_data_dir_token";
    let database_url = format!(
        "sqlite://{}?mode=rwc",
        data_dir.join("raindrop.db").display()
    );
    let app = build_router(AppState::new(SetupService::required(
        &data_dir,
        SecretString::from(setup_token.to_owned()),
        None,
    )));
    assert!(!data_dir.exists());

    let response = app
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
    assert!(data_dir.join("raindrop.db").exists());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&data_dir)
            .expect("data directory metadata should load")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o700);
    }
}

#[tokio::test]
async fn separate_setup_services_share_one_database_first_administrator_claim() {
    let data = tempdir().expect("temporary directory should be created");
    let database_url = format!(
        "sqlite://{}?mode=rwc",
        data.path().join("claim.db").display()
    );
    let migrated = connect(&DatabaseConfig::new(SecretString::from(
        database_url.clone(),
    )))
    .await
    .expect("database should connect");
    migrate(&migrated).await.expect("database should migrate");
    migrated.close().await.expect("database should close");
    let first_dir = data.path().join("first");
    let second_dir = data.path().join("second");
    let first = SetupService::required(
        &first_dir,
        SecretString::from("rd_setup_first_claim_token".to_owned()),
        None,
    );
    let second = SetupService::required(
        &second_dir,
        SecretString::from("rd_setup_second_claim_token".to_owned()),
        None,
    );

    let (first_result, second_result) = tokio::join!(
        first.complete(
            "rd_setup_first_claim_token",
            raindrop::setup::SetupCompleteInput {
                database_url: SecretString::from(database_url.clone()),
                username: "FirstReader".to_owned(),
                password: SecretString::from("correct horse battery staple".to_owned()),
                email: None,
            },
        ),
        second.complete(
            "rd_setup_second_claim_token",
            raindrop::setup::SetupCompleteInput {
                database_url: SecretString::from(database_url.clone()),
                username: "SecondReader".to_owned(),
                password: SecretString::from("another correct horse battery staple".to_owned()),
                email: None,
            },
        )
    );

    assert_ne!(first_result.is_ok(), second_result.is_ok());
    let loser = if first_result.is_err() {
        first_result.expect_err("first should lose")
    } else {
        second_result.expect_err("second should lose")
    };
    assert!(
        matches!(loser, raindrop::setup::SetupError::AlreadyComplete),
        "unexpected loser error: {loser:?}"
    );
    let database = connect(&DatabaseConfig::new(SecretString::from(database_url)))
        .await
        .expect("database should reconnect");
    assert_eq!(
        user::Entity::find()
            .all(&database)
            .await
            .expect("users should load")
            .len(),
        1
    );
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
    assert_sensitive_cache_headers(&response);
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
    let mut request = Request::builder()
        .method(Method::POST)
        .uri("/api/v1/auth/login")
        .header(CONTENT_TYPE, "application/json")
        .body(Body::from("{"))
        .expect("request should build");
    request.extensions_mut().insert(ConnectInfo(
        "127.0.0.1:40000"
            .parse::<SocketAddr>()
            .expect("peer should parse"),
    ));
    let response = app.oneshot(request).await.expect("request should complete");
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let body = response_json(response).await;
    assert_eq!(body["error"]["code"], "VALIDATION_ERROR");
    assert!(body["error"]["requestId"].is_string());
    assert!(!body.to_string().contains("line 1 column"));
}

#[tokio::test]
async fn more_than_ten_successful_logins_from_one_peer_are_not_proxy_locked() {
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

    let peer = "192.0.2.10:41000".parse().expect("peer should parse");
    for attempt in 0..11 {
        let response = app
            .clone()
            .oneshot(api_request_from(
                Method::POST,
                "/api/v1/auth/login",
                Some(json!({
                    "login": "reader",
                    "password": "correct horse battery staple"
                })),
                None,
                peer,
                Some(&format!("198.51.100.{attempt}")),
            ))
            .await
            .expect("login request should complete");
        assert_eq!(
            response.status(),
            StatusCode::OK,
            "attempt {attempt} was unexpectedly denied"
        );
    }
}

#[tokio::test]
async fn rotating_peers_and_structurally_invalid_requests_cannot_exhaust_login_capacity() {
    let data = tempdir().expect("temporary directory should be created");
    let setup_token = "rd_setup_rotating_peer_test_token";
    let database_url = format!(
        "sqlite://{}?mode=rwc",
        data.path().join("rotating-peer.db").display()
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

    for attempt in 0..10_001_u32 {
        let octets = [
            10,
            ((attempt >> 16) & 0xff) as u8,
            ((attempt >> 8) & 0xff) as u8,
            (attempt & 0xff) as u8,
        ];
        let peer = SocketAddr::from((octets, 40_000));
        let response = app
            .clone()
            .oneshot(api_request_from(
                Method::POST,
                "/api/v1/auth/login",
                Some(json!({
                    "login": format!("missing-{attempt}"),
                    "password": ""
                })),
                None,
                peer,
                Some(&format!("198.51.{}.{}", attempt / 256, attempt % 256)),
            ))
            .await
            .expect("invalid login request should complete");
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    let response = app
        .oneshot(api_request_from(
            Method::POST,
            "/api/v1/auth/login",
            Some(json!({
                "login": "reader",
                "password": "correct horse battery staple"
            })),
            None,
            "203.0.113.250:41000".parse().expect("peer should parse"),
            Some("203.0.113.250"),
        ))
        .await
        .expect("login request should complete");
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn account_failures_never_lock_out_correct_credentials_from_another_peer() {
    let data = tempdir().expect("temporary directory should be created");
    let setup_token = "rd_setup_account_delay_test_token";
    let database_url = format!(
        "sqlite://{}?mode=rwc",
        data.path().join("account-delay.db").display()
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

    for attempt in 0..10 {
        let peer = format!("192.0.2.{}:{}", attempt + 20, 42000 + attempt)
            .parse()
            .expect("peer should parse");
        let response = app
            .clone()
            .oneshot(api_request_from(
                Method::POST,
                "/api/v1/auth/login",
                Some(json!({
                    "login": "reader",
                    "password": "wrong password value"
                })),
                None,
                peer,
                Some("203.0.113.44"),
            ))
            .await
            .expect("login request should complete");
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    let response = app
        .oneshot(api_request_from(
            Method::POST,
            "/api/v1/auth/login",
            Some(json!({
                "login": "reader",
                "password": "correct horse battery staple"
            })),
            None,
            "198.51.100.70:43000".parse().expect("peer should parse"),
            Some("192.0.2.10"),
        ))
        .await
        .expect("login request should complete");
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn setup_config_parse_errors_discard_secret_source_input() {
    let data = tempdir().expect("temporary directory should be created");
    let setup_token = "rd_setup_parse_redaction_token";
    let sentinels = [
        "setup-database-sentinel-2c8a",
        "setup-session-sentinel-4f6d",
        "setup-password-sentinel-1e9b",
    ];
    std::fs::write(
        data.path().join("config.toml"),
        format!(
            "database_url = \"{}\"\nsession_secret = \"{}\"\n[bootstrap_admin]\npassword = \"{}\n",
            sentinels[0], sentinels[1], sentinels[2]
        ),
    )
    .expect("configuration file should be written");
    let database_url = format!(
        "sqlite://{}?mode=rwc",
        data.path().join("parse-redaction.db").display()
    );
    let service = SetupService::required(
        data.path(),
        SecretString::from(setup_token.to_owned()),
        None,
    );

    let error = service
        .complete(
            setup_token,
            raindrop::setup::SetupCompleteInput {
                database_url: SecretString::from(database_url),
                username: "Reader".to_owned(),
                password: SecretString::from("correct horse battery staple".to_owned()),
                email: None,
            },
        )
        .await
        .expect_err("malformed existing configuration should fail");
    let chain = error_chain(&error);

    assert!(chain.contains("configuration could not be read"));
    for sentinel in sentinels {
        assert!(!chain.contains(sentinel), "error disclosed {sentinel}");
    }
}

#[tokio::test]
async fn invalid_administrator_email_is_a_redacted_field_validation_error() {
    let invalid_emails = [
        "missing-at.example.com".to_owned(),
        "two@@example.com".to_owned(),
        "local part@example.com".to_owned(),
        format!("{}@example.com", "l".repeat(65)),
        format!("reader@{}", "d".repeat(256)),
        format!("{}@example.com", "x".repeat(310)),
        "reader@exam\u{0007}ple.com".to_owned(),
        "\u{0130}@example.com".to_owned(),
    ];

    for (index, email) in invalid_emails.into_iter().enumerate() {
        let data = tempdir().expect("temporary directory should be created");
        let setup_token = format!("rd_setup_email_validation_token_{index}");
        let database_url = format!(
            "sqlite://{}?mode=rwc",
            data.path().join(format!("email-{index}.db")).display()
        );
        let app = build_router(AppState::new(SetupService::required(
            data.path(),
            SecretString::from(setup_token.clone()),
            None,
        )));

        let response = app
            .oneshot(api_request(
                Method::POST,
                "/api/v1/setup/complete",
                Some(json!({
                    "databaseUrl": database_url,
                    "username": "Reader",
                    "password": "correct horse battery staple",
                    "email": email
                })),
                Some(&setup_token),
            ))
            .await
            .expect("request should complete");

        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
        let body = response_json(response).await;
        assert_eq!(body["error"]["code"], "VALIDATION_ERROR");
        assert!(body["error"]["fields"]["email"].is_string());
        assert!(!body.to_string().contains(&email));
        assert!(!data.path().join("config.toml").exists());
    }
}

#[tokio::test]
async fn every_auth_and_setup_response_disables_caching_including_rejections() {
    let data = tempdir().expect("temporary directory should be created");
    let setup_token = "rd_setup_cache_contract_token";
    let database_url = format!(
        "sqlite://{}?mode=rwc",
        data.path().join("cache-contract.db").display()
    );
    let app = build_router(AppState::new(SetupService::required(
        data.path(),
        SecretString::from(setup_token.to_owned()),
        None,
    )));

    for request in [
        api_request(Method::GET, "/api/v1/bootstrap", None, None),
        api_request(
            Method::POST,
            "/api/v1/setup/complete",
            Some(json!({})),
            None,
        ),
        api_request(Method::GET, "/api/v1/setup/complete", None, None),
        api_request(Method::GET, "/api/v1/setup/missing", None, None),
        api_request(Method::GET, "/api/v1/auth/login", None, None),
        api_request(Method::GET, "/api/v1/auth/missing", None, None),
        api_request(Method::GET, "/api/v1/auth/session", None, None),
        auth_request(
            Method::GET,
            "/api/v1/auth/session",
            "raindrop_session=invalid",
            None,
        ),
    ] {
        let response = app
            .clone()
            .oneshot(request)
            .await
            .expect("sensitive request should complete");
        assert_sensitive_cache_headers(&response);
    }

    let mut duplicate_token = api_request(
        Method::POST,
        "/api/v1/setup/database-check",
        Some(json!({ "databaseUrl": database_url })),
        Some(setup_token),
    );
    duplicate_token
        .headers_mut()
        .append("x-setup-token", setup_token.parse().expect("token header"));
    let response = app
        .clone()
        .oneshot(duplicate_token)
        .await
        .expect("duplicate token request should complete");
    assert_sensitive_cache_headers(&response);

    let mut malformed = Request::builder()
        .method(Method::POST)
        .uri("/api/v1/auth/login")
        .header(CONTENT_TYPE, "application/json")
        .body(Body::from("{"))
        .expect("request should build");
    malformed.extensions_mut().insert(ConnectInfo(
        "127.0.0.1:40000"
            .parse::<SocketAddr>()
            .expect("peer should parse"),
    ));
    let response = app
        .clone()
        .oneshot(malformed)
        .await
        .expect("malformed request should complete");
    assert_sensitive_cache_headers(&response);

    let response = app
        .clone()
        .oneshot(api_request(
            Method::POST,
            "/api/v1/setup/admin",
            Some(json!({
                "username": "Reader",
                "password": "correct horse battery staple"
            })),
            Some(setup_token),
        ))
        .await
        .expect("wrong-mode admin request should complete");
    assert_eq!(response.status(), StatusCode::CONFLICT);
    assert_sensitive_cache_headers(&response);

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
    assert_sensitive_cache_headers(&response);

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/api/v1/auth/login")
                .header(CONTENT_TYPE, "application/json")
                .header("x-forwarded-for", "203.0.113.55")
                .body(Body::from(
                    json!({
                        "login": "reader",
                        "password": "correct horse battery staple"
                    })
                    .to_string(),
                ))
                .expect("request should build"),
        )
        .await
        .expect("missing peer request should complete");
    assert_eq!(response.status(), StatusCode::OK);
    assert_sensitive_cache_headers(&response);

    let response = app
        .clone()
        .oneshot(api_request(
            Method::POST,
            "/api/v1/auth/login",
            Some(json!({
                "login": "reader",
                "password": "correct horse battery staple"
            })),
            None,
        ))
        .await
        .expect("login request should complete");
    assert_eq!(response.status(), StatusCode::OK);
    assert_sensitive_cache_headers(&response);
    let set_cookie = response.headers()[SET_COOKIE]
        .to_str()
        .expect("cookie should be text")
        .to_owned();
    let cookie = set_cookie
        .split(';')
        .next()
        .expect("cookie should contain a pair")
        .to_owned();
    let login_body = response_json(response).await;
    let csrf = login_body["csrfToken"]
        .as_str()
        .expect("CSRF token should be text");

    for request in [
        auth_request(Method::GET, "/api/v1/auth/session", &cookie, None),
        auth_request(Method::POST, "/api/v1/auth/logout", &cookie, None),
        auth_request(
            Method::POST,
            "/api/v1/auth/logout",
            &cookie,
            Some("AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"),
        ),
        auth_request(Method::POST, "/api/v1/auth/logout", &cookie, Some(csrf)),
    ] {
        let response = app
            .clone()
            .oneshot(request)
            .await
            .expect("authenticated request should complete");
        assert_sensitive_cache_headers(&response);
    }
}

fn api_request(
    method: Method,
    uri: &str,
    body: Option<Value>,
    setup_token: Option<&str>,
) -> Request<Body> {
    api_request_from(
        method,
        uri,
        body,
        setup_token,
        "127.0.0.1:40000".parse().expect("peer should parse"),
        None,
    )
}

fn api_request_from(
    method: Method,
    uri: &str,
    body: Option<Value>,
    setup_token: Option<&str>,
    peer: SocketAddr,
    forwarded_for: Option<&str>,
) -> Request<Body> {
    let mut request = Request::builder().method(method).uri(uri);
    if body.is_some() {
        request = request.header(CONTENT_TYPE, "application/json");
    }
    if let Some(setup_token) = setup_token {
        request = request.header("x-setup-token", setup_token);
    }
    if let Some(forwarded_for) = forwarded_for {
        request = request
            .header("x-forwarded-for", forwarded_for)
            .header("forwarded", format!("for={forwarded_for}"));
    }
    let mut request = request
        .body(body.map_or_else(Body::empty, |value| Body::from(value.to_string())))
        .expect("request should build");
    request.extensions_mut().insert(ConnectInfo(peer));
    request
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

fn error_chain(error: &dyn std::error::Error) -> String {
    let mut messages = vec![error.to_string()];
    let mut source = error.source();
    while let Some(error) = source {
        messages.push(error.to_string());
        source = error.source();
    }
    messages.join(": ")
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

fn assert_sensitive_cache_headers(response: &axum::response::Response) {
    assert_eq!(response.headers().get(CACHE_CONTROL).unwrap(), "no-store");
    assert_eq!(response.headers().get(PRAGMA).unwrap(), "no-cache");
}
