use super::*;
use axum::{
    Router,
    body::Bytes,
    extract::State,
    http::{HeaderMap, Method, StatusCode, Uri},
    response::IntoResponse,
};
use std::sync::Mutex;
use zeroize::Zeroizing;

#[derive(Default)]
struct Mock {
    subscriptions: Vec<Subscription>,
    categories: Vec<serde_json::Value>,
    posts: usize,
    lose_ack: bool,
    repeat_cursor: bool,
    unique_cursors: bool,
    cursor_bytes: usize,
    page_padding: usize,
    subscription_pages: usize,
}

struct Server {
    address: String,
    state: Arc<Mutex<Mock>>,
    shutdown: Option<tokio::sync::oneshot::Sender<()>>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Server {
    fn new() -> Self {
        let state = Arc::new(Mutex::new(Mock::default()));
        let (sender, receiver) = std::sync::mpsc::sync_channel(1);
        let (shutdown, stopped) = tokio::sync::oneshot::channel();
        let mock = state.clone();
        let thread = std::thread::spawn(move || {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap()
                .block_on(async {
                    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
                    sender
                        .send(format!("http://{}/", listener.local_addr().unwrap()))
                        .unwrap();
                    axum::serve(listener, Router::new().fallback(dispatch).with_state(mock))
                        .with_graceful_shutdown(async {
                            let _ = stopped.await;
                        })
                        .await
                        .unwrap();
                });
        });
        Self {
            address: receiver.recv().unwrap(),
            state,
            shutdown: Some(shutdown),
            thread: Some(thread),
        }
    }

    fn login(&self) -> Api {
        Api::login(&self.address, "reader", Zeroizing::new("password".into()))
            .unwrap()
            .0
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
        if let Some(thread) = self.thread.take() {
            thread.join().unwrap();
        }
    }
}

async fn dispatch(
    State(state): State<Arc<Mutex<Mock>>>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
) -> axum::response::Response {
    let mut state = state.lock().unwrap();
    let path = uri.path();
    if path == "/api/v1/auth/login" {
        return ([("set-cookie", "session=fixture; HttpOnly; Path=/")], axum::Json(serde_json::json!({"csrfToken":"fixture", "user":{"id":"account-1", "username":"reader"}}))).into_response();
    }
    assert_eq!(headers.get("cookie").unwrap(), "session=fixture");
    if method != Method::GET {
        assert_eq!(headers.get("x-csrf-token").unwrap(), "fixture");
    }
    let value = match (method, path) {
        (Method::GET, "/api/v1/subscriptions") => {
            state.subscription_pages += 1;
            let cursor = if state.repeat_cursor {
                Some("again".to_owned())
            } else if state.cursor_bytes > 0 {
                Some("x".repeat(state.cursor_bytes))
            } else if state.unique_cursors {
                Some(format!("page-{}", state.subscription_pages))
            } else {
                None
            };
            serde_json::json!({"items":state.subscriptions, "nextCursor":cursor, "padding":"x".repeat(state.page_padding)})
        }
        (Method::GET, "/api/v1/categories") => serde_json::json!({"items":state.categories}),
        (Method::GET, "/api/v1/ai/providers") => serde_json::json!({"items":[]}),
        (Method::GET, "/api/v1/ai/config") => {
            serde_json::json!({"pluginState":"DISABLED","config":null})
        }
        (Method::POST, "/api/v1/subscriptions") => {
            let input: serde_json::Value = serde_json::from_slice(&body).unwrap();
            assert_eq!(input.as_object().unwrap().len(), 1);
            state.posts += 1;
            let subscription = sample("pushed", input["url"].as_str().unwrap());
            state.subscriptions.push(subscription.clone());
            if state.lose_ack {
                return StatusCode::INTERNAL_SERVER_ERROR.into_response();
            }
            serde_json::json!({"created":true, "subscription":subscription})
        }
        _ => return StatusCode::NOT_FOUND.into_response(),
    };
    axum::Json(value).into_response()
}

fn sample(id: &str, url: &str) -> Subscription {
    Subscription {
        subscription_id: id.into(),
        feed_id: format!("feed-{id}"),
        title: "Remote feed".into(),
        feed_url: url.into(),
        ..Default::default()
    }
}

fn local() -> (tempfile::TempDir, Api) {
    let directory = tempfile::tempdir().unwrap();
    let api = Api::isolated(directory.path().to_path_buf()).unwrap();
    (directory, api)
}

#[test]
fn imports_are_deduplicated_and_preserve_local_metadata() {
    let (_directory, api) = local();
    let server = Server::new();
    let remote = server.login();
    let subscription = api.subscribe("https://example.com/feed.xml").unwrap();
    api.write::<serde_json::Value>(
        Method::PATCH,
        &format!("api/v1/subscriptions/{}", subscription.subscription_id),
        &serde_json::json!({"titleOverride":"My title"}),
    )
    .unwrap();
    server
        .state
        .lock()
        .unwrap()
        .subscriptions
        .push(sample("remote-1", &subscription.feed_url));
    let profile = connect(&api, &remote, "reader".into()).unwrap();
    assert_eq!(import(&api, &remote, &profile.id).unwrap().linked, 1);
    assert_eq!(import(&api, &remote, &profile.id).unwrap().added, 0);
    let subscriptions = api.subscriptions().unwrap();
    assert_eq!(subscriptions.len(), 1);
    assert_eq!(subscriptions[0].title_override.as_deref(), Some("My title"));
    let journal = profiles(&api).unwrap();
    assert_eq!(journal.links.len(), 1);
    let encoded = serde_json::to_string(&journal).unwrap();
    assert!(!encoded.contains("password"));
    assert!(!encoded.contains("csrfToken"));
    let mut other_account = remote.clone();
    other_account.user_id = "account-2".into();
    assert_eq!(
        import(&api, &other_account, &profile.id).unwrap_err(),
        ApiError::Unauthorized
    );
    let other_profile = connect(&api, &other_account, "reader".into()).unwrap();
    assert_ne!(profile.id, other_profile.id);
}

#[test]
fn uncertain_push_never_recreates_deleted_remote_subscription() {
    let (_directory, api) = local();
    let server = Server::new();
    let remote = server.login();
    let profile = connect(&api, &remote, "reader".into()).unwrap();
    let subscription = api.subscribe("https://example.com/feed.xml").unwrap();
    queue_push(&api, &profile.id, &subscription.subscription_id).unwrap();
    server.state.lock().unwrap().lose_ack = true;
    assert_eq!(
        push_pending(&api, &remote, &profile.id),
        Err(ApiError::Server)
    );
    assert_eq!(
        profiles(&api).unwrap().intents[0].status,
        IntentStatus::Unknown
    );
    server.state.lock().unwrap().subscriptions.clear();
    for _attempt in 0..3 {
        push_pending(&api, &remote, &profile.id).unwrap();
    }
    assert_eq!(server.state.lock().unwrap().posts, 1);
    assert_eq!(
        profiles(&api).unwrap().intents[0].status,
        IntentStatus::Unknown
    );
}

#[test]
fn uncertain_push_can_be_confirmed_by_reading_and_never_posts_twice() {
    let (_directory, api) = local();
    let server = Server::new();
    let remote = server.login();
    let profile = connect(&api, &remote, "reader".into()).unwrap();
    let subscription = api.subscribe("https://example.com/feed.xml").unwrap();
    queue_push(&api, &profile.id, &subscription.subscription_id).unwrap();
    server.state.lock().unwrap().lose_ack = true;
    assert!(push_pending(&api, &remote, &profile.id).is_err());
    push_pending(&api, &remote, &profile.id).unwrap();
    let journal = profiles(&api).unwrap();
    assert_eq!(journal.intents[0].status, IntentStatus::Confirmed);
    assert_eq!(journal.links.len(), 1);
    assert_eq!(server.state.lock().unwrap().posts, 1);
    disconnect(&api, &profile.id).unwrap();
    assert!(profiles(&api).unwrap().profiles.is_empty());
    assert_eq!(api.subscriptions().unwrap().len(), 1);
}

#[test]
fn rejects_repeated_remote_cursor_before_modifying_local_data() {
    let (_directory, api) = local();
    let server = Server::new();
    let remote = server.login();
    let profile = connect(&api, &remote, "reader".into()).unwrap();
    server.state.lock().unwrap().repeat_cursor = true;
    assert_eq!(
        import(&api, &remote, &profile.id).unwrap_err(),
        ApiError::Response
    );
    assert!(api.subscriptions().unwrap().is_empty());
}

#[test]
fn local_database_and_journal_survive_restart_and_enforce_single_instance() {
    let (directory, api) = local();
    let core = api.local_core().unwrap();
    let durability = core
        .block_on(core.database.query_one(Statement::from_string(
            DbBackend::Sqlite,
            "PRAGMA synchronous",
        )))
        .unwrap()
        .unwrap();
    assert_eq!(durability.try_get::<i32>("", "synchronous").unwrap(), 2);
    drop(core);
    let server = Server::new();
    let remote = server.login();
    let profile = connect(&api, &remote, "reader".into()).unwrap();
    let subscription = api.subscribe("https://example.com/feed.xml").unwrap();
    queue_push(&api, &profile.id, &subscription.subscription_id).unwrap();
    assert!(matches!(
        Api::isolated(directory.path().to_path_buf()),
        Err(ApiError::LocalBusy)
    ));
    drop(api);
    let started = std::time::Instant::now();
    let restarted = loop {
        match Api::isolated(directory.path().to_path_buf()) {
            Ok(api) => break api,
            Err(ApiError::LocalBusy) if started.elapsed() < std::time::Duration::from_secs(10) => {
                std::thread::sleep(std::time::Duration::from_millis(10))
            }
            Err(error) => panic!("restart failed: {error}"),
        }
    };
    assert_eq!(
        restarted.subscriptions().unwrap()[0].subscription_id,
        subscription.subscription_id
    );
    assert_eq!(
        profiles(&restarted).unwrap().intents[0].status,
        IntentStatus::Ready
    );
    assert_eq!(profiles(&restarted).unwrap().profiles[0].id, profile.id);
}

#[test]
fn hundred_subscription_import_is_atomic_and_recovery_preserves_metadata() {
    let (_directory, api) = local();
    let server = Server::new();
    let remote = server.login();
    let profile = connect(&api, &remote, "reader".into()).unwrap();
    {
        let mut mock = server.state.lock().unwrap();
        mock.categories
            .push(serde_json::json!({"categoryId":"category-1", "title":"News & <Ideas>"}));
        mock.subscriptions = (0..100)
            .map(|index| {
                let mut subscription = sample(
                    &format!("remote-{index:03}"),
                    &format!("https://example.com/feed-{index}.xml"),
                );
                subscription.title_override = Some(format!("Title {index} & <\"quoted\">"));
                subscription.category_id = Some("category-1".into());
                subscription.position = index;
                subscription
            })
            .collect();
    }
    let report = import(&api, &remote, &profile.id).unwrap();
    assert_eq!(report.added, 100);
    let subscriptions = api.subscriptions().unwrap();
    assert_eq!(subscriptions.len(), 100);
    assert_eq!(
        subscriptions[0].title_override.as_deref(),
        Some("Title 0 & <\"quoted\">")
    );
    assert_eq!(
        subscriptions
            .iter()
            .filter_map(|subscription| subscription.category_id.as_ref())
            .collect::<HashSet<_>>()
            .len(),
        1
    );
    let core = api.local_core().unwrap();
    let mut journal = profiles(&api).unwrap();
    journal.links.clear();
    journal.save(&core).unwrap();
    assert_eq!(import(&api, &remote, &profile.id).unwrap().linked, 100);
    assert_eq!(api.subscriptions().unwrap(), subscriptions);
    assert_eq!(profiles(&api).unwrap().links.len(), 100);
}

#[test]
fn failed_import_rolls_back_categories_and_metadata_before_retry() {
    let (_directory, api) = local();
    let server = Server::new();
    let remote = server.login();
    let profile = connect(&api, &remote, "reader".into()).unwrap();
    {
        let mut mock = server.state.lock().unwrap();
        mock.categories
            .push(serde_json::json!({"categoryId":"category-1", "title":"News"}));
        for (index, title) in ["first", "reject"].into_iter().enumerate() {
            let mut subscription = sample(
                &format!("remote-{index}"),
                &format!("https://example.com/feed-{index}.xml"),
            );
            subscription.title_override = Some(title.into());
            subscription.category_id = Some("category-1".into());
            mock.subscriptions.push(subscription);
        }
    }
    let core = api.local_core().unwrap();
    core.block_on(core.database.execute_unprepared("CREATE TRIGGER fail_import BEFORE INSERT ON subscriptions WHEN NEW.title_override = 'reject' BEGIN SELECT RAISE(ABORT, 'fixture'); END")).unwrap();
    assert!(import(&api, &remote, &profile.id).is_err());
    assert!(api.subscriptions().unwrap().is_empty());
    assert!(
        api.read::<serde_json::Value>("api/v1/categories").unwrap()["items"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert!(profiles(&api).unwrap().links.is_empty());
    core.block_on(core.database.execute_unprepared("DROP TRIGGER fail_import"))
        .unwrap();
    assert_eq!(import(&api, &remote, &profile.id).unwrap().added, 2);
    assert_eq!(
        api.subscriptions()
            .unwrap()
            .iter()
            .filter(|subscription| subscription.category_id.is_some()
                && subscription.title_override.is_some())
            .count(),
        2
    );
}

#[test]
fn unsubscribe_suppresses_uncertain_push_even_before_a_link_exists() {
    let (_directory, api) = local();
    let server = Server::new();
    let remote = server.login();
    let profile = connect(&api, &remote, "reader".into()).unwrap();
    let subscription = api.subscribe("https://example.com/feed.xml").unwrap();
    queue_push(&api, &profile.id, &subscription.subscription_id).unwrap();
    server.state.lock().unwrap().lose_ack = true;
    assert!(push_pending(&api, &remote, &profile.id).is_err());
    unsubscribe(&api, &subscription.subscription_id).unwrap();
    assert_eq!(import(&api, &remote, &profile.id).unwrap().suppressed, 1);
    assert!(api.subscriptions().unwrap().is_empty());
    push_pending(&api, &remote, &profile.id).unwrap();
    assert_eq!(server.state.lock().unwrap().posts, 1);
}

#[test]
fn staged_import_survives_restart_and_unsubscribe_before_binding_never_resurrects() {
    let (directory, api) = local();
    let server = Server::new();
    let remote = server.login();
    let profile = connect(&api, &remote, "reader".into()).unwrap();
    server
        .state
        .lock()
        .unwrap()
        .subscriptions
        .push(sample("remote", "https://example.com/feed.xml"));
    let core = api.local_core().unwrap();
    core.block_on(core.database.execute_unprepared("CREATE TRIGGER fail_import_links BEFORE UPDATE ON desktop_sync_state WHEN json_array_length(json_extract(NEW.document, '$.links')) > 0 BEGIN SELECT RAISE(ABORT, 'fixture'); END")).unwrap();
    assert_eq!(
        import(&api, &remote, &profile.id).unwrap_err(),
        ApiError::LocalData
    );
    let subscription = api.subscriptions().unwrap().pop().unwrap();
    assert_eq!(profiles(&api).unwrap().pending_imports.len(), 1);
    assert!(profiles(&api).unwrap().links.is_empty());
    drop(core);
    drop(api);
    let started = std::time::Instant::now();
    let restarted = loop {
        match Api::isolated(directory.path().to_path_buf()) {
            Ok(api) => break api,
            Err(ApiError::LocalBusy) if started.elapsed() < std::time::Duration::from_secs(10) => {
                std::thread::sleep(std::time::Duration::from_millis(10))
            }
            Err(error) => panic!("restart failed: {error}"),
        }
    };
    unsubscribe(&restarted, &subscription.subscription_id).unwrap();
    let core = restarted.local_core().unwrap();
    core.block_on(
        core.database
            .execute_unprepared("DROP TRIGGER fail_import_links"),
    )
    .unwrap();
    assert_eq!(
        import(&restarted, &remote, &profile.id).unwrap().suppressed,
        1
    );
    assert!(restarted.subscriptions().unwrap().is_empty());
}

#[test]
fn remote_pagination_has_page_cursor_and_cumulative_response_budgets() {
    let server = Server::new();
    let remote = server.login();
    server.state.lock().unwrap().unique_cursors = true;
    assert_eq!(remote.subscriptions().unwrap_err(), ApiError::Response);
    assert_eq!(server.state.lock().unwrap().subscription_pages, 256);
    {
        let mut state = server.state.lock().unwrap();
        state.subscription_pages = 0;
        state.cursor_bytes = 4097;
    }
    assert_eq!(remote.subscriptions().unwrap_err(), ApiError::Response);
    assert_eq!(server.state.lock().unwrap().subscription_pages, 1);
    {
        let mut state = server.state.lock().unwrap();
        state.subscription_pages = 0;
        state.cursor_bytes = 0;
        state.page_padding = 1024 * 1024;
    }
    assert_eq!(remote.subscriptions().unwrap_err(), ApiError::Response);
    assert!(server.state.lock().unwrap().subscription_pages <= 32);
}
