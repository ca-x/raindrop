#[allow(dead_code)]
mod support;

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use axum::{
    Router,
    body::Body,
    http::{
        HeaderMap, Method, Request, StatusCode,
        header::{AUTHORIZATION, CACHE_CONTROL, CONTENT_TYPE, COOKIE, HOST, ORIGIN, PRAGMA},
    },
};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use http_body_util::BodyExt;
use raindrop::{
    app::{AppState, build_router},
    auth::build_session_cookie,
    content::provider::{
        EncodedProviderRequest, ProviderEndpoint, ProviderSecretKeyring, ProviderTransport,
        ProviderTransportError, ProviderTransportErrorKind, ProviderTransportResponse,
    },
    db::{
        DatabaseConfig, connect,
        entities::{entry, tts_config},
        migrate,
    },
    setup::SetupService,
};
use sea_orm::{ActiveModelTrait, ActiveValue::Set, DatabaseConnection, EntityTrait};
use secrecy::{ExposeSecret, SecretString};
use serde_json::{Value, json};
use support::database::{
    ENTRY_A_ID, ENTRY_B_ID, HASH_C, SUBSCRIPTION_A_ID, USER_A_ID, USER_B_ID, insert_entry,
    insert_feed, insert_subscription, insert_user,
};
use tempfile::TempDir;
use time::macros::datetime;
use tokio::sync::Semaphore;
use tower::ServiceExt;

const CONFIG: &str = "/api/v1/plugins/tts";
const PREVIEW: &str = "/api/v1/plugins/tts/test";
const SECRET: &str = "tts-secret-sentinel";
const MAX_AUDIO_BYTES: usize = 2 * 1024 * 1024;

#[derive(Clone)]
struct RecordedRequest {
    url: String,
    path: String,
    body: Value,
    authorization: Option<String>,
    authorization_is_secret: bool,
}

enum FakeReply {
    Audio,
    Response(StatusCode, Vec<u8>),
    Error(ProviderTransportErrorKind),
}

struct RecordingTransport {
    requests: Mutex<Vec<RecordedRequest>>,
    reply: Mutex<FakeReply>,
    gate: Option<Arc<Semaphore>>,
    entered: Semaphore,
}

impl RecordingTransport {
    fn new(gate: Option<Arc<Semaphore>>) -> Self {
        Self {
            requests: Mutex::new(Vec::new()),
            reply: Mutex::new(FakeReply::Audio),
            gate,
            entered: Semaphore::new(0),
        }
    }

    fn requests(&self) -> Vec<RecordedRequest> {
        self.requests.lock().expect("request lock").clone()
    }
}

#[async_trait]
impl ProviderTransport for RecordingTransport {
    async fn execute(
        &self,
        provider_id: &str,
        endpoint: &ProviderEndpoint,
        request: EncodedProviderRequest,
    ) -> Result<ProviderTransportResponse, ProviderTransportError> {
        let authorization = request
            .headers()
            .iter()
            .find(|header| header.name() == AUTHORIZATION);
        self.requests
            .lock()
            .expect("request lock")
            .push(RecordedRequest {
                url: endpoint
                    .join_adapter_path(request.path())
                    .expect("speech URL")
                    .to_string(),
                path: request.path().to_owned(),
                body: serde_json::from_slice(request.body()).expect("OpenAI JSON body"),
                authorization: authorization.map(|header| {
                    header
                        .secret_value()
                        .map(|value| value.expose_secret().to_owned())
                        .or_else(|| {
                            header
                                .public_value()
                                .map(|value| value.to_str().expect("header UTF-8").to_owned())
                        })
                        .expect("authorization header value")
                }),
                authorization_is_secret: authorization.is_some_and(|header| header.is_secret()),
            });
        self.entered.add_permits(1);
        if let Some(gate) = &self.gate {
            gate.acquire().await.expect("fake transport gate").forget();
        }
        match &*self.reply.lock().expect("reply lock") {
            FakeReply::Audio => Ok(ProviderTransportResponse::new(StatusCode::OK, mp3(), None)),
            FakeReply::Response(status, bytes) => {
                Ok(ProviderTransportResponse::new(*status, bytes.clone(), None))
            }
            FakeReply::Error(kind) => Err(ProviderTransportError::new(provider_id, *kind)),
        }
    }
}

struct Fixture {
    _data: TempDir,
    database: DatabaseConnection,
    app: Router,
    transport: Arc<RecordingTransport>,
    cookies: Vec<String>,
    csrf: Vec<String>,
}

impl Fixture {
    async fn new() -> Self {
        Self::with_options(true, None).await
    }

    async fn with_options(with_keyring: bool, gate: Option<Arc<Semaphore>>) -> Self {
        let data = tempfile::tempdir().expect("TTS temporary directory");
        let url = format!(
            "sqlite://{}?mode=rwc",
            data.path().join("tts-api.db").display()
        );
        let database = connect(&DatabaseConfig::new(SecretString::from(url)))
            .await
            .expect("TTS database");
        migrate(&database).await.expect("TTS migration");
        insert_user(&database, USER_A_ID, "tts-a").await;
        insert_user(&database, USER_B_ID, "tts-b").await;
        let at = datetime!(2026-11-17 12:00:00 UTC);
        insert_feed(&database, at).await;
        insert_subscription(&database, SUBSCRIPTION_A_ID, USER_A_ID, at).await;
        insert_entry(&database, ENTRY_A_ID, 2, "tts-entry", HASH_C, None, at).await;
        let setup = SetupService::ready(data.path(), None, database.clone());
        let a = setup.sessions().create(USER_A_ID).await.expect("A session");
        let b = setup.sessions().create(USER_B_ID).await.expect("B session");
        let mut sessions = vec![a, b];
        for (id, username) in [
            ("00000000-0000-4000-8000-000000000003", "tts-c"),
            ("00000000-0000-4000-8000-000000000004", "tts-d"),
            ("00000000-0000-4000-8000-000000000005", "tts-e"),
        ] {
            insert_user(&database, id, username).await;
            sessions.push(setup.sessions().create(id).await.expect("extra session"));
        }
        let cookies = sessions
            .iter()
            .map(|session| {
                build_session_cookie(session, false)
                    .to_string()
                    .split(';')
                    .next()
                    .expect("cookie pair")
                    .to_owned()
            })
            .collect();
        let csrf = sessions
            .iter()
            .map(|session| session.csrf_token.expose_secret().to_owned())
            .collect();
        let keyring = with_keyring.then(|| Arc::new(keyring()));
        let transport = Arc::new(RecordingTransport::new(gate));
        let state = AppState::new(setup)
            .with_provider_keyring(keyring)
            .with_tts_transport(transport.clone());
        Self {
            _data: data,
            database,
            app: build_router(state),
            transport,
            cookies,
            csrf,
        }
    }

    async fn request(
        &self,
        method: Method,
        uri: &str,
        body: Option<Value>,
        user: Option<usize>,
        csrf: bool,
    ) -> CapturedResponse {
        let mut request = Request::builder().method(method).uri(uri);
        if let Some(user) = user {
            request = request.header(COOKIE, &self.cookies[user]);
            if csrf {
                request = request
                    .header("x-csrf-token", &self.csrf[user])
                    .header(ORIGIN, "http://tts.test")
                    .header(HOST, "tts.test");
            }
        }
        let body = match body {
            Some(body) => {
                request = request.header(CONTENT_TYPE, "application/json");
                Body::from(body.to_string())
            }
            None => Body::empty(),
        };
        let response = self
            .app
            .clone()
            .oneshot(request.body(body).expect("TTS request"))
            .await
            .expect("TTS response");
        let (parts, body) = response.into_parts();
        CapturedResponse {
            status: parts.status,
            headers: parts.headers,
            body: body
                .collect()
                .await
                .expect("response bytes")
                .to_bytes()
                .to_vec(),
        }
    }

    async fn get(&self, user: usize) -> CapturedResponse {
        self.request(Method::GET, CONFIG, None, Some(user), false)
            .await
    }

    async fn save(&self, user: usize, body: Value) -> CapturedResponse {
        self.request(Method::PUT, CONFIG, Some(body), Some(user), true)
            .await
    }

    async fn preview(&self, user: usize, body: Value) -> CapturedResponse {
        self.request(Method::POST, PREVIEW, Some(body), Some(user), true)
            .await
    }

    async fn plan(&self, user: usize, id: &str) -> CapturedResponse {
        self.request(
            Method::POST,
            &entry_url(id, "plan"),
            Some(json!({})),
            Some(user),
            true,
        )
        .await
    }

    async fn speak(&self, user: usize, id: &str, hash: &Value, index: Value) -> CapturedResponse {
        self.request(
            Method::POST,
            &entry_url(id, "speak"),
            Some(json!({"contentHash": hash, "chunkIndex": index})),
            Some(user),
            true,
        )
        .await
    }

    async fn stored(&self, user: &str) -> tts_config::Model {
        tts_config::Entity::find_by_id(user)
            .one(&self.database)
            .await
            .expect("config query")
            .expect("stored config")
    }
}

struct CapturedResponse {
    status: StatusCode,
    headers: HeaderMap,
    body: Vec<u8>,
}

impl CapturedResponse {
    fn json(&self) -> Value {
        serde_json::from_slice(&self.body).unwrap_or_else(|_| {
            panic!(
                "expected JSON, status {}, body {:?}",
                self.status,
                String::from_utf8_lossy(&self.body)
            )
        })
    }

    fn ok(&self) {
        assert_eq!(
            self.status,
            StatusCode::OK,
            "{}",
            String::from_utf8_lossy(&self.body)
        );
    }

    fn error(&self, status: StatusCode, code: &str) {
        assert_eq!(
            self.status,
            status,
            "{}",
            String::from_utf8_lossy(&self.body)
        );
        assert_eq!(self.json()["error"]["code"], code);
        self.no_store();
    }

    fn no_store(&self) {
        assert_eq!(self.headers[CACHE_CONTROL], "no-store");
        assert_eq!(self.headers[PRAGMA], "no-cache");
    }

    fn audio(&self) {
        self.ok();
        assert_eq!(self.headers[CONTENT_TYPE], "audio/mpeg");
        assert_eq!(self.headers["x-content-type-options"], "nosniff");
        assert_eq!(self.body, mp3());
        self.no_store();
    }
}

fn keyring() -> ProviderSecretKeyring {
    ProviderSecretKeyring::from_entries(&[SecretString::from(format!(
        "primary:{}",
        URL_SAFE_NO_PAD.encode([0x72_u8; 32])
    ))])
    .expect("TTS keyring")
}

fn config(revision: i64) -> Value {
    json!({"isEnabled": true, "baseUrl": "https://speech.example/v1", "model": "tts-1", "voice": "zh-CN-YunyangNeural", "speed": 1.25, "revision": revision})
}

fn draft() -> Value {
    let mut body = config(0);
    body["isEnabled"] = json!(false);
    body["text"] = json!("你好 & hello");
    body
}

fn entry_url(id: &str, action: &str) -> String {
    format!("/api/v1/plugins/tts/entries/{id}/{action}")
}

fn mp3() -> Vec<u8> {
    let mut bytes = vec![0xff, 0xfb, 0x90, 0x00];
    bytes.extend([0; 32]);
    bytes
}

#[tokio::test]
async fn defaults_and_all_routes_require_authentication_and_mutations_require_csrf() {
    let fixture = Fixture::new().await;
    let initial = fixture.get(0).await;
    initial.ok();
    initial.no_store();
    assert_eq!(
        initial.json(),
        json!({"isEnabled": false, "baseUrl": "https://api.openai.com/v1", "model": "tts-1", "voice": "alloy", "speed": 1.0, "hasApiKey": false, "keyringAvailable": true, "revision": 0})
    );
    for (method, uri, body) in [
        (Method::GET, CONFIG.to_owned(), None),
        (Method::PUT, CONFIG.to_owned(), Some(config(0))),
        (Method::POST, PREVIEW.to_owned(), Some(draft())),
        (Method::POST, entry_url(ENTRY_A_ID, "plan"), Some(json!({}))),
        (
            Method::POST,
            entry_url(ENTRY_A_ID, "speak"),
            Some(json!({"contentHash": "a".repeat(64), "chunkIndex": 0})),
        ),
    ] {
        fixture
            .request(method.clone(), &uri, body.clone(), None, false)
            .await
            .error(StatusCode::UNAUTHORIZED, "AUTHENTICATION_REQUIRED");
        if method != Method::GET {
            fixture
                .request(method, &uri, body, Some(0), false)
                .await
                .error(StatusCode::FORBIDDEN, "FORBIDDEN");
        }
    }
    assert!(fixture.transport.requests().is_empty());
}

#[tokio::test]
async fn saved_credentials_are_encrypted_isolated_and_never_returned() {
    let fixture = Fixture::new().await;
    let mut body = config(0);
    body["apiKey"] = json!(SECRET);
    let saved = fixture.save(0, body).await;
    saved.ok();
    assert_eq!(saved.json()["revision"], 1);
    assert_eq!(saved.json()["hasApiKey"], true);
    let own = fixture.get(0).await;
    own.ok();
    assert_eq!(own.json()["voice"], "zh-CN-YunyangNeural");
    for response in [&saved, &own] {
        assert!(response.json().get("apiKey").is_none());
        assert!(!String::from_utf8_lossy(&response.body).contains(SECRET));
    }
    let stored = fixture.stored(USER_A_ID).await;
    let encrypted = stored.encrypted_api_key.as_ref().expect("encrypted key");
    assert!(!encrypted.contains(SECRET));
    assert!(!stored.config_json.contains(SECRET));
    let other = fixture.get(1).await;
    other.ok();
    assert_eq!(other.json()["hasApiKey"], false);
    assert_eq!(other.json()["revision"], 0);
    fixture.preview(0, draft()).await.audio();
    fixture.preview(1, draft()).await.audio();
    let requests = fixture.transport.requests();
    assert_eq!(
        requests[0].authorization.as_deref(),
        Some("Bearer tts-secret-sentinel")
    );
    assert!(requests[0].authorization_is_secret);
    assert!(requests[1].authorization.is_none());
    fixture.save(1, config(0)).await.ok();
    tts_config::ActiveModel {
        user_id: Set(USER_B_ID.to_owned()),
        encrypted_api_key: Set(Some(encrypted.clone())),
        ..Default::default()
    }
    .update(&fixture.database)
    .await
    .expect("copy ciphertext to other user");
    fixture
        .preview(1, draft())
        .await
        .error(StatusCode::SERVICE_UNAVAILABLE, "TTS_KEYRING_UNAVAILABLE");
    assert_eq!(
        fixture.transport.requests().len(),
        2,
        "copied ciphertext must not reach upstream"
    );
}

#[tokio::test]
async fn revision_cas_preserves_replaces_and_clears_keys_without_stale_credential_writes() {
    let fixture = Fixture::new().await;
    let mut first = config(0);
    first["apiKey"] = json!(SECRET);
    fixture.save(0, first).await.ok();
    let ciphertext = fixture.stored(USER_A_ID).await.encrypted_api_key;
    fixture.save(0, config(1)).await.ok();
    assert_eq!(
        fixture.stored(USER_A_ID).await.encrypted_api_key,
        ciphertext
    );
    for key in [Value::Null, json!("stale-secret")] {
        let mut stale = config(1);
        stale["apiKey"] = key;
        fixture
            .save(0, stale)
            .await
            .error(StatusCode::CONFLICT, "TTS_CONFLICT");
        assert_eq!(
            fixture.stored(USER_A_ID).await.encrypted_api_key,
            ciphertext
        );
        assert_eq!(fixture.get(0).await.json()["revision"], 2);
    }
    let mut replace = config(2);
    replace["apiKey"] = json!("replacement-secret");
    fixture.save(0, replace).await.ok();
    fixture.preview(0, draft()).await.audio();
    assert_eq!(
        fixture.transport.requests()[0].authorization.as_deref(),
        Some("Bearer replacement-secret")
    );
    let mut clear = config(3);
    clear["apiKey"] = Value::Null;
    let cleared = fixture.save(0, clear).await;
    cleared.ok();
    assert_eq!(cleared.json()["hasApiKey"], false);
    assert_eq!(cleared.json()["revision"], 4);
    assert!(fixture.stored(USER_A_ID).await.encrypted_api_key.is_none());
    fixture.preview(0, draft()).await.audio();
    assert!(fixture.transport.requests()[1].authorization.is_none());
}

#[tokio::test]
async fn concurrent_settings_writers_cannot_overwrite_the_winning_credential() {
    let fixture = Fixture::new().await;
    fixture.save(0, config(0)).await.ok();
    let mut a = config(1);
    a["apiKey"] = json!("writer-a");
    let mut b = config(1);
    b["apiKey"] = json!("writer-b");
    let (a, b) = tokio::join!(fixture.save(0, a), fixture.save(0, b));
    let winner = match (a.status, b.status) {
        (StatusCode::OK, StatusCode::CONFLICT) => "Bearer writer-a",
        (StatusCode::CONFLICT, StatusCode::OK) => "Bearer writer-b",
        statuses => panic!(
            "exactly one CAS winner expected: {statuses:?}; A={}, B={}",
            a.json(),
            b.json()
        ),
    };
    assert_eq!(fixture.get(0).await.json()["revision"], 2);
    fixture.preview(0, draft()).await.audio();
    assert_eq!(
        fixture.transport.requests()[0].authorization.as_deref(),
        Some(winner)
    );
}

#[tokio::test]
async fn preview_uses_unsaved_disabled_drafts_and_exact_openai_body_with_both_api_root_forms() {
    let fixture = Fixture::new().await;
    for root in [
        "https://speech.example",
        "https://speech.example/v1",
        "https://speech.example/v1/",
    ] {
        let mut body = draft();
        body.as_object_mut().unwrap().remove("revision");
        body["baseUrl"] = json!(root);
        body["apiKey"] = json!("preview-only-key");
        fixture.preview(0, body).await.audio();
    }
    for request in fixture.transport.requests() {
        assert_eq!(request.url, "https://speech.example/v1/audio/speech");
        assert_eq!(request.path, "/audio/speech");
        assert_eq!(
            request.body,
            json!({"input": "你好 & hello", "model": "tts-1", "voice": "zh-CN-YunyangNeural", "speed": 1.25, "response_format": "mp3"})
        );
        assert_eq!(
            request.authorization.as_deref(),
            Some("Bearer preview-only-key")
        );
    }
    let unchanged = fixture.get(0).await;
    assert_eq!(unchanged.json()["revision"], 0);
    assert_eq!(unchanged.json()["hasApiKey"], false);
    assert_eq!(unchanged.json()["isEnabled"], false);
}

#[tokio::test]
async fn keyless_configuration_works_without_keyring_but_persisting_a_key_fails() {
    let fixture = Fixture::with_options(false, None).await;
    assert_eq!(fixture.get(0).await.json()["keyringAvailable"], false);
    fixture.save(0, config(0)).await.ok();
    fixture.preview(0, draft()).await.audio();
    let mut keyed = config(1);
    keyed["apiKey"] = json!(SECRET);
    fixture
        .save(0, keyed)
        .await
        .error(StatusCode::SERVICE_UNAVAILABLE, "TTS_KEYRING_UNAVAILABLE");
    assert_eq!(fixture.get(0).await.json()["revision"], 1);
    assert_eq!(fixture.get(0).await.json()["hasApiKey"], false);
}

#[tokio::test]
async fn unsafe_or_malformed_endpoints_are_rejected_before_transport_or_storage() {
    let fixture = Fixture::new().await;
    for endpoint in [
        "http://speech.example/v1",
        "not a URL",
        "https://localhost/v1",
        "https://127.0.0.1/v1",
        "https://10.0.0.1/v1",
        "https://169.254.169.254/v1",
        "https://[::1]/v1",
        "https://[fc00::1]/v1",
        "https://user:password@public.example/v1",
        "https://speech.example/v1?key=secret",
        "https://speech.example/v1#fragment",
        "https://speech.example/v1/audio/speech",
    ] {
        let mut body = config(0);
        body["baseUrl"] = json!(endpoint);
        fixture
            .save(0, body.clone())
            .await
            .error(StatusCode::BAD_REQUEST, "TTS_INVALID_INPUT");
        body["text"] = json!("preview");
        fixture
            .preview(0, body)
            .await
            .error(StatusCode::BAD_REQUEST, "TTS_INVALID_INPUT");
    }
    assert_eq!(fixture.get(0).await.json()["revision"], 0);
    assert!(fixture.transport.requests().is_empty());
}

#[tokio::test]
async fn strict_json_and_semantic_validation_reject_untrusted_drafts() {
    let fixture = Fixture::new().await;
    for field in ["unexpected", "hasApiKey", "keyringAvailable"] {
        let mut body = config(0);
        body[field] = json!(true);
        fixture
            .save(0, body.clone())
            .await
            .error(StatusCode::UNPROCESSABLE_ENTITY, "VALIDATION_ERROR");
        body["text"] = json!("preview");
        fixture
            .preview(0, body)
            .await
            .error(StatusCode::UNPROCESSABLE_ENTITY, "VALIDATION_ERROR");
    }
    for (field, value) in [
        ("speed", json!(0.24)),
        ("speed", json!(4.01)),
        ("voice", json!("")),
        ("model", json!("bad\nmodel")),
        ("apiKey", json!("bad\nkey")),
        ("revision", json!(-1)),
    ] {
        let mut body = config(0);
        body[field] = value;
        fixture
            .save(0, body)
            .await
            .error(StatusCode::BAD_REQUEST, "TTS_INVALID_INPUT");
    }
    for text in ["".to_owned(), " \n ".to_owned(), "好".repeat(201)] {
        let mut body = draft();
        body["text"] = json!(text);
        fixture
            .preview(0, body)
            .await
            .error(StatusCode::BAD_REQUEST, "TTS_INVALID_INPUT");
    }
    let mut body = draft();
    body["text"] = json!("好".repeat(200));
    fixture.preview(0, body).await.audio();
    assert_eq!(fixture.transport.requests().len(), 1);
}

#[tokio::test]
async fn owned_article_plan_drives_synthesis_and_other_users_or_missing_entries_cannot_play() {
    let fixture = Fixture::new().await;
    fixture.save(0, config(0)).await.ok();
    fixture.save(1, config(0)).await.ok();
    let plan = fixture.plan(0, ENTRY_A_ID).await;
    plan.ok();
    plan.no_store();
    let plan = plan.json();
    assert_eq!(plan["entryId"], ENTRY_A_ID);
    assert_eq!(
        plan["contentHash"].as_str().expect("snapshot hash").len(),
        64
    );
    let chunks = plan["chunks"].as_array().expect("chunks");
    assert!(!chunks.is_empty());
    let text = chunks
        .iter()
        .map(|value| value.as_str().expect("text chunk"))
        .collect::<String>();
    assert!(text.contains("Safe content"));
    assert!(!text.contains("<p>"));
    assert!(fixture.transport.requests().is_empty());
    fixture
        .speak(0, ENTRY_A_ID, &plan["contentHash"], json!(0))
        .await
        .audio();
    assert_eq!(fixture.transport.requests()[0].body["input"], chunks[0]);
    assert_eq!(
        fixture.transport.requests()[0].body["voice"],
        "zh-CN-YunyangNeural"
    );
    for (user, id) in [(1, ENTRY_A_ID), (0, ENTRY_B_ID)] {
        fixture
            .plan(user, id)
            .await
            .error(StatusCode::NOT_FOUND, "TTS_NOT_FOUND");
        fixture
            .speak(user, id, &plan["contentHash"], json!(0))
            .await
            .error(StatusCode::NOT_FOUND, "TTS_NOT_FOUND");
    }
    assert_eq!(fixture.transport.requests().len(), 1);
}

#[tokio::test]
async fn speak_rejects_disabled_settings_forged_bodies_and_invalid_chunk_indexes() {
    let fixture = Fixture::new().await;
    fixture
        .plan(0, ENTRY_A_ID)
        .await
        .error(StatusCode::CONFLICT, "TTS_DISABLED");
    fixture
        .speak(0, ENTRY_A_ID, &json!("a".repeat(64)), json!(0))
        .await
        .error(StatusCode::CONFLICT, "TTS_DISABLED");
    fixture.save(0, config(0)).await.ok();
    let plan = fixture.plan(0, ENTRY_A_ID).await.json();
    fixture
        .speak(0, ENTRY_A_ID, &plan["contentHash"], json!(-1))
        .await
        .error(StatusCode::UNPROCESSABLE_ENTITY, "VALIDATION_ERROR");
    fixture
        .speak(
            0,
            ENTRY_A_ID,
            &plan["contentHash"],
            json!(plan["chunks"].as_array().unwrap().len()),
        )
        .await
        .error(StatusCode::BAD_REQUEST, "TTS_INVALID_INPUT");
    fixture
        .speak(0, ENTRY_A_ID, &json!("invalid-hash"), json!(0))
        .await
        .error(StatusCode::BAD_REQUEST, "TTS_INVALID_INPUT");
    for body in [
        json!({}),
        json!({"contentHash": plan["contentHash"], "chunkIndex": 0, "text": "attacker supplied text"}),
    ] {
        fixture
            .request(
                Method::POST,
                &entry_url(ENTRY_A_ID, "speak"),
                Some(body),
                Some(0),
                true,
            )
            .await
            .error(StatusCode::UNPROCESSABLE_ENTITY, "VALIDATION_ERROR");
    }
    assert!(fixture.transport.requests().is_empty());
}

#[tokio::test]
async fn article_edits_settings_changes_and_key_rotation_invalidate_planned_snapshots() {
    let fixture = Fixture::new().await;
    fixture.save(0, config(0)).await.ok();
    let first = fixture.plan(0, ENTRY_A_ID).await.json();
    entry::ActiveModel {
        id: Set(ENTRY_A_ID.to_owned()),
        sanitized_content: Set(
            "rdsc:v1:{\"html\":\"<p>Changed article</p>\",\"inertImages\":[]}".to_owned(),
        ),
        ..Default::default()
    }
    .update(&fixture.database)
    .await
    .expect("update article");
    fixture
        .speak(0, ENTRY_A_ID, &first["contentHash"], json!(0))
        .await
        .error(StatusCode::CONFLICT, "TTS_CONFLICT");
    let second = fixture.plan(0, ENTRY_A_ID).await.json();
    assert_ne!(first["contentHash"], second["contentHash"]);
    let mut changed = config(1);
    changed["voice"] = json!("alloy");
    fixture.save(0, changed).await.ok();
    fixture
        .speak(0, ENTRY_A_ID, &second["contentHash"], json!(0))
        .await
        .error(StatusCode::CONFLICT, "TTS_CONFLICT");
    let third = fixture.plan(0, ENTRY_A_ID).await.json();
    let mut rotated = config(2);
    rotated["voice"] = json!("alloy");
    rotated["apiKey"] = json!(SECRET);
    fixture.save(0, rotated).await.ok();
    fixture
        .speak(0, ENTRY_A_ID, &third["contentHash"], json!(0))
        .await
        .error(StatusCode::CONFLICT, "TTS_CONFLICT");
    assert!(fixture.transport.requests().is_empty());
}

#[tokio::test]
async fn invalid_and_oversized_upstream_audio_is_redacted_and_never_retried() {
    let fixture = Fixture::new().await;
    let mut oversized = mp3();
    oversized.resize(MAX_AUDIO_BYTES + 1, 0);
    for bytes in [
        Vec::new(),
        b"<html>tts-secret-sentinel upstream-body-sentinel</html>".to_vec(),
        b"{\"error\":\"upstream-body-sentinel\"}".to_vec(),
        oversized,
    ] {
        *fixture.transport.reply.lock().expect("reply lock") =
            FakeReply::Response(StatusCode::OK, bytes);
        let response = fixture.preview(0, draft()).await;
        response.error(StatusCode::BAD_GATEWAY, "TTS_UPSTREAM_INVALID");
        assert!(!String::from_utf8_lossy(&response.body).contains(SECRET));
        assert!(!String::from_utf8_lossy(&response.body).contains("upstream-body-sentinel"));
    }
    assert_eq!(fixture.transport.requests().len(), 4);
}

#[tokio::test]
async fn provider_status_and_transport_failures_have_generic_errors_and_no_retries() {
    let fixture = Fixture::new().await;
    for (status, expected, code) in [
        (
            StatusCode::UNAUTHORIZED,
            StatusCode::BAD_GATEWAY,
            "TTS_UPSTREAM_AUTH",
        ),
        (
            StatusCode::FORBIDDEN,
            StatusCode::BAD_GATEWAY,
            "TTS_UPSTREAM_AUTH",
        ),
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            StatusCode::BAD_GATEWAY,
            "TTS_UPSTREAM",
        ),
        (StatusCode::FOUND, StatusCode::BAD_GATEWAY, "TTS_UPSTREAM"),
        (
            StatusCode::TOO_MANY_REQUESTS,
            StatusCode::TOO_MANY_REQUESTS,
            "TTS_RATE_LIMITED",
        ),
    ] {
        *fixture.transport.reply.lock().expect("reply lock") = FakeReply::Response(
            status,
            format!("{SECRET} upstream-body-sentinel").into_bytes(),
        );
        let response = fixture.preview(0, draft()).await;
        response.error(expected, code);
        let message = String::from_utf8_lossy(&response.body);
        assert!(!message.contains(SECRET));
        assert!(!message.contains("upstream-body-sentinel"));
        assert!(!message.contains("speech.example"));
    }
    for (kind, expected, code) in [
        (
            ProviderTransportErrorKind::Timeout,
            StatusCode::GATEWAY_TIMEOUT,
            "TTS_TIMEOUT",
        ),
        (
            ProviderTransportErrorKind::ResponseTooLarge,
            StatusCode::BAD_GATEWAY,
            "TTS_UPSTREAM_INVALID",
        ),
        (
            ProviderTransportErrorKind::Decode,
            StatusCode::BAD_GATEWAY,
            "TTS_UPSTREAM_INVALID",
        ),
        (
            ProviderTransportErrorKind::RedirectDenied,
            StatusCode::BAD_GATEWAY,
            "TTS_UPSTREAM",
        ),
    ] {
        *fixture.transport.reply.lock().expect("reply lock") = FakeReply::Error(kind);
        fixture.preview(0, draft()).await.error(expected, code);
    }
    assert_eq!(fixture.transport.requests().len(), 9);
}

#[tokio::test]
async fn user_concurrency_is_bounded_before_transport_and_permits_are_released() {
    let gate = Arc::new(Semaphore::new(0));
    let fixture = Fixture::with_options(true, Some(gate.clone())).await;
    let first = fixture.preview(0, draft());
    let competing = async {
        let entered = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            fixture.transport.entered.acquire(),
        )
        .await
        .expect("first request must reach transport")
        .expect("entered semaphore");
        entered.forget();
        fixture
            .preview(0, draft())
            .await
            .error(StatusCode::TOO_MANY_REQUESTS, "TTS_RATE_LIMITED");
        assert_eq!(fixture.transport.requests().len(), 1);
        gate.add_permits(1);
    };
    let (first, ()) = tokio::join!(first, competing);
    first.audio();
    gate.add_permits(1);
    fixture.preview(0, draft()).await.audio();
    assert_eq!(fixture.transport.requests().len(), 2);
}

#[tokio::test]
async fn global_concurrency_limits_distinct_users_to_four_upstream_requests() {
    let gate = Arc::new(Semaphore::new(0));
    let fixture = Fixture::with_options(true, Some(gate.clone())).await;
    let admitted =
        futures_util::future::join_all((0..4).map(|user| fixture.preview(user, draft())));
    let competing = async {
        tokio::time::timeout(
            std::time::Duration::from_secs(5),
            fixture.transport.entered.acquire_many(4),
        )
        .await
        .expect("four requests must reach transport")
        .expect("entered semaphore")
        .forget();
        fixture
            .preview(4, draft())
            .await
            .error(StatusCode::TOO_MANY_REQUESTS, "TTS_RATE_LIMITED");
        assert_eq!(fixture.transport.requests().len(), 4);
        gate.add_permits(4);
    };
    let (responses, ()) = tokio::join!(admitted, competing);
    for response in responses {
        response.audio();
    }
    gate.add_permits(1);
    fixture.preview(4, draft()).await.audio();
    assert_eq!(fixture.transport.requests().len(), 5);
}

#[tokio::test]
async fn execution_rate_budget_is_per_user_and_independent_of_config_mutations() {
    let fixture = Fixture::new().await;
    for _ in 0..512 {
        fixture.preview(0, draft()).await.audio();
    }
    fixture
        .preview(0, draft())
        .await
        .error(StatusCode::TOO_MANY_REQUESTS, "TTS_RATE_LIMITED");
    assert_eq!(fixture.transport.requests().len(), 512);
    fixture.preview(1, draft()).await.audio();
    fixture.save(0, config(0)).await.ok();
    fixture.get(0).await.ok();
    assert_eq!(fixture.transport.requests().len(), 513);
}
