mod repository;
mod service;

pub use repository::{SpeechConfig, SpeechDraft, SpeechRepository};
pub use service::{SpeechPlan, SpeechService};

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum SpeechError {
    #[error("Invalid speech configuration or input")]
    InvalidInput,
    #[error("Speech configuration or article changed; reload before retrying")]
    Conflict,
    #[error("Speech playback is disabled")]
    Disabled,
    #[error("Article not found")]
    NotFound,
    #[error("Speech credential encryption is unavailable")]
    KeyringUnavailable,
    #[error("Speech provider authentication failed")]
    UpstreamAuth,
    #[error("Speech provider returned invalid audio")]
    UpstreamInvalid,
    #[error("Speech provider is unavailable")]
    Upstream,
    #[error("Speech provider timed out")]
    Timeout,
    #[error("Too many speech requests")]
    RateLimited,
    #[error("Speech storage is unavailable")]
    Storage,
}
