#[allow(dead_code)]
mod support;
use axum::{
    Router,
    body::Body,
    http::{Request, StatusCode},
};
use http_body_util::BodyExt;
use raindrop::{
    app::{AppState, build_router},
    auth::build_session_cookie,
    db::{DatabaseConfig, connect, connect_reader, migrate},
    setup::SetupService,
};
use sea_orm::{ConnectionTrait, DatabaseBackend, Statement};
use secrecy::{ExposeSecret, SecretString};
use support::database::{USER_A_ID, USER_B_ID, insert_user};
use tower::ServiceExt;

async fn request(
    app: &Router,
    method: &str,
    path: &str,
    cookie: &str,
    csrf: &str,
) -> (StatusCode, serde_json::Value) {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(method)
                .uri(path)
                .header("cookie", cookie)
                .header("x-csrf-token", csrf)
                .header("origin", "http://database.test")
                .header("host", "database.test")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.headers()["cache-control"], "no-store");
    let status = response.status();
    let json =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    (status, json)
}

#[tokio::test]
async fn maintenance_requires_admin_csrf_and_runs_in_background_without_blocking_reads() {
    let dir = tempfile::tempdir().unwrap();
    let config = DatabaseConfig::new(SecretString::from(format!(
        "sqlite://{}?mode=rwc",
        dir.path().join("api.db").display()
    )));
    let db = connect(&config).await.unwrap();
    migrate(&db).await.unwrap();
    insert_user(&db, USER_A_ID, "admin").await;
    insert_user(&db, USER_B_ID, "reader").await;
    db.execute(Statement::from_sql_and_values(
        DatabaseBackend::Sqlite,
        "INSERT INTO user_roles(user_id, role) VALUES (?, 'ADMIN')",
        [USER_A_ID.into()],
    ))
    .await
    .unwrap();
    let reader = connect_reader(&config, &db).await.unwrap();
    let setup = SetupService::ready_with_reader(dir.path(), None, db.clone(), reader);
    let admin = setup.sessions().create(USER_A_ID).await.unwrap();
    let user = setup.sessions().create(USER_B_ID).await.unwrap();
    let cookie = build_session_cookie(&admin, false)
        .to_string()
        .split(';')
        .next()
        .unwrap()
        .to_owned();
    let user_cookie = build_session_cookie(&user, false)
        .to_string()
        .split(';')
        .next()
        .unwrap()
        .to_owned();
    let csrf = admin.csrf_token.expose_secret();
    let app = build_router(AppState::new(setup));
    assert_eq!(
        request(&app, "GET", "/api/v1/database", "", "").await.0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        request(&app, "GET", "/api/v1/database", &user_cookie, "")
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        request(
            &app,
            "POST",
            "/api/v1/database/compact",
            &user_cookie,
            user.csrf_token.expose_secret()
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        request(&app, "POST", "/api/v1/database/compact", &cookie, "")
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        request(&app, "GET", "/api/v1/database", &cookie, "")
            .await
            .1["articleRetention"]["enabled"],
        false
    );
    for (body, expected) in [
        (
            r#"{"enabled":true,"retentionDays":0}"#,
            StatusCode::UNPROCESSABLE_ENTITY,
        ),
        (r#"{"enabled":true,"retentionDays":30}"#, StatusCode::OK),
        (r#"{"enabled":false,"retentionDays":30}"#, StatusCode::OK),
    ] {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri("/api/v1/database/article-retention")
                    .header("cookie", &cookie)
                    .header("x-csrf-token", csrf)
                    .header("content-type", "application/json")
                    .header("origin", "http://database.test")
                    .header("host", "database.test")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), expected);
    }
    // Occupy the writer before starting. POST and polling must still complete using reader pool.
    let writer = db.get_sqlite_connection_pool().acquire().await.unwrap();
    let accepted = tokio::time::timeout(
        std::time::Duration::from_secs(1),
        request(&app, "POST", "/api/v1/database/compact", &cookie, csrf),
    )
    .await
    .unwrap();
    assert_eq!(accepted.0, StatusCode::ACCEPTED);
    assert_eq!(accepted.1["running"], true);
    assert_eq!(
        request(&app, "GET", "/api/v1/database", &cookie, "")
            .await
            .1["running"],
        true
    );
    assert_eq!(
        request(&app, "POST", "/api/v1/database/compact", &cookie, csrf)
            .await
            .0,
        StatusCode::CONFLICT
    );
    drop(writer);
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let status = request(&app, "GET", "/api/v1/database", &cookie, "").await;
            if status.1["running"] == false {
                assert!(status.1["error"].is_null());
                assert!(status.1["result"].is_object());
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
}
