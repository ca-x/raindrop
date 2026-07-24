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
    http::{Method, Request, StatusCode, header::COOKIE},
};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use http_body_util::BodyExt;
use raindrop::{
    app::{AppState, build_router},
    auth::build_session_cookie,
    backups::{BackupError, BackupTransport, ExecutionTarget},
    content::provider::ProviderSecretKeyring,
    db::{DatabaseConfig, connect, migrate},
    setup::SetupService,
};
use secrecy::{ExposeSecret, SecretString};
use serde_json::{Value, json};
use support::database::{USER_A_ID, insert_user};
use tempfile::TempDir;
use tower::ServiceExt;

struct FakeBackupTransport {
    tests: Arc<AtomicUsize>,
}

#[async_trait]
impl BackupTransport for FakeBackupTransport {
    async fn test(&self, _target: &ExecutionTarget) -> Result<(), BackupError> {
        self.tests.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }

    async fn upload(&self, _target: &ExecutionTarget, _body: &[u8]) -> Result<(), BackupError> {
        Ok(())
    }
}

struct Fixture {
    _data: TempDir,
    app: Router,
    cookie: String,
    csrf: String,
    tests: Arc<AtomicUsize>,
}

impl Fixture {
    async fn new() -> Self {
        let data = tempfile::tempdir().expect("temporary directory");
        let database = connect(&DatabaseConfig::new(SecretString::from(format!(
            "sqlite://{}?mode=rwc",
            data.path().join("backup-api.db").display()
        ))))
        .await
        .expect("database connect");
        migrate(&database).await.expect("database migrate");
        insert_user(&database, USER_A_ID, "backup-api").await;
        let setup = SetupService::ready(data.path(), None, database);
        let session = setup
            .sessions()
            .create(USER_A_ID)
            .await
            .expect("session create");
        let tests = Arc::new(AtomicUsize::new(0));
        let state = AppState::new(setup)
            .with_provider_keyring(Some(Arc::new(keyring())))
            .with_backup_transport(Arc::new(FakeBackupTransport {
                tests: Arc::clone(&tests),
            }));
        Self {
            _data: data,
            app: build_router(state),
            cookie: build_session_cookie(&session, false)
                .to_string()
                .split(';')
                .next()
                .expect("cookie pair")
                .to_owned(),
            csrf: session.csrf_token.expose_secret().to_owned(),
            tests,
        }
    }

    async fn request(
        &self,
        method: Method,
        uri: &str,
        body: Option<Value>,
        csrf: bool,
    ) -> axum::response::Response {
        let mut request = Request::builder()
            .method(method)
            .uri(uri)
            .header(COOKIE, &self.cookie);
        if csrf {
            request = request.header("x-csrf-token", &self.csrf);
        }
        let body = if let Some(value) = body {
            request = request.header("content-type", "application/json");
            Body::from(value.to_string())
        } else {
            Body::empty()
        };
        self.app
            .clone()
            .oneshot(request.body(body).expect("request build"))
            .await
            .expect("request complete")
    }
}

#[tokio::test]
async fn backup_api_manages_multiple_targets_schedule_and_manual_job_without_disclosing_secrets() {
    let fixture = Fixture::new().await;
    let first = create_target(
        &fixture,
        json!({
            "displayName": "Primary S3",
            "enabled": true,
            "config": { "kind": "S3", "settings": {
                "endpoint": "https://objects.example", "region": "us-east-1",
                "bucket": "reader-backups", "prefix": "daily", "pathStyle": true
            }},
            "credentials": { "kind": "S3", "values": {
                "accessKeyId": "access-sentinel", "secretAccessKey": "secret-sentinel",
                "sessionToken": null
            }},
            "retention": { "retainCount": 7, "retainDays": 30 }
        }),
    )
    .await;
    let second = create_target(
        &fixture,
        json!({
            "displayName": "Archive S3",
            "enabled": true,
            "config": { "kind": "S3", "settings": {
                "endpoint": "https://archive.example", "region": "auto",
                "bucket": "rss-archive", "prefix": "", "pathStyle": false
            }},
            "credentials": { "kind": "S3", "values": {
                "accessKeyId": "archive-access", "secretAccessKey": "archive-secret",
                "sessionToken": null
            }},
            "retention": { "retainCount": null, "retainDays": 365 }
        }),
    )
    .await;
    let third = create_target(
        &fixture,
        json!({
            "displayName": "Home WebDAV",
            "enabled": true,
            "config": { "kind": "WEBDAV", "settings": {
                "endpoint": "https://dav.example/storage", "prefix": "rss"
            }},
            "credentials": { "kind": "WEBDAV", "values": {
                "username": "reader", "password": "password-sentinel"
            }},
            "retention": { "retainCount": 14, "retainDays": null }
        }),
    )
    .await;
    for value in [&first, &second, &third] {
        assert_backup_target_wire(value);
        assert_eq!(value["hasCredentials"], true);
        let serialized = value.to_string();
        assert!(!serialized.contains("sentinel"));
        assert!(value.get("secretConfigCiphertext").is_none());
    }

    let target_ids = vec![
        first["targetId"].as_str().unwrap(),
        second["targetId"].as_str().unwrap(),
        third["targetId"].as_str().unwrap(),
    ];
    let schedule = fixture
        .request(
            Method::PUT,
            "/api/v1/backups/schedule",
            Some(json!({ "enabled": true, "intervalHours": 12, "targetIds": target_ids })),
            true,
        )
        .await;
    assert_eq!(schedule.status(), StatusCode::OK);
    let schedule_body = response_json(schedule).await;
    assert!(schedule_body["nextRunAt"].is_string());

    let job = fixture
        .request(
            Method::POST,
            "/api/v1/backups/jobs",
            Some(json!({ "targetIds": target_ids })),
            true,
        )
        .await;
    assert_eq!(job.status(), StatusCode::ACCEPTED);
    let job_body = response_json(job).await;
    assert_eq!(job_body["targetCount"], 3);
    assert_eq!(job_body["targets"].as_array().unwrap().len(), 3);
    assert!(job_body["createdAt"].is_string());
    assert!(job_body["startedAt"].is_null());
    assert!(job_body["completedAt"].is_null());

    let test = fixture
        .request(
            Method::POST,
            &format!(
                "/api/v1/backups/targets/{}/test",
                first["targetId"].as_str().unwrap()
            ),
            None,
            true,
        )
        .await;
    assert_eq!(test.status(), StatusCode::OK);
    assert_eq!(fixture.tests.load(Ordering::SeqCst), 1);

    let list = fixture
        .request(Method::GET, "/api/v1/backups/targets", None, false)
        .await;
    assert_eq!(list.status(), StatusCode::OK);
    let list_body = response_json(list).await;
    let listed_targets = list_body["items"].as_array().unwrap();
    assert_eq!(listed_targets.len(), 3);
    for value in listed_targets {
        assert_backup_target_wire(value);
    }
}

#[tokio::test]
async fn backup_mutations_require_csrf() {
    let fixture = Fixture::new().await;
    let response = fixture
        .request(
            Method::POST,
            "/api/v1/backups/targets",
            Some(json!({})),
            false,
        )
        .await;
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

async fn create_target(fixture: &Fixture, body: Value) -> Value {
    let response = fixture
        .request(Method::POST, "/api/v1/backups/targets", Some(body), true)
        .await;
    assert_eq!(response.status(), StatusCode::CREATED);
    response_json(response).await
}

async fn response_json(response: axum::response::Response) -> Value {
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("body collect")
        .to_bytes();
    serde_json::from_slice(&bytes).expect("JSON response")
}

fn assert_backup_target_wire(value: &Value) {
    assert!(value["createdAt"].is_string());
    assert!(value["updatedAt"].is_string());
}

fn keyring() -> ProviderSecretKeyring {
    ProviderSecretKeyring::from_entries(&[SecretString::from(format!(
        "backup:{}",
        URL_SAFE_NO_PAD.encode([17_u8; 32])
    ))])
    .expect("backup keyring")
}
