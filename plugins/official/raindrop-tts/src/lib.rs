use serde::{Deserialize, Serialize};

pub const PLUGIN_KEY: &str = "raindrop.tts";
pub const VERSION: &str = "1.0.0";
pub const MAX_CHUNK_CHARACTERS: usize = 600;
pub const MAX_ARTICLE_CHARACTERS: usize = 200_000;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpeechParameters {
    pub model: String,
    pub voice: String,
    pub speed: f64,
}

impl Default for SpeechParameters {
    fn default() -> Self {
        Self {
            model: "tts-1".into(),
            voice: "alloy".into(),
            speed: 1.0,
        }
    }
}

impl SpeechParameters {
    #[must_use]
    pub fn is_valid(&self) -> bool {
        [&self.model, &self.voice].into_iter().all(|value| {
            !value.is_empty()
                && value.len() <= 128
                && value.trim() == value
                && !value.chars().any(char::is_control)
        }) && self.speed.is_finite()
            && (0.25..=4.0).contains(&self.speed)
    }

    pub fn request_body(&self, input: &str) -> Result<Vec<u8>, SpeechInputError> {
        if !self.is_valid()
            || input.trim().is_empty()
            || input.chars().count() > MAX_CHUNK_CHARACTERS
        {
            return Err(SpeechInputError);
        }
        serde_json::to_vec(&serde_json::json!({
            "model": self.model, "voice": self.voice, "speed": self.speed,
            "input": input, "response_format": "mp3"
        }))
        .map_err(|_| SpeechInputError)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SpeechInputError;

pub fn article_chunks(text: &str) -> Result<Vec<String>, SpeechInputError> {
    if text.trim().is_empty() || text.chars().count() > MAX_ARTICLE_CHARACTERS {
        return Err(SpeechInputError);
    }
    let mut chunks = Vec::new();
    let mut chunk = String::new();
    let mut characters = 0;
    for character in text.chars() {
        chunk.push(character);
        characters += 1;
        let sentence_end = matches!(character, '\n' | '。' | '！' | '？' | '.' | '!' | '?');
        if characters == MAX_CHUNK_CHARACTERS || characters >= 300 && sentence_end {
            if !chunk.trim().is_empty() {
                chunks.push(chunk.clone());
            }
            chunk.clear();
            characters = 0;
        }
    }
    if !chunk.trim().is_empty() {
        chunks.push(chunk);
    }
    if chunks.is_empty() {
        Err(SpeechInputError)
    } else {
        Ok(chunks)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_custom_voice_and_standard_openai_fields_are_preserved() {
        let parameters = SpeechParameters {
            voice: "zh-CN-YunyangNeural".into(),
            ..Default::default()
        };
        let body: serde_json::Value =
            serde_json::from_slice(&parameters.request_body("你好 & hello").unwrap()).unwrap();
        assert_eq!(
            body,
            serde_json::json!({"model":"tts-1","voice":"zh-CN-YunyangNeural","speed":1.0,"input":"你好 & hello","response_format":"mp3"})
        );
    }

    #[test]
    fn long_unicode_articles_are_not_truncated_or_split_in_the_middle_of_a_character() {
        let source = "你好。🦀 testing\n".repeat(250);
        let chunks = article_chunks(&source).unwrap();
        assert_eq!(chunks.concat(), source);
        assert!(
            chunks
                .iter()
                .all(|chunk| chunk.chars().count() <= MAX_CHUNK_CHARACTERS)
        );
    }

    #[test]
    fn invalid_or_unbounded_input_is_rejected() {
        assert!(article_chunks("   \n").is_err());
        assert!(article_chunks(&"a".repeat(MAX_ARTICLE_CHARACTERS + 1)).is_err());
        let mut parameters = SpeechParameters::default();
        for speed in [f64::NAN, f64::INFINITY, 0.24, 4.01] {
            parameters.speed = speed;
            assert!(!parameters.is_valid());
        }
        parameters.speed = 1.0;
        parameters.voice = "bad\nvoice".into();
        assert!(!parameters.is_valid());
    }
}
