use std::{fmt, sync::Arc, time::Duration};

use tokio::sync::{Notify, watch};
use uuid::Uuid;

use crate::{content::provider::ProviderSecretKeyring, feeds::FeedRepository, setup::SetupService};

use super::{
    BackupError, BackupErrorKind, BackupRepository, BackupTransport, ProductionBackupTransport,
};

const POLL_INTERVAL: Duration = Duration::from_secs(30);
const MAX_JOBS_PER_PASS: usize = 4;

pub struct BackupRuntime {
    setup: SetupService,
    keyring: Option<Arc<ProviderSecretKeyring>>,
    transport: Arc<dyn BackupTransport>,
    owner: String,
    notify: Arc<Notify>,
    shutdown_rx: watch::Receiver<bool>,
}

#[derive(Clone)]
pub struct BackupRuntimeHandle {
    notify: Arc<Notify>,
    shutdown_tx: watch::Sender<bool>,
}

impl BackupRuntimeHandle {
    #[must_use]
    pub fn inert() -> Self {
        let (shutdown_tx, _) = watch::channel(false);
        Self {
            notify: Arc::new(Notify::new()),
            shutdown_tx,
        }
    }

    pub fn notify(&self) {
        self.notify.notify_one();
    }

    pub fn shutdown(&self) {
        let _ = self.shutdown_tx.send(true);
        self.notify.notify_waiters();
    }
}

impl BackupRuntime {
    pub fn production(
        setup: SetupService,
        keyring: Option<Arc<ProviderSecretKeyring>>,
    ) -> Result<(Self, BackupRuntimeHandle), BackupError> {
        Self::new(setup, keyring, Arc::new(ProductionBackupTransport::new()?))
    }

    pub fn new(
        setup: SetupService,
        keyring: Option<Arc<ProviderSecretKeyring>>,
        transport: Arc<dyn BackupTransport>,
    ) -> Result<(Self, BackupRuntimeHandle), BackupError> {
        let notify = Arc::new(Notify::new());
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let handle = BackupRuntimeHandle {
            notify: Arc::clone(&notify),
            shutdown_tx,
        };
        Ok((
            Self {
                setup,
                keyring,
                transport,
                owner: format!("backup-{}", Uuid::new_v4()),
                notify,
                shutdown_rx,
            },
            handle,
        ))
    }

    pub async fn run(mut self) -> Result<(), BackupRuntimeError> {
        loop {
            if *self.shutdown_rx.borrow() {
                return Ok(());
            }
            if let Err(error) = self.run_pass().await {
                tracing::error!(code = error.public_code(), "backup runtime pass failed");
            }
            tokio::select! {
                _ = self.notify.notified() => {}
                _ = tokio::time::sleep(POLL_INTERVAL) => {}
                changed = self.shutdown_rx.changed() => {
                    if changed.is_err() || *self.shutdown_rx.borrow() {
                        return Ok(());
                    }
                }
            }
        }
    }

    async fn run_pass(&self) -> Result<(), BackupError> {
        let Ok(database) = self.setup.database() else {
            return Ok(());
        };
        let repository = BackupRepository::new(database.clone(), self.keyring.clone());
        repository.enqueue_due_schedules().await?;
        for _ in 0..MAX_JOBS_PER_PASS {
            let Some(claim) = repository.claim_next(&self.owner).await? else {
                break;
            };
            let opml = match FeedRepository::new(database.clone())
                .export_opml(&claim.user_id)
                .await
            {
                Ok(opml) => opml,
                Err(_) => {
                    let error = BackupError::new(BackupErrorKind::ExportFailed);
                    repository.fail_pending_targets(&claim, &error).await?;
                    let _ = repository.finish_job(&claim).await?;
                    continue;
                }
            };
            let targets = match repository.pending_execution_targets(&claim).await {
                Ok(targets) => targets,
                Err(error) => {
                    repository.fail_pending_targets(&claim, &error).await?;
                    let _ = repository.finish_job(&claim).await?;
                    continue;
                }
            };
            for target in targets {
                repository.heartbeat(&claim).await?;
                repository
                    .mark_target_running(&claim, &target.target_result_id)
                    .await?;
                let result = self
                    .transport
                    .upload(&target, &opml)
                    .await
                    .map(|()| opml.len() as u64);
                repository
                    .complete_target(&claim, &target.target_result_id, result)
                    .await?;
            }
            let _ = repository.finish_job(&claim).await?;
        }
        let _ = repository.cleanup_history().await?;
        Ok(())
    }
}

#[derive(Debug)]
pub struct BackupRuntimeError;

impl fmt::Display for BackupRuntimeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("backup runtime failed")
    }
}

impl std::error::Error for BackupRuntimeError {}
