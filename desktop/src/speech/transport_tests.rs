use super::*;
use raindrop::content::provider::{
    EncodedProviderRequest, ProviderEndpoint, ProviderTransport, ProviderTransportError,
    ProviderTransportResponse,
};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    mpsc,
};

struct BlockedTransport {
    calls: AtomicUsize,
    dropped: Arc<AtomicUsize>,
    entered: mpsc::Sender<usize>,
}

struct DroppedFuture(Arc<AtomicUsize>);
impl Drop for DroppedFuture {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

#[async_trait::async_trait]
impl ProviderTransport for BlockedTransport {
    async fn execute(
        &self,
        _provider_id: &str,
        _endpoint: &ProviderEndpoint,
        _request: EncodedProviderRequest,
    ) -> Result<ProviderTransportResponse, ProviderTransportError> {
        let call = self.calls.fetch_add(1, Ordering::SeqCst);
        self.entered.send(call).unwrap();
        if call == 0 {
            let _dropped = DroppedFuture(self.dropped.clone());
            std::future::pending::<()>().await;
        }
        assert_eq!(
            self.dropped.load(Ordering::SeqCst),
            1,
            "replacement must wait for the cancelled upstream future to drop"
        );
        Ok(ProviderTransportResponse::new(
            axum::http::StatusCode::OK,
            include_bytes!("../fixtures/silence.mp3").to_vec(),
            None,
        ))
    }
}

fn cancel_local_synthesis(explicit_stop: bool) {
    let directory = tempfile::tempdir().unwrap();
    let (entered, requests) = mpsc::channel();
    let transport = Arc::new(BlockedTransport {
        calls: AtomicUsize::new(0),
        dropped: Arc::new(AtomicUsize::new(0)),
        entered,
    });
    let api = Api::isolated_speech(directory.path().to_path_buf(), transport.clone()).unwrap();
    let playback = Playback::default();
    let draft = json!({"isEnabled":false,"baseUrl":"https://tts.example/v1","model":"tts-1","voice":"zh-CN-YunyangNeural","speed":1.0,"revision":0,"text":"preview"});
    let (first, first_done) = mpsc::channel();
    let first_api = api.clone();
    let first_draft = draft.clone();
    playback.start(move |_, _, cancellation| {
        let result = first_api.binary("api/v1/plugins/tts/test", &first_draft, &cancellation);
        first.send(result).unwrap();
        Ok(())
    });
    assert_eq!(requests.recv_timeout(Duration::from_secs(3)).unwrap(), 0);
    if explicit_stop {
        playback.stop();
        assert_eq!(
            first_done.recv_timeout(Duration::from_secs(3)).unwrap(),
            Err(ApiError::Cancelled)
        );
        assert_eq!(transport.dropped.load(Ordering::SeqCst), 1);
    }
    let (second, second_done) = mpsc::channel();
    playback.start(move |_, _, cancellation| {
        let result = api.binary("api/v1/plugins/tts/test", &draft, &cancellation);
        second.send(result).unwrap();
        Ok(())
    });
    if !explicit_stop {
        assert_eq!(
            first_done.recv_timeout(Duration::from_secs(3)).unwrap(),
            Err(ApiError::Cancelled)
        );
    }
    assert_eq!(requests.recv_timeout(Duration::from_secs(3)).unwrap(), 1);
    assert_eq!(
        second_done
            .recv_timeout(Duration::from_secs(3))
            .unwrap()
            .unwrap(),
        include_bytes!("../fixtures/silence.mp3")
    );
    assert_eq!(transport.calls.load(Ordering::SeqCst), 2);
    assert_eq!(transport.dropped.load(Ordering::SeqCst), 1);
    playback.stop();
}

#[test]
fn stopping_local_speech_drops_upstream_future_and_releases_user_permit() {
    cancel_local_synthesis(true);
}

#[test]
fn replacing_local_speech_waits_for_cancelled_request_before_acquiring_permit() {
    cancel_local_synthesis(false);
}
