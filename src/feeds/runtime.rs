use std::{future::Future, sync::Arc, time::Duration};

use sea_orm::DatabaseConnection;
use tokio::{
    sync::{Notify, watch},
    task::JoinSet,
    time::Instant,
};
use uuid::Uuid;

use crate::setup::SetupService;

use super::{
    ClaimRequest, FeedExecutor, FeedRepository, FeedServiceError, FeedTransport,
    RefreshRepositoryError,
};

const LANE_COUNT: usize = 2;
const LEASE_DURATION: Duration = Duration::from_secs(60);
const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(20);
const IDLE_POLL_INTERVAL: Duration = Duration::from_secs(1);
const SCHEDULE_SCAN_INTERVAL: Duration = Duration::from_secs(30);
const SHUTDOWN_WAIT: Duration = Duration::from_secs(30);
const MAINTENANCE_LIMIT: u16 = 100;

type ExecutorFactory<T> =
    Arc<dyn Fn(DatabaseConnection) -> Result<Arc<FeedExecutor<T>>, FeedServiceError> + Send + Sync>;

#[derive(Clone)]
pub struct FeedRuntimeHandle {
    notify: Arc<Notify>,
    shutdown_tx: watch::Sender<bool>,
}

impl FeedRuntimeHandle {
    pub fn notify(&self) {
        self.notify.notify_waiters();
    }

    pub fn shutdown(&self) {
        let _ = self.shutdown_tx.send(true);
        self.notify.notify_waiters();
    }
}

pub struct FeedRuntime<T: FeedTransport> {
    setup: SetupService,
    executor_factory: ExecutorFactory<T>,
    notify: Arc<Notify>,
    shutdown_rx: watch::Receiver<bool>,
    #[cfg(debug_assertions)]
    terminal_ready_hook: Option<Arc<TerminalReadyHook>>,
    #[cfg(debug_assertions)]
    lane_future_factory: Option<Arc<LaneFutureFactory>>,
}

#[cfg(debug_assertions)]
struct TerminalReadyHook {
    ready: Arc<Notify>,
    release: Arc<Notify>,
    heartbeat_attempts: Arc<std::sync::atomic::AtomicUsize>,
}

#[cfg(debug_assertions)]
type LaneFuture =
    std::pin::Pin<Box<dyn Future<Output = Result<(), FeedServiceError>> + Send + 'static>>;

#[cfg(debug_assertions)]
type LaneFutureFactory = dyn Fn(usize) -> LaneFuture + Send + Sync;

impl<T> FeedRuntime<T>
where
    T: FeedTransport + 'static,
{
    pub fn new<F>(setup: SetupService, executor_factory: F) -> (Self, FeedRuntimeHandle)
    where
        F: Fn(DatabaseConnection) -> Result<Arc<FeedExecutor<T>>, FeedServiceError>
            + Send
            + Sync
            + 'static,
    {
        let notify = Arc::new(Notify::new());
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        (
            Self {
                setup,
                executor_factory: Arc::new(executor_factory),
                notify: notify.clone(),
                shutdown_rx,
                #[cfg(debug_assertions)]
                terminal_ready_hook: None,
                #[cfg(debug_assertions)]
                lane_future_factory: None,
            },
            FeedRuntimeHandle {
                notify,
                shutdown_tx,
            },
        )
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

    #[cfg(debug_assertions)]
    #[doc(hidden)]
    #[must_use]
    pub fn with_lane_future_factory<F>(mut self, factory: F) -> Self
    where
        F: Fn(usize) -> LaneFuture + Send + Sync + 'static,
    {
        self.lane_future_factory = Some(Arc::new(factory));
        self
    }

    pub async fn run(mut self) -> Result<(), FeedServiceError> {
        let database = loop {
            if *self.shutdown_rx.borrow() {
                return Ok(());
            }
            if self.setup.is_ready()
                && let Ok(database) = self.setup.database()
            {
                break database;
            }
            tokio::select! {
                _ = self.notify.notified() => {}
                _ = tokio::time::sleep(Duration::from_secs(1)) => {}
                changed = self.shutdown_rx.changed() => {
                    if changed.is_err() || *self.shutdown_rx.borrow() {
                        return Ok(());
                    }
                }
            }
        };
        let repository = FeedRepository::new(database.clone());
        let executor = (self.executor_factory)(database)?;
        self.run_lanes(repository, executor).await
    }

    async fn run_lanes(
        &mut self,
        repository: FeedRepository,
        executor: Arc<FeedExecutor<T>>,
    ) -> Result<(), FeedServiceError> {
        let runtime_id = Uuid::new_v4();
        let mut lanes = JoinSet::new();
        for lane_index in 0..LANE_COUNT {
            #[cfg(debug_assertions)]
            if let Some(factory) = self.lane_future_factory.as_ref() {
                lanes.spawn(factory(lane_index));
                continue;
            }
            lanes.spawn(run_lane(
                lane_index,
                format!("feed-runtime-{runtime_id}-lane-{lane_index}"),
                repository.clone(),
                executor.clone(),
                self.notify.clone(),
                self.shutdown_rx.clone(),
                #[cfg(debug_assertions)]
                self.terminal_ready_hook.clone(),
            ));
        }

        loop {
            if lanes.is_empty() {
                abort_and_drain(&mut lanes).await;
                return Err(FeedServiceError::RuntimeSupervision);
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
                            tracing::error!("feed runtime lane returned unexpectedly");
                        }
                        Some(Ok(Err(error))) => {
                            tracing::error!(?error, "feed runtime lane failed unexpectedly");
                        }
                        Some(Err(error)) => {
                            tracing::error!(
                                cancelled = error.is_cancelled(),
                                panicked = error.is_panic(),
                                "feed runtime lane task failed unexpectedly"
                            );
                        }
                        None => {
                            tracing::error!("all feed runtime lanes disappeared unexpectedly");
                        }
                    }
                    abort_and_drain(&mut lanes).await;
                    return Err(FeedServiceError::RuntimeSupervision);
                }
            }
        }

        self.notify.notify_waiters();
        let drained = tokio::time::timeout(SHUTDOWN_WAIT, async {
            while let Some(result) = lanes.join_next().await {
                if let Err(error) = result {
                    tracing::error!(?error, "feed runtime lane terminated unexpectedly");
                }
            }
        })
        .await;
        if drained.is_err() {
            lanes.abort_all();
            while lanes.join_next().await.is_some() {}
        }
        Ok(())
    }
}

async fn abort_and_drain<T: 'static>(tasks: &mut JoinSet<T>) {
    tasks.abort_all();
    while tasks.join_next().await.is_some() {}
}

async fn run_lane<T>(
    lane_index: usize,
    owner: String,
    repository: FeedRepository,
    executor: Arc<FeedExecutor<T>>,
    notify: Arc<Notify>,
    mut shutdown_rx: watch::Receiver<bool>,
    #[cfg(debug_assertions)] terminal_ready_hook: Option<Arc<TerminalReadyHook>>,
) -> Result<(), FeedServiceError>
where
    T: FeedTransport + 'static,
{
    let scheduler_lane = lane_index == 0;
    let mut next_schedule_scan = Instant::now();
    loop {
        if *shutdown_rx.borrow() {
            return Ok(());
        }

        match repository.recover_expired_runs(MAINTENANCE_LIMIT).await {
            Ok(queued) if !queued.is_empty() => notify.notify_waiters(),
            Ok(_) => {}
            Err(error) => {
                tracing::warn!(?error, "feed runtime stale recovery failed");
            }
        }
        if scheduler_lane && Instant::now() >= next_schedule_scan {
            match repository.enqueue_due_scheduled(MAINTENANCE_LIMIT).await {
                Ok(queued) if queued > 0 => notify.notify_waiters(),
                Ok(_) => {}
                Err(error) => {
                    tracing::warn!(?error, "feed runtime scheduled enqueue failed");
                }
            }
            next_schedule_scan = Instant::now() + SCHEDULE_SCAN_INTERVAL;
        }

        if *shutdown_rx.borrow() {
            return Ok(());
        }
        match repository
            .claim_due(ClaimRequest {
                owner: owner.clone(),
                lease_duration: LEASE_DURATION,
            })
            .await
        {
            Ok(Some(claim)) => {
                notify.notify_one();
                execute_with_heartbeat(
                    &repository,
                    executor.clone(),
                    claim,
                    notify.as_ref(),
                    #[cfg(debug_assertions)]
                    terminal_ready_hook.clone(),
                )
                .await;
            }
            Ok(None) => {
                wait_until_woken(&notify, &mut shutdown_rx).await;
            }
            Err(error) => {
                tracing::warn!(?error, "feed runtime claim failed");
                wait_until_woken(&notify, &mut shutdown_rx).await;
            }
        }
    }
}

async fn execute_with_heartbeat<T>(
    repository: &FeedRepository,
    executor: Arc<FeedExecutor<T>>,
    claim: super::RefreshClaim,
    notify: &Notify,
    #[cfg(debug_assertions)] terminal_ready_hook: Option<Arc<TerminalReadyHook>>,
) where
    T: FeedTransport + 'static,
{
    #[cfg(debug_assertions)]
    if let Some(hook) = terminal_ready_hook {
        let attempt_executor = executor.clone();
        let attempt_claim = claim.clone();
        let attempt_ready = hook.ready.clone();
        let task = tokio::spawn(async move {
            let result = attempt_executor.execute_claim(attempt_claim).await;
            attempt_ready.notify_one();
            result
        });
        let attempt = async move {
            match task.await {
                Ok(result) => result,
                Err(error) => {
                    tracing::error!(?error, "feed runtime observed attempt task failure");
                    Err(FeedServiceError::RuntimeSupervision)
                }
            }
        };
        coordinate_attempt(repository, attempt, claim, notify, Some(hook), true).await;
        return;
    }

    let attempt_claim = claim.clone();
    let attempt = async move { executor.execute_claim(attempt_claim).await };
    coordinate_attempt(
        repository,
        attempt,
        claim,
        notify,
        #[cfg(debug_assertions)]
        None,
        #[cfg(debug_assertions)]
        false,
    )
    .await;
}

async fn coordinate_attempt<A>(
    repository: &FeedRepository,
    attempt: A,
    mut claim: super::RefreshClaim,
    notify: &Notify,
    #[cfg(debug_assertions)] terminal_ready_hook: Option<Arc<TerminalReadyHook>>,
    #[cfg(debug_assertions)] mut gate_first_select: bool,
) where
    A: Future<Output = Result<super::RefreshDto, FeedServiceError>>,
{
    tokio::pin!(attempt);
    loop {
        let heartbeat = tokio::time::sleep(HEARTBEAT_INTERVAL);
        tokio::pin!(heartbeat);
        #[cfg(debug_assertions)]
        if gate_first_select {
            if let Some(hook) = terminal_ready_hook.as_ref() {
                hook.release.notified().await;
            }
            gate_first_select = false;
        }
        tokio::select! {
            biased;
            result = &mut attempt => {
                if let Err(error) = result {
                    let lease_lost = matches!(
                        &error,
                        FeedServiceError::RefreshRepository(RefreshRepositoryError::LeaseLost)
                    );
                    let database_error = matches!(
                        &error,
                        FeedServiceError::RefreshRepository(RefreshRepositoryError::Database(_))
                    );
                    tracing::warn!(?error, database_error, "feed runtime attempt failed");
                    if lease_lost {
                        recover_after_lease_loss(repository, notify).await;
                    }
                }
                return;
            }
            () = &mut heartbeat => {
                #[cfg(debug_assertions)]
                if let Some(hook) = terminal_ready_hook.as_ref() {
                    hook.heartbeat_attempts
                        .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                }
                match repository.extend_lease(&claim, LEASE_DURATION).await {
                    Ok(extended) => claim = extended,
                    Err(error) => {
                        let lease_lost = matches!(error, RefreshRepositoryError::LeaseLost);
                        tracing::warn!(?error, "feed runtime heartbeat failed");
                        if lease_lost {
                            recover_after_lease_loss(repository, notify).await;
                        }
                        return;
                    }
                }
            }
        }
    }
}

async fn recover_after_lease_loss(repository: &FeedRepository, notify: &Notify) {
    match repository.recover_expired_runs(MAINTENANCE_LIMIT).await {
        Ok(queued) if !queued.is_empty() => notify.notify_waiters(),
        Ok(_) => {}
        Err(error) => tracing::warn!(?error, "feed runtime lease-loss recovery failed"),
    }
}

async fn wait_until_woken(notify: &Notify, shutdown_rx: &mut watch::Receiver<bool>) {
    tokio::select! {
        _ = notify.notified() => {}
        _ = tokio::time::sleep(IDLE_POLL_INTERVAL) => {}
        _ = shutdown_rx.changed() => {}
    }
}
