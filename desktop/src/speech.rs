use crate::api::{Api, ApiError, Cancellation};
use rodio::{Decoder, DeviceSinkBuilder, Player};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    io::Cursor,
    sync::{Arc, Mutex},
    time::Duration,
};

pub(crate) const VOICES: &[(&str, &str)] = &[
    ("zh-CN-YunyangNeural", "Edge 云扬"),
    ("zh-CN-XiaoxiaoNeural", "Edge 晓晓"),
    ("alloy", "OpenAI Alloy"),
    ("ash", "OpenAI Ash"),
    ("ballad", "OpenAI Ballad"),
    ("coral", "OpenAI Coral"),
    ("echo", "OpenAI Echo"),
    ("fable", "OpenAI Fable"),
    ("nova", "OpenAI Nova"),
    ("onyx", "OpenAI Onyx"),
    ("sage", "OpenAI Sage"),
    ("shimmer", "OpenAI Shimmer"),
    ("verse", "OpenAI Verse"),
];

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Config {
    pub(crate) is_enabled: bool,
    pub(crate) base_url: String,
    pub(crate) model: String,
    pub(crate) voice: String,
    pub(crate) speed: f64,
    pub(crate) has_api_key: bool,
    pub(crate) keyring_available: bool,
    pub(crate) revision: u64,
}

impl Config {
    pub(crate) fn draft(&self, key: &str, clear: bool) -> Value {
        let mut body = json!({"isEnabled": self.is_enabled, "baseUrl": self.base_url,
            "model": self.model, "voice": self.voice, "speed": self.speed, "revision": self.revision});
        if clear {
            body["apiKey"] = Value::Null;
        } else if !key.is_empty() {
            body["apiKey"] = Value::String(key.to_owned());
        }
        body
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Plan {
    entry_id: String,
    content_hash: String,
    chunks: Vec<String>,
}

fn decode_mp3(bytes: Vec<u8>) -> Result<Decoder<Cursor<Vec<u8>>>, &'static str> {
    Decoder::try_from(Cursor::new(bytes)).map_err(|_| "无法解码语音音频，请检查服务配置。")
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Status {
    #[default]
    Idle,
    Generating,
    Playing,
    Paused,
    Failed(&'static str),
}

#[derive(Default)]
struct State {
    generation: u64,
    status: Status,
    paused: bool,
    player: Option<Player>,
    cancellation: Cancellation,
    worker: Option<std::thread::JoinHandle<()>>,
}

#[derive(Clone, Default)]
pub(crate) struct Playback(Arc<Mutex<State>>);

impl Playback {
    fn state(&self) -> std::sync::MutexGuard<'_, State> {
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
    pub(crate) fn status(&self) -> Status {
        self.state().status
    }
    pub(crate) fn stop(&self) {
        let mut state = self.state();
        state.cancellation.cancel();
        state.generation = state.generation.wrapping_add(1);
        if let Some(player) = state.player.take() {
            player.stop();
        }
        state.status = Status::Idle;
        state.paused = false;
    }
    pub(crate) fn toggle_pause(&self) {
        let mut state = self.state();
        if !matches!(
            state.status,
            Status::Playing | Status::Generating | Status::Paused
        ) {
            return;
        }
        state.paused = !state.paused;
        if let Some(player) = &state.player {
            if state.paused {
                player.pause();
            } else {
                player.play();
            }
        }
        state.status = if state.paused {
            Status::Paused
        } else if state.player.is_some() {
            Status::Playing
        } else {
            Status::Generating
        };
    }
    fn current(&self, generation: u64) -> bool {
        self.state().generation == generation
    }
    pub(crate) fn article(&self, api: Api, id: String) {
        self.start(move |playback, generation, cancellation| {
            if id.is_empty()
                || !id
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
            {
                return Err(ApiError::Response.message());
            }
            let path = format!("api/v1/plugins/tts/entries/{id}");
            let plan: Plan = api
                .speech_json(&format!("{path}/plan"), &json!({}), &cancellation)
                .map_err(ApiError::message)?;
            if plan.entry_id != id || plan.content_hash.is_empty() || plan.chunks.is_empty() {
                return Err(ApiError::Response.message());
            }
            playback.play_chunks(
                generation,
                plan.chunks.len(),
                cancellation,
                move |index, cancellation| {
                    api.binary(
                        &format!("{path}/speak"),
                        &json!({"contentHash": plan.content_hash, "chunkIndex": index}),
                        cancellation,
                    )
                },
            )
        });
    }
    pub(crate) fn preview(&self, api: Api, draft: Value) {
        self.start(move |playback, generation, cancellation| {
            playback.play_chunks(generation, 1, cancellation, move |_, cancellation| {
                api.binary("api/v1/plugins/tts/test", &draft, cancellation)
            })
        });
    }
    fn start(
        &self,
        work: impl FnOnce(&Self, u64, Cancellation) -> Result<(), &'static str> + Send + 'static,
    ) {
        self.stop();
        let (generation, cancellation, previous) = {
            let mut state = self.state();
            state.status = Status::Generating;
            state.cancellation = Cancellation::default();
            (
                state.generation,
                state.cancellation.clone(),
                state.worker.take(),
            )
        };
        let playback = self.clone();
        let result = std::thread::Builder::new()
            .name("raindrop-speech".into())
            .spawn(move || {
                if let Some(previous) = previous {
                    let _ = previous.join();
                }
                if !playback.current(generation) {
                    return;
                }
                let result = work(&playback, generation, cancellation);
                let mut state = playback.state();
                if state.generation == generation {
                    if let Some(player) = state.player.take() {
                        player.stop();
                    }
                    state.status = result.map_or_else(Status::Failed, |_| Status::Idle);
                }
            });
        let mut state = self.state();
        match result {
            Ok(worker) => state.worker = Some(worker),
            Err(_) => state.status = Status::Failed("无法启动音频播放。"),
        }
    }

    fn play_chunks(
        &self,
        generation: u64,
        count: usize,
        cancellation: Cancellation,
        fetch: impl Fn(usize, &Cancellation) -> Result<Vec<u8>, ApiError>,
    ) -> Result<(), &'static str> {
        if !self.current(generation) {
            return Ok(());
        }
        let mut device = DeviceSinkBuilder::open_default_sink()
            .map_err(|_| "无法打开音频设备，请检查系统声音设置。")?;
        device.log_on_drop(false);
        for index in 0..count {
            while self.current(generation) && self.state().paused {
                std::thread::sleep(Duration::from_millis(30));
            }
            if !self.current(generation) {
                return Ok(());
            }
            {
                let mut state = self.state();
                if state.generation != generation {
                    return Ok(());
                }
                state.status = if state.paused {
                    Status::Paused
                } else {
                    Status::Generating
                };
            }
            let bytes = fetch(index, &cancellation).map_err(ApiError::message)?;
            if !self.current(generation) {
                return Ok(());
            }
            let decoder = decode_mp3(bytes)?;
            {
                let mut state = self.state();
                if state.generation != generation {
                    return Ok(());
                }
                let player = Player::connect_new(device.mixer());
                if state.paused {
                    player.pause();
                }
                player.append(decoder);
                state.status = if state.paused {
                    Status::Paused
                } else {
                    Status::Playing
                };
                state.player = Some(player);
            }
            loop {
                let state = self.state();
                if state.generation != generation {
                    return Ok(());
                }
                if state.player.as_ref().is_none_or(Player::empty) {
                    break;
                }
                drop(state);
                std::thread::sleep(Duration::from_millis(30));
            }
            let mut state = self.state();
            if state.generation != generation {
                return Ok(());
            }
            state.player = None;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn decodes_real_mp3_without_opening_an_audio_device() {
        use rodio::Source;
        let decoder = decode_mp3(include_bytes!("fixtures/silence.mp3").to_vec()).unwrap();
        assert_eq!(decoder.sample_rate().get(), 24_000);
        assert_eq!(decoder.channels().get(), 1);
        let samples: Vec<_> = decoder.collect();
        assert!(
            samples.len() >= 2400,
            "the 100 ms fixture must contain decoded samples"
        );
        assert!(samples.iter().all(|sample| sample.is_finite()));
    }

    #[test]
    fn rejects_html_and_invalid_mp3_without_opening_an_audio_device() {
        for bytes in [
            b"<html>Access denied</html>".as_slice(),
            b"ID3\0invalid",
            b"",
        ] {
            assert!(
                decode_mp3(bytes.to_vec()).is_err(),
                "non-audio response must not play"
            );
        }
    }

    #[test]
    fn cancelled_worker_cannot_replace_a_new_session_status() {
        let playback = Playback::default();
        let (ready, started) = std::sync::mpsc::channel();
        let (release, released) = std::sync::mpsc::channel();
        let (done, completed) = std::sync::mpsc::channel();
        playback.start(move |playback, generation, _cancellation| {
            ready.send(()).unwrap();
            released.recv().unwrap();
            assert!(!playback.current(generation));
            done.send(()).unwrap();
            Err("obsolete request")
        });
        started.recv_timeout(Duration::from_secs(2)).unwrap();
        playback.stop();
        playback.state().status = Status::Paused;
        release.send(()).unwrap();
        completed.recv_timeout(Duration::from_secs(2)).unwrap();
        assert_eq!(playback.status(), Status::Paused);
    }

    #[test]
    fn draft_preserves_selected_voice_and_key_intent() {
        let mut config: Config = serde_json::from_value(json!({"isEnabled":true,"baseUrl":"https://tts.example/v1","model":"tts-1","voice":"zh-CN-YunyangNeural","speed":1.0,"hasApiKey":true,"keyringAvailable":true,"revision":7})).unwrap();
        assert_eq!(config.draft("", false)["voice"], "zh-CN-YunyangNeural");
        assert!(config.draft("", false).get("apiKey").is_none());
        assert!(config.draft("", true)["apiKey"].is_null());
        assert_eq!(config.draft("secret", false)["apiKey"], "secret");
        config.is_enabled = false;
        assert_eq!(
            config.draft("", false)["isEnabled"],
            false,
            "disabled drafts remain usable for explicit preview"
        );
        config.voice = "my-custom/voice".into();
        assert_eq!(config.draft("", false)["voice"], "my-custom/voice");
        assert_eq!(config.draft("", false)["revision"], 7);
    }
    #[test]
    fn stop_invalidates_generation_even_while_request_is_pending() {
        let playback = Playback::default();
        let generation = playback.state().generation;
        playback.state().status = Status::Generating;
        playback.toggle_pause();
        assert_eq!(playback.status(), Status::Paused);
        playback.toggle_pause();
        assert_eq!(playback.status(), Status::Generating);
        playback.stop();
        assert!(!playback.current(generation));
        assert_eq!(playback.status(), Status::Idle);
        assert!(
            playback
                .play_chunks(generation, 1, Cancellation::default(), |_, _| panic!(
                    "stale synthesis requested"
                ))
                .is_ok()
        );
    }
}

#[cfg(test)]
#[path = "speech/transport_tests.rs"]
mod transport_tests;
