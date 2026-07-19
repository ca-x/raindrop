use std::{error::Error, fmt, future::Future, pin::Pin, sync::Arc};

use secrecy::SecretString;
use tokio::{sync::watch, task::JoinError};

use crate::{
    config::FeedRetentionConfig,
    content::worker::{ContentRuntimeHandle, ContentWorkerError, ProductionContentRuntime},
    feeds::{
        FeedExecutor, FeedRepository, FeedRetentionPolicy, FeedRuntime, FeedRuntimeHandle,
        FeedServiceError, FeedUrlPolicy, HttpFeedTransport,
    },
    setup::SetupService,
};

type FeedFuture = Pin<Box<dyn Future<Output = Result<(), FeedServiceError>> + Send + 'static>>;
type ContentFuture = Pin<Box<dyn Future<Output = Result<(), ContentWorkerError>> + Send + 'static>>;

pub struct BackgroundRuntime {
    feed: FeedFuture,
    content: ContentFuture,
    handle: BackgroundRuntimeHandle,
    shutdown_rx: watch::Receiver<bool>,
}

#[derive(Clone)]
pub struct BackgroundRuntimeHandle {
    feed: FeedRuntimeHandle,
    content: ContentRuntimeHandle,
    shutdown_tx: watch::Sender<bool>,
}

impl BackgroundRuntimeHandle {
    #[must_use]
    pub fn feed(&self) -> FeedRuntimeHandle {
        self.feed.clone()
    }

    #[must_use]
    pub fn content(&self) -> ContentRuntimeHandle {
        self.content.clone()
    }

    pub fn shutdown(&self) {
        let _ = self.shutdown_tx.send(true);
        self.feed.shutdown();
        self.content.shutdown();
    }
}

impl BackgroundRuntime {
    pub fn production(
        setup: SetupService,
        retention: FeedRetentionConfig,
        provider_secret_keys: Vec<SecretString>,
    ) -> Result<(Self, BackgroundRuntimeHandle), BackgroundRuntimeError> {
        let (feed, feed_handle) = FeedRuntime::new(setup.clone(), |database| {
            let url_policy = FeedUrlPolicy::new(false);
            let transport = HttpFeedTransport::new(url_policy)
                .map_err(FeedServiceError::ExecutorInitialization)?;
            Ok(Arc::new(FeedExecutor::new(
                FeedRepository::new(database),
                url_policy,
                transport,
            )))
        });
        let feed = feed.with_retention_policy(FeedRetentionPolicy::new(retention.orphan_grace));
        let (content, content_handle) = ProductionContentRuntime::new(setup, provider_secret_keys)
            .map_err(BackgroundRuntimeError::content)?;
        Ok(Self::from_futures(
            feed.run(),
            feed_handle,
            content.run(),
            content_handle,
        ))
    }

    fn from_futures<Feed, Content>(
        feed: Feed,
        feed_handle: FeedRuntimeHandle,
        content: Content,
        content_handle: ContentRuntimeHandle,
    ) -> (Self, BackgroundRuntimeHandle)
    where
        Feed: Future<Output = Result<(), FeedServiceError>> + Send + 'static,
        Content: Future<Output = Result<(), ContentWorkerError>> + Send + 'static,
    {
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let handle = BackgroundRuntimeHandle {
            feed: feed_handle,
            content: content_handle,
            shutdown_tx,
        };
        (
            Self {
                feed: Box::pin(feed),
                content: Box::pin(content),
                handle: handle.clone(),
                shutdown_rx,
            },
            handle,
        )
    }

    #[cfg(test)]
    fn controlled<Feed, Content>(
        feed: Feed,
        feed_handle: FeedRuntimeHandle,
        content: Content,
        content_handle: ContentRuntimeHandle,
    ) -> (Self, BackgroundRuntimeHandle)
    where
        Feed: Future<Output = Result<(), FeedServiceError>> + Send + 'static,
        Content: Future<Output = Result<(), ContentWorkerError>> + Send + 'static,
    {
        Self::from_futures(feed, feed_handle, content, content_handle)
    }

    pub async fn run(self) -> Result<(), BackgroundRuntimeError> {
        let Self {
            feed,
            content,
            handle,
            shutdown_rx,
        } = self;
        let mut feed_task = tokio::spawn(feed);
        let mut content_task = tokio::spawn(content);

        enum FirstCompletion {
            Feed(Result<Result<(), FeedServiceError>, JoinError>),
            Content(Result<Result<(), ContentWorkerError>, JoinError>),
        }

        let first = tokio::select! {
            biased;
            result = &mut feed_task => FirstCompletion::Feed(result),
            result = &mut content_task => FirstCompletion::Content(result),
        };
        let expected = *shutdown_rx.borrow();
        if !expected {
            handle.shutdown();
        }

        match first {
            FirstCompletion::Feed(feed_result) => {
                let content_result = content_task.await;
                if expected {
                    expected_feed_result(feed_result)?;
                    expected_content_result(content_result)
                } else {
                    log_secondary_content_failure(&content_result);
                    unexpected_feed_result(feed_result)
                }
            }
            FirstCompletion::Content(content_result) => {
                let feed_result = feed_task.await;
                if expected {
                    expected_content_result(content_result)?;
                    expected_feed_result(feed_result)
                } else {
                    log_secondary_feed_failure(&feed_result);
                    unexpected_content_result(content_result)
                }
            }
        }
    }
}

fn expected_feed_result(
    result: Result<Result<(), FeedServiceError>, JoinError>,
) -> Result<(), BackgroundRuntimeError> {
    match result {
        Ok(Ok(())) => Ok(()),
        Ok(Err(error)) => Err(BackgroundRuntimeError::feed(error)),
        Err(_) => Err(BackgroundRuntimeError::supervision()),
    }
}

fn expected_content_result(
    result: Result<Result<(), ContentWorkerError>, JoinError>,
) -> Result<(), BackgroundRuntimeError> {
    match result {
        Ok(Ok(())) => Ok(()),
        Ok(Err(error)) => Err(BackgroundRuntimeError::content(error)),
        Err(_) => Err(BackgroundRuntimeError::supervision()),
    }
}

fn unexpected_feed_result(
    result: Result<Result<(), FeedServiceError>, JoinError>,
) -> Result<(), BackgroundRuntimeError> {
    match result {
        Ok(Ok(())) => Err(BackgroundRuntimeError::new(
            BackgroundRuntimeErrorKind::FeedStoppedUnexpectedly,
        )),
        Ok(Err(error)) => Err(BackgroundRuntimeError::feed(error)),
        Err(_) => Err(BackgroundRuntimeError::supervision()),
    }
}

fn unexpected_content_result(
    result: Result<Result<(), ContentWorkerError>, JoinError>,
) -> Result<(), BackgroundRuntimeError> {
    match result {
        Ok(Ok(())) => Err(BackgroundRuntimeError::new(
            BackgroundRuntimeErrorKind::ContentStoppedUnexpectedly,
        )),
        Ok(Err(error)) => Err(BackgroundRuntimeError::content(error)),
        Err(_) => Err(BackgroundRuntimeError::supervision()),
    }
}

fn log_secondary_feed_failure(result: &Result<Result<(), FeedServiceError>, JoinError>) {
    match result {
        Ok(Ok(())) => {}
        Ok(Err(_)) => tracing::error!("Feed runtime failed while sibling shutdown was in progress"),
        Err(_) => {
            tracing::error!("Feed runtime task failed while sibling shutdown was in progress")
        }
    }
}

fn log_secondary_content_failure(result: &Result<Result<(), ContentWorkerError>, JoinError>) {
    match result {
        Ok(Ok(())) => {}
        Ok(Err(_)) => {
            tracing::error!("Content runtime failed while sibling shutdown was in progress");
        }
        Err(_) => {
            tracing::error!("Content runtime task failed while sibling shutdown was in progress");
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BackgroundRuntimeErrorKind {
    FeedFailed,
    ContentFailed,
    FeedStoppedUnexpectedly,
    ContentStoppedUnexpectedly,
    SupervisionFailed,
}

enum BackgroundRuntimeErrorSource {
    Feed(FeedServiceError),
    Content(ContentWorkerError),
}

pub struct BackgroundRuntimeError {
    kind: BackgroundRuntimeErrorKind,
    source: Option<BackgroundRuntimeErrorSource>,
}

impl BackgroundRuntimeError {
    const fn new(kind: BackgroundRuntimeErrorKind) -> Self {
        Self { kind, source: None }
    }

    fn feed(error: FeedServiceError) -> Self {
        Self {
            kind: BackgroundRuntimeErrorKind::FeedFailed,
            source: Some(BackgroundRuntimeErrorSource::Feed(error)),
        }
    }

    fn content(error: ContentWorkerError) -> Self {
        Self {
            kind: BackgroundRuntimeErrorKind::ContentFailed,
            source: Some(BackgroundRuntimeErrorSource::Content(error)),
        }
    }

    const fn supervision() -> Self {
        Self::new(BackgroundRuntimeErrorKind::SupervisionFailed)
    }

    #[must_use]
    pub const fn kind(&self) -> BackgroundRuntimeErrorKind {
        self.kind
    }
}

impl fmt::Debug for BackgroundRuntimeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("BackgroundRuntimeError")
            .field("kind", &self.kind)
            .finish()
    }
}

impl fmt::Display for BackgroundRuntimeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self.kind {
            BackgroundRuntimeErrorKind::FeedFailed => "background Feed runtime failed",
            BackgroundRuntimeErrorKind::ContentFailed => "background Content runtime failed",
            BackgroundRuntimeErrorKind::FeedStoppedUnexpectedly => {
                "background Feed runtime stopped unexpectedly"
            }
            BackgroundRuntimeErrorKind::ContentStoppedUnexpectedly => {
                "background Content runtime stopped unexpectedly"
            }
            BackgroundRuntimeErrorKind::SupervisionFailed => {
                "background runtime supervision failed"
            }
        })
    }
}

impl Error for BackgroundRuntimeError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self.source.as_ref() {
            Some(BackgroundRuntimeErrorSource::Feed(error)) => Some(error),
            Some(BackgroundRuntimeErrorSource::Content(error)) => Some(error),
            None => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{
        sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        },
        time::Duration,
    };

    use secrecy::SecretString;

    use crate::{
        config::FeedRetentionConfig,
        content::worker::ContentRuntimeHandle,
        feeds::{FeedRuntime, FeedServiceError, HttpFeedTransport},
        setup::SetupService,
    };

    use super::{BackgroundRuntime, BackgroundRuntimeErrorKind};

    #[tokio::test]
    async fn production_group_stays_inert_while_setup_is_required() {
        let data = tempfile::tempdir().expect("temporary background runtime directory");
        let setup = SetupService::required(
            data.path(),
            SecretString::from("background-production-token"),
            None,
        );
        let (runtime, handle) =
            BackgroundRuntime::production(setup, FeedRetentionConfig::default(), Vec::new())
                .expect("production background composition");

        handle.shutdown();

        runtime
            .run()
            .await
            .expect("pre-start production background shutdown should succeed");
    }

    #[tokio::test]
    async fn group_shutdown_joins_both_children() {
        let data = tempfile::tempdir().expect("temporary background runtime directory");
        let setup = SetupService::required(
            data.path(),
            SecretString::from("background-setup-token"),
            None,
        );
        let (feed, feed_handle) =
            FeedRuntime::<HttpFeedTransport>::new(setup, |_| Err(FeedServiceError::CorruptFeed));
        let (content_handle, _, mut content_shutdown) = ContentRuntimeHandle::control();
        let feed_stopped = Arc::new(AtomicBool::new(false));
        let observed_feed_stopped = Arc::clone(&feed_stopped);
        let content_stopped = Arc::new(AtomicBool::new(false));
        let observed_content_stopped = Arc::clone(&content_stopped);
        let (runtime, handle) = BackgroundRuntime::controlled(
            async move {
                let result = feed.run().await;
                observed_feed_stopped.store(true, Ordering::SeqCst);
                result
            },
            feed_handle,
            async move {
                while !*content_shutdown.borrow() {
                    if content_shutdown.changed().await.is_err() {
                        break;
                    }
                }
                observed_content_stopped.store(true, Ordering::SeqCst);
                Ok(())
            },
            content_handle,
        );
        let task = tokio::spawn(runtime.run());

        handle.shutdown();

        tokio::time::timeout(Duration::from_secs(2), task)
            .await
            .expect("group shutdown should finish promptly")
            .expect("background task should join")
            .expect("normal group shutdown should succeed");
        assert!(feed_stopped.load(Ordering::SeqCst));
        assert!(content_stopped.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn unexpected_feed_completion_stops_and_joins_content() {
        let data = tempfile::tempdir().expect("temporary background runtime directory");
        let setup = SetupService::required(
            data.path(),
            SecretString::from("background-setup-token"),
            None,
        );
        let (_feed, feed_handle) =
            FeedRuntime::<HttpFeedTransport>::new(setup, |_| Err(FeedServiceError::CorruptFeed));
        let (content_handle, _, mut content_shutdown) = ContentRuntimeHandle::control();
        let content_stopped = Arc::new(AtomicBool::new(false));
        let observed_content_stopped = Arc::clone(&content_stopped);
        let (runtime, _) = BackgroundRuntime::controlled(
            async { Ok(()) },
            feed_handle,
            async move {
                while !*content_shutdown.borrow() {
                    if content_shutdown.changed().await.is_err() {
                        break;
                    }
                }
                observed_content_stopped.store(true, Ordering::SeqCst);
                Ok(())
            },
            content_handle,
        );

        let error = runtime
            .run()
            .await
            .expect_err("unexpected Feed completion must fail supervision");
        assert_eq!(
            error.kind(),
            BackgroundRuntimeErrorKind::FeedStoppedUnexpectedly
        );
        assert!(content_stopped.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn unexpected_content_completion_stops_and_joins_feed() {
        let data = tempfile::tempdir().expect("temporary background runtime directory");
        let setup = SetupService::required(
            data.path(),
            SecretString::from("background-setup-token"),
            None,
        );
        let (feed, feed_handle) =
            FeedRuntime::<HttpFeedTransport>::new(setup, |_| Err(FeedServiceError::CorruptFeed));
        let (content_handle, _, _) = ContentRuntimeHandle::control();
        let feed_stopped = Arc::new(AtomicBool::new(false));
        let observed_feed_stopped = Arc::clone(&feed_stopped);
        let (runtime, _) = BackgroundRuntime::controlled(
            async move {
                let result = feed.run().await;
                observed_feed_stopped.store(true, Ordering::SeqCst);
                result
            },
            feed_handle,
            async { Ok(()) },
            content_handle,
        );

        let error = runtime
            .run()
            .await
            .expect_err("unexpected Content completion must fail supervision");
        assert_eq!(
            error.kind(),
            BackgroundRuntimeErrorKind::ContentStoppedUnexpectedly
        );
        assert!(feed_stopped.load(Ordering::SeqCst));
    }
}
