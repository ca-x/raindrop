use std::{sync::Arc, time::Duration};

use tokio::{
    sync::{Notify, oneshot, watch},
    task::{JoinHandle, JoinSet},
};
use uuid::Uuid;

use crate::content::jobs::{
    ClaimContentJob, ClaimOutcome, ContentJobClaim, ContentRepository, ContentRepositoryErrorKind,
};

use super::{ContentProcessor, ContentWorkerError, ContentWorkerErrorKind};

const LANE_COUNT: usize = 8;
const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(10);
const IDLE_POLL_INTERVAL: Duration = Duration::from_secs(1);
const SHUTDOWN_WAIT: Duration = Duration::from_secs(30);

#[derive(Clone)]
pub struct ContentWorker {
    repository: Arc<ContentRepository>,
    processor: Arc<dyn ContentProcessor>,
    #[cfg(debug_assertions)]
    terminal_ready_hook: Option<Arc<TerminalReadyHook>>,
}

#[cfg(debug_assertions)]
struct TerminalReadyHook {
    ready: Arc<Notify>,
    release: Arc<Notify>,
    heartbeat_attempts: Arc<std::sync::atomic::AtomicUsize>,
}

impl ContentWorker {
    #[must_use]
    pub fn new(repository: Arc<ContentRepository>, processor: Arc<dyn ContentProcessor>) -> Self {
        Self {
            repository,
            processor,
            #[cfg(debug_assertions)]
            terminal_ready_hook: None,
        }
    }

    #[cfg(debug_assertions)]
    #[doc(hidden)]
    #[must_use]
    pub fn with_terminal_ready_hook(
        mut self,
        ready: Arc<Notify>,
        release: Arc<Notify>,
        heartbeat_attempts: Arc<std::sync::atomic::AtomicUsize>,
    ) -> Self {
        self.terminal_ready_hook = Some(Arc::new(TerminalReadyHook {
            ready,
            release,
            heartbeat_attempts,
        }));
        self
    }

    pub async fn run_claim(&self, claim: ContentJobClaim) -> Result<(), ContentWorkerError> {
        let lease = match self.repository.heartbeat(&claim).await {
            Ok(lease) => lease,
            Err(error) => return map_repository_result(error.kind()),
        };
        let mut heartbeat = HeartbeatTask::start(
            Arc::clone(&self.repository),
            claim.clone(),
            #[cfg(debug_assertions)]
            self.terminal_ready_hook.clone(),
        );
        let mut processing = Box::pin(self.processor.process(&claim, lease.remaining_attempt()));

        enum ClaimRace<T> {
            Processed(T),
            HeartbeatEnded,
        }

        let raced = tokio::select! {
            result = &mut processing => ClaimRace::Processed(result),
            _ = heartbeat.ended() => ClaimRace::HeartbeatEnded,
        };
        match raced {
            ClaimRace::HeartbeatEnded => {
                drop(processing);
                let heartbeat_result = heartbeat.stop_and_join().await?;
                match heartbeat_result {
                    Ok(()) => Err(ContentWorkerError::new(
                        ContentWorkerErrorKind::RuntimeUnavailable,
                    )),
                    Err(kind) => map_repository_result(kind),
                }
            }
            ClaimRace::Processed(result) => {
                drop(processing);
                let heartbeat_result = heartbeat.stop_and_join().await?;
                if let Err(kind) = heartbeat_result {
                    return map_repository_result(kind);
                }

                #[cfg(debug_assertions)]
                if let Some(hook) = &self.terminal_ready_hook {
                    hook.ready.notify_one();
                    hook.release.notified().await;
                }

                let terminal = match result {
                    Ok(success) => {
                        let (artifact, usage) = success.into_parts();
                        self.repository
                            .complete_success(&claim, artifact, usage)
                            .await
                            .map(|_| ())
                    }
                    Err(failure) => self
                        .repository
                        .complete_failure(&claim, failure.into_attempt_failure())
                        .await
                        .map(|_| ()),
                };
                match terminal {
                    Ok(()) => Ok(()),
                    Err(error) => map_repository_result(error.kind()),
                }
            }
        }
    }
}

struct HeartbeatTask {
    stop_tx: watch::Sender<bool>,
    ended_rx: oneshot::Receiver<()>,
    handle: Option<JoinHandle<Result<(), ContentRepositoryErrorKind>>>,
}

impl HeartbeatTask {
    fn start(
        repository: Arc<ContentRepository>,
        claim: ContentJobClaim,
        #[cfg(debug_assertions)] terminal_ready_hook: Option<Arc<TerminalReadyHook>>,
    ) -> Self {
        let (stop_tx, mut stop_rx) = watch::channel(false);
        let (ended_tx, ended_rx) = oneshot::channel();
        let handle = tokio::spawn(async move {
            loop {
                tokio::select! {
                    changed = stop_rx.changed() => {
                        if changed.is_err() || *stop_rx.borrow() {
                            return Ok(());
                        }
                    }
                    _ = tokio::time::sleep(HEARTBEAT_INTERVAL) => {
                        #[cfg(debug_assertions)]
                        if let Some(hook) = &terminal_ready_hook {
                            hook.heartbeat_attempts.fetch_add(
                                1,
                                std::sync::atomic::Ordering::SeqCst,
                            );
                        }
                        if let Err(error) = repository.heartbeat(&claim).await {
                            let _ = ended_tx.send(());
                            return Err(error.kind());
                        }
                    }
                }
            }
        });
        Self {
            stop_tx,
            ended_rx,
            handle: Some(handle),
        }
    }

    fn ended(&mut self) -> &mut oneshot::Receiver<()> {
        &mut self.ended_rx
    }

    async fn stop_and_join(
        mut self,
    ) -> Result<Result<(), ContentRepositoryErrorKind>, ContentWorkerError> {
        let _ = self.stop_tx.send(true);
        let handle = self
            .handle
            .take()
            .ok_or_else(|| ContentWorkerError::new(ContentWorkerErrorKind::RuntimeUnavailable))?;
        handle
            .await
            .map_err(|_| ContentWorkerError::new(ContentWorkerErrorKind::RuntimeUnavailable))
    }
}

impl Drop for HeartbeatTask {
    fn drop(&mut self) {
        let _ = self.stop_tx.send(true);
        if let Some(handle) = self.handle.take() {
            handle.abort();
        }
    }
}

fn map_repository_result(kind: ContentRepositoryErrorKind) -> Result<(), ContentWorkerError> {
    match kind {
        ContentRepositoryErrorKind::LeaseLost | ContentRepositoryErrorKind::AlreadyCompleted => {
            Ok(())
        }
        ContentRepositoryErrorKind::InvalidInput
        | ContentRepositoryErrorKind::NotFound
        | ContentRepositoryErrorKind::EntryChanged
        | ContentRepositoryErrorKind::IdempotencyConflict
        | ContentRepositoryErrorKind::HashCollision
        | ContentRepositoryErrorKind::NoWork
        | ContentRepositoryErrorKind::UserConcurrencyLimited
        | ContentRepositoryErrorKind::AttemptsExhausted
        | ContentRepositoryErrorKind::ArtifactTooLarge
        | ContentRepositoryErrorKind::ExecutionInputTooLarge
        | ContentRepositoryErrorKind::NonCanonicalJson
        | ContentRepositoryErrorKind::CorruptData
        | ContentRepositoryErrorKind::Database => Err(ContentWorkerError::new(
            ContentWorkerErrorKind::RuntimeUnavailable,
        )),
    }
}

#[derive(Clone)]
pub struct ContentRuntimeHandle {
    notify: Arc<Notify>,
    shutdown_tx: watch::Sender<bool>,
}

impl ContentRuntimeHandle {
    pub fn notify(&self) {
        self.notify.notify_waiters();
    }

    pub fn shutdown(&self) {
        let _ = self.shutdown_tx.send(true);
        self.notify.notify_waiters();
    }
}

pub struct ContentRuntime {
    worker: ContentWorker,
    notify: Arc<Notify>,
    shutdown_rx: watch::Receiver<bool>,
}

impl ContentRuntime {
    #[must_use]
    pub fn new(
        repository: Arc<ContentRepository>,
        processor: Arc<dyn ContentProcessor>,
    ) -> (Self, ContentRuntimeHandle) {
        let notify = Arc::new(Notify::new());
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        (
            Self {
                worker: ContentWorker::new(repository, processor),
                notify: Arc::clone(&notify),
                shutdown_rx,
            },
            ContentRuntimeHandle {
                notify,
                shutdown_tx,
            },
        )
    }

    pub async fn run(mut self) -> Result<(), ContentWorkerError> {
        let instance_id = Uuid::new_v4();
        let mut lanes = JoinSet::new();
        for lane in 0..LANE_COUNT {
            lanes.spawn(run_lane(
                format!("content:{instance_id}:{lane}"),
                self.worker.clone(),
                Arc::clone(&self.notify),
                self.shutdown_rx.clone(),
            ));
        }

        loop {
            if lanes.is_empty() {
                abort_and_drain(&mut lanes).await;
                return Err(ContentWorkerError::new(
                    ContentWorkerErrorKind::SupervisionFailed,
                ));
            }
            tokio::select! {
                changed = self.shutdown_rx.changed() => {
                    if changed.is_err() || *self.shutdown_rx.borrow() {
                        break;
                    }
                }
                result = lanes.join_next() => {
                    if *self.shutdown_rx.borrow() {
                        break;
                    }
                    match result {
                        Some(Ok(Ok(()))) => {
                            tracing::error!("content runtime lane returned unexpectedly");
                        }
                        Some(Ok(Err(error))) => {
                            tracing::error!(?error, "content runtime lane failed unexpectedly");
                        }
                        Some(Err(error)) => {
                            tracing::error!(
                                cancelled = error.is_cancelled(),
                                panicked = error.is_panic(),
                                "content runtime lane task failed unexpectedly"
                            );
                        }
                        None => {
                            tracing::error!("all content runtime lanes disappeared unexpectedly");
                        }
                    }
                    abort_and_drain(&mut lanes).await;
                    return Err(ContentWorkerError::new(
                        ContentWorkerErrorKind::SupervisionFailed,
                    ));
                }
            }
        }

        self.notify.notify_waiters();
        if tokio::time::timeout(SHUTDOWN_WAIT, async {
            while let Some(result) = lanes.join_next().await {
                if let Err(error) = result {
                    tracing::error!(?error, "content runtime lane terminated unexpectedly");
                }
            }
        })
        .await
        .is_err()
        {
            lanes.abort_all();
            while lanes.join_next().await.is_some() {}
        }
        Ok(())
    }
}

async fn run_lane(
    owner: String,
    worker: ContentWorker,
    notify: Arc<Notify>,
    mut shutdown_rx: watch::Receiver<bool>,
) -> Result<(), ContentWorkerError> {
    loop {
        if *shutdown_rx.borrow() {
            return Ok(());
        }
        let claim = worker
            .repository
            .claim_next(ClaimContentJob::new(owner.clone()).map_err(|_| {
                ContentWorkerError::new(ContentWorkerErrorKind::InvalidConfiguration)
            })?)
            .await;
        match claim {
            Ok(ClaimOutcome::Claimed(claim)) => {
                notify.notify_one();
                if let Err(error) = worker.run_claim(claim).await {
                    tracing::warn!(?error, "content job execution failed");
                }
            }
            Ok(ClaimOutcome::RecoveredTerminal(_)) => {
                notify.notify_one();
            }
            Ok(ClaimOutcome::NoWork) => {
                wait_until_woken(&notify, &mut shutdown_rx).await;
            }
            Err(error) => {
                tracing::warn!(?error, "content job claim failed");
                wait_until_woken(&notify, &mut shutdown_rx).await;
            }
        }
    }
}

async fn wait_until_woken(notify: &Notify, shutdown_rx: &mut watch::Receiver<bool>) {
    tokio::select! {
        _ = notify.notified() => {}
        _ = tokio::time::sleep(IDLE_POLL_INTERVAL) => {}
        changed = shutdown_rx.changed() => {
            let _ = changed;
        }
    }
}

async fn abort_and_drain<T: 'static>(tasks: &mut JoinSet<T>) {
    tasks.abort_all();
    while tasks.join_next().await.is_some() {}
}
