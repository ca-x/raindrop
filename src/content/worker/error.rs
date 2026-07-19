use std::{error::Error, fmt, time::Duration};

use crate::content::jobs::{ArtifactCandidate, AttemptFailure, AttemptUsage};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ContentWorkerErrorKind {
    InvalidConfiguration,
    RuntimeUnavailable,
    SupervisionFailed,
}

#[derive(Clone, Copy, Eq, PartialEq)]
pub struct ContentWorkerError {
    kind: ContentWorkerErrorKind,
}

impl ContentWorkerError {
    pub(crate) const fn new(kind: ContentWorkerErrorKind) -> Self {
        Self { kind }
    }

    #[must_use]
    pub const fn kind(&self) -> ContentWorkerErrorKind {
        self.kind
    }
}

impl fmt::Debug for ContentWorkerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ContentWorkerError")
            .field("kind", &self.kind)
            .finish()
    }
}

impl fmt::Display for ContentWorkerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self.kind {
            ContentWorkerErrorKind::InvalidConfiguration => {
                "content worker configuration is invalid"
            }
            ContentWorkerErrorKind::RuntimeUnavailable => "content worker runtime is unavailable",
            ContentWorkerErrorKind::SupervisionFailed => "content worker supervision failed",
        })
    }
}

impl Error for ContentWorkerError {}

pub struct ContentProcessSuccess {
    artifact: ArtifactCandidate,
    usage: AttemptUsage,
}

impl ContentProcessSuccess {
    #[must_use]
    pub const fn new(artifact: ArtifactCandidate, usage: AttemptUsage) -> Self {
        Self { artifact, usage }
    }

    #[must_use]
    pub const fn artifact(&self) -> &ArtifactCandidate {
        &self.artifact
    }

    #[must_use]
    pub const fn usage(&self) -> &AttemptUsage {
        &self.usage
    }

    #[must_use]
    pub fn into_parts(self) -> (ArtifactCandidate, AttemptUsage) {
        (self.artifact, self.usage)
    }
}

impl fmt::Debug for ContentProcessSuccess {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ContentProcessSuccess")
            .field("usage", &self.usage)
            .finish_non_exhaustive()
    }
}

pub struct ContentProcessFailure {
    attempt_failure: AttemptFailure,
}

impl ContentProcessFailure {
    #[must_use]
    pub const fn from_attempt_failure(attempt_failure: AttemptFailure) -> Self {
        Self { attempt_failure }
    }

    pub(crate) fn fixed(
        error_code: &'static str,
        retryable: bool,
        outcome_unknown: bool,
        retry_after: Option<Duration>,
        usage: AttemptUsage,
    ) -> Self {
        let attempt_failure = AttemptFailure::new(
            error_code.to_owned(),
            retryable,
            outcome_unknown,
            retry_after,
            usage,
        )
        .expect("fixed content processor failure must satisfy the repository contract");
        Self { attempt_failure }
    }

    #[must_use]
    pub const fn attempt_failure(&self) -> &AttemptFailure {
        &self.attempt_failure
    }

    #[must_use]
    pub fn into_attempt_failure(self) -> AttemptFailure {
        self.attempt_failure
    }
}

impl fmt::Debug for ContentProcessFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ContentProcessFailure")
            .field("error_code", &self.attempt_failure.error_code())
            .field("retryable", &self.attempt_failure.retryable())
            .field("outcome_unknown", &self.attempt_failure.outcome_unknown())
            .finish()
    }
}

impl fmt::Display for ContentProcessFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("content processing failed")
    }
}

impl Error for ContentProcessFailure {}
