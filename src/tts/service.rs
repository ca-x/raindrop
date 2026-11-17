use std::sync::Arc;

use http::{
    HeaderValue,
    header::{ACCEPT, AUTHORIZATION, CONTENT_TYPE, USER_AGENT},
};
use secrecy::{ExposeSecret, SecretString};
use serde::Serialize;

use super::{
    SpeechDraft, SpeechError, SpeechRepository,
    repository::{Settings, speech_endpoint},
};
use crate::{
    content::{
        provider::{
            EncodedProviderRequest, ProviderHeader, ProviderTransport, ProviderTransportErrorKind,
        },
        sanitize::rendered_translation_segments,
    },
    feeds::FeedRepository,
};

pub const MAX_AUDIO_BYTES: usize = 2 * 1024 * 1024;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SpeechPlan {
    pub entry_id: String,
    pub content_hash: String,
    pub chunks: Vec<String>,
}

pub struct SpeechService {
    repository: SpeechRepository,
    feeds: FeedRepository,
    transport: Arc<dyn ProviderTransport>,
}

impl SpeechService {
    #[must_use]
    pub fn new(
        repository: SpeechRepository,
        feeds: FeedRepository,
        transport: Arc<dyn ProviderTransport>,
    ) -> Self {
        Self {
            repository,
            feeds,
            transport,
        }
    }

    pub async fn test(&self, user_id: &str, draft: SpeechDraft) -> Result<Vec<u8>, SpeechError> {
        let text = draft
            .text
            .as_deref()
            .unwrap_or("你好，这是朗读测试。 Hello, this is a reading preview.");
        if text.trim().is_empty() || text.chars().count() > 200 {
            return Err(SpeechError::InvalidInput);
        }
        let (settings, key) = self.repository.preview(user_id, &draft).await?;
        self.synthesize(user_id, &settings, key, text).await
    }

    pub async fn plan(&self, user_id: &str, entry_id: &str) -> Result<SpeechPlan, SpeechError> {
        let (settings, revision, _) = self.repository.snapshot(user_id).await?;
        if !settings.is_enabled {
            return Err(SpeechError::Disabled);
        }
        self.article_plan(user_id, entry_id, revision).await
    }

    pub async fn speak(
        &self,
        user_id: &str,
        entry_id: &str,
        content_hash: &str,
        chunk_index: usize,
    ) -> Result<Vec<u8>, SpeechError> {
        if content_hash.len() != 64 || !content_hash.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(SpeechError::InvalidInput);
        }
        let (settings, revision, key) = self.repository.snapshot(user_id).await?;
        if !settings.is_enabled {
            return Err(SpeechError::Disabled);
        }
        let plan = self.article_plan(user_id, entry_id, revision).await?;
        if plan.content_hash != content_hash {
            return Err(SpeechError::Conflict);
        }
        let text = plan
            .chunks
            .get(chunk_index)
            .ok_or(SpeechError::InvalidInput)?;
        self.synthesize(user_id, &settings, key, text).await
    }

    async fn article_plan(
        &self,
        user_id: &str,
        entry_id: &str,
        revision: i64,
    ) -> Result<SpeechPlan, SpeechError> {
        if uuid::Uuid::parse_str(entry_id).is_err() {
            return Err(SpeechError::InvalidInput);
        }
        let entry = self
            .feeds
            .get_detail_for_user(user_id, entry_id)
            .await
            .map_err(|_| SpeechError::Storage)?
            .ok_or(SpeechError::NotFound)?;
        let mut segments = rendered_translation_segments(&entry.content_html);
        if segments.is_empty()
            && let Some(summary) = entry.summary
        {
            segments.push(summary);
        }
        if let Some(title) = entry.title.filter(|title| !title.trim().is_empty()) {
            segments.insert(0, title);
        }
        let text = segments.join("\n\n");
        let chunks = raindrop_tts::article_chunks(&text).map_err(|_| SpeechError::InvalidInput)?;
        let snapshot = serde_json::to_vec(&(
            raindrop_tts::PLUGIN_KEY,
            user_id,
            entry_id,
            revision,
            &chunks,
        ))
        .map_err(|_| SpeechError::Storage)?;
        let content_hash = blake3::hash(&snapshot).to_hex().to_string();
        Ok(SpeechPlan {
            entry_id: entry_id.into(),
            content_hash,
            chunks,
        })
    }

    async fn synthesize(
        &self,
        user_id: &str,
        settings: &Settings,
        key: Option<SecretString>,
        text: &str,
    ) -> Result<Vec<u8>, SpeechError> {
        let endpoint = speech_endpoint(&settings.base_url)?;
        let mut headers = vec![
            ProviderHeader::public(CONTENT_TYPE, HeaderValue::from_static("application/json")),
            ProviderHeader::public(ACCEPT, HeaderValue::from_static("audio/mpeg")),
            ProviderHeader::public(
                USER_AGENT,
                HeaderValue::from_static(concat!("Raindrop/", env!("CARGO_PKG_VERSION"))),
            ),
        ];
        if let Some(key) = key {
            headers.push(ProviderHeader::secret(
                AUTHORIZATION,
                format!("Bearer {}", key.expose_secret()).into(),
            ));
        }
        let body = settings
            .parameters()
            .request_body(text)
            .map_err(|_| SpeechError::InvalidInput)?;
        let request = EncodedProviderRequest::new("/audio/speech".into(), headers, body)
            .map_err(|_| SpeechError::InvalidInput)?;
        let response = tokio::time::timeout(
            std::time::Duration::from_secs(90),
            self.transport.execute(user_id, &endpoint, request),
        )
        .await
        .map_err(|_| SpeechError::Timeout)?
        .map_err(|error| match error.kind() {
            ProviderTransportErrorKind::Timeout => SpeechError::Timeout,
            ProviderTransportErrorKind::ResponseTooLarge | ProviderTransportErrorKind::Decode => {
                SpeechError::UpstreamInvalid
            }
            _ => SpeechError::Upstream,
        })?;
        match response.status().as_u16() {
            200..=299 => {}
            401 | 403 => return Err(SpeechError::UpstreamAuth),
            429 => return Err(SpeechError::RateLimited),
            _ => return Err(SpeechError::Upstream),
        }
        let audio = response.body();
        if audio.len() > MAX_AUDIO_BYTES || !is_mp3(audio) {
            return Err(SpeechError::UpstreamInvalid);
        }
        Ok(audio.to_vec())
    }
}

fn is_mp3(audio: &[u8]) -> bool {
    if audio.len() < 16 {
        return false;
    }
    let frame = if audio.starts_with(b"ID3") {
        if !matches!(audio[3], 2..=4) || audio[6..10].iter().any(|byte| byte & 0x80 != 0) {
            return false;
        }
        let size = audio[6..10]
            .iter()
            .fold(0usize, |size, byte| (size << 7) | usize::from(*byte));
        let footer = if audio[3] == 4 && audio[5] & 0x10 != 0 {
            10
        } else {
            0
        };
        let Some(frame) = audio.get(10 + size + footer..) else {
            return false;
        };
        frame
    } else {
        audio
    };
    frame.len() >= 4
        && frame[0] == 0xff
        && frame[1] & 0xe0 == 0xe0
        && (frame[1] >> 3) & 3 != 1
        && frame[1] & 6 == 2
        && !matches!(frame[2] >> 4, 0 | 15)
        && (frame[2] >> 2) & 3 != 3
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_html_json_and_invalid_id3_are_not_served_as_audio() {
        for invalid in [
            b"<html>not audio</html>".as_slice(),
            b"{\"error\":\"credential leak\"}",
            b"ID3\x04\0\0\x7f\x7f\x7f\x7f123456",
        ] {
            assert!(!is_mp3(invalid));
        }
        let mut frame = vec![0xff, 0xfb, 0x90, 0x00];
        frame.extend([0; 32]);
        assert!(is_mp3(&frame));
        let mut tagged = b"ID3\x04\0\0\0\0\0\0".to_vec();
        tagged.extend(frame);
        assert!(is_mp3(&tagged));
    }
}
