use super::*;
use axum::{
    Router,
    body::Bytes,
    extract::State,
    http::{HeaderMap, Method, Uri},
    response::IntoResponse,
};
use nsh_engine::{EngineConfig, Root};
use nsh_testing::Headless;
use std::sync::{Arc, Mutex};

struct Server {
    address: String,
    saved: Arc<Mutex<serde_json::Value>>,
    shutdown: Option<tokio::sync::oneshot::Sender<()>>,
    thread: Option<std::thread::JoinHandle<()>>,
}
impl Server {
    fn new() -> Self {
        let saved = Arc::new(Mutex::new(
            serde_json::json!({"isEnabled":false,"baseUrl":"https://tts.example/v1","model":"tts-1","voice":"zh-CN-YunyangNeural","speed":1.0,"hasApiKey":true,"keyringAvailable":true,"revision":0}),
        ));
        let state = saved.clone();
        let (sender, receiver) = std::sync::mpsc::sync_channel(1);
        let (shutdown, stopped) = tokio::sync::oneshot::channel();
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
                    axum::serve(listener, Router::new().fallback(dispatch).with_state(state))
                        .with_graceful_shutdown(async {
                            let _ = stopped.await;
                        })
                        .await
                        .unwrap();
                });
        });
        Self {
            address: receiver.recv().unwrap(),
            saved,
            shutdown: Some(shutdown),
            thread: Some(thread),
        }
    }
    fn app(&self) -> Headless {
        let address = self.address.clone();
        Headless::start(EngineConfig {
            viewport: Size::new(px(1100.0), px(1200.0)),
            ..EngineConfig::new(Root::new::<Fixture>(move || FixtureProps {
                address: address.clone(),
            }))
        })
        .unwrap()
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
    State(saved): State<Arc<Mutex<serde_json::Value>>>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
) -> axum::response::Response {
    if uri.path() == "/api/v1/auth/login" {
        return ([("set-cookie", "session=fixture; Path=/; HttpOnly")], axum::Json(serde_json::json!({"csrfToken":"fixture","user":{"id":"reader","username":"reader"}}))).into_response();
    }
    assert_eq!(headers["cookie"], "session=fixture");
    if method == Method::PUT {
        assert_eq!(headers["x-csrf-token"], "fixture");
        let draft: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let mut value = saved.lock().unwrap();
        assert_eq!(draft["revision"], value["revision"]);
        assert!(
            draft.get("apiKey").is_none(),
            "blank key should preserve stored key"
        );
        for field in ["isEnabled", "baseUrl", "model", "voice", "speed"] {
            value[field] = draft[field].clone();
        }
        value["revision"] = (value["revision"].as_u64().unwrap() + 1).into();
    }
    axum::Json(saved.lock().unwrap().clone()).into_response()
}

#[derive(Clone, PartialEq, Props)]
struct FixtureProps {
    address: String,
}
#[component]
fn Fixture(props: FixtureProps) -> impl IntoElement {
    set_tokens(nsh_base::tokens_of(Some(include_str!("../../theme.json"))).unwrap());
    let api = Api::login(
        &props.address,
        "reader",
        zeroize::Zeroizing::new("password".into()),
    )
    .unwrap()
    .0;
    let store = Store::with_api(api);
    provide_context(store);
    scroll().child(
        view()
            .gap_4()
            .child(speech_settings())
            .child(controls(store)),
    )
}

#[test]
fn speech_settings_persist_explicit_edge_and_custom_voice_choices() -> Result<(), String> {
    let server = Server::new();
    {
        let mut app = server.app();
        app.wait_for("保存语音配置")?;
        assert!(app.screen().selected("role:button[Edge 云扬]")?);
        app.press("role:button[Edge 晓晓]")?;
        app.press("启用语音朗读")?;
        app.press("保存语音配置")?;
        app.wait_for("语音配置已保存。")?;
        assert_eq!(
            server.saved.lock().unwrap()["voice"],
            "zh-CN-XiaoxiaoNeural"
        );
        assert_eq!(server.saved.lock().unwrap()["isEnabled"], true);
    }
    {
        let mut app = server.app();
        app.wait_for("保存语音配置")?;
        assert!(app.screen().selected("role:button[Edge 晓晓]")?);
        app.press("自定义声音")?;
        app.clear("label:声音 ID")?;
        app.type_text("label:声音 ID", "my-provider-custom-voice")?;
        app.press("保存语音配置")?;
        app.wait_for("语音配置已保存。")?;
        assert_eq!(
            server.saved.lock().unwrap()["voice"],
            "my-provider-custom-voice"
        );
    }
    let mut app = server.app();
    app.wait_for("label:声音 ID")?;
    assert!(app.screen().selected("role:button[自定义声音]")?);
    assert_eq!(
        app.screen().input_text("label:声音 ID")?,
        Some("my-provider-custom-voice")
    );
    app.wait_for("label:朗读文章")?;
    app.wait_for("label:停止朗读")?;
    Ok(())
}

#[test]
fn speech_preview_validates_unsaved_draft_before_network_or_audio() -> Result<(), String> {
    let server = Server::new();
    let mut app = server.app();
    app.wait_for("保存语音配置")?;
    app.clear("label:语速（0.25–4）")?;
    app.type_text("label:语速（0.25–4）", "9")?;
    app.press("试听")?;
    app.wait_for("语速应在 0.25 到 4 之间。")?;
    assert_eq!(server.saved.lock().unwrap()["revision"], 0);
    Ok(())
}
