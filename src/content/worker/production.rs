use std::{sync::Arc, time::Duration};

use secrecy::SecretString;
use tokio::sync::{Notify, watch};

use crate::{
    content::{
        ai::ProviderAiBroker,
        jobs::ContentRepository,
        provider::{
            HttpsProviderTransport, ProviderClient, ProviderRepository, ProviderSecretKeyring,
        },
    },
    plugins::{
        EmbeddedOfficialAiPlugin, EmbeddedSignatureMode, PluginRegistryErrorKind,
        PluginRegistryRepository,
        runtime::{AiCapabilityBroker, CompiledPlugin, PluginRuntime},
    },
    setup::SetupService,
};

use super::{
    ContentRuntime, ContentRuntimeHandle, ContentWorkerError, ContentWorkerErrorKind,
    OfficialAiProcessor,
};

pub struct ProductionContentRuntime {
    setup: SetupService,
    provider_keyring: Option<ProviderSecretKeyring>,
    bundle: EmbeddedOfficialAiPlugin,
    plugin_runtime: PluginRuntime,
    compiled: Arc<CompiledPlugin>,
    notify: Arc<Notify>,
    shutdown_rx: watch::Receiver<bool>,
}

impl ProductionContentRuntime {
    pub fn new(
        setup: SetupService,
        provider_secret_keys: Vec<SecretString>,
    ) -> Result<(Self, ContentRuntimeHandle), ContentWorkerError> {
        let provider_keyring = if provider_secret_keys.is_empty() {
            None
        } else {
            Some(
                ProviderSecretKeyring::from_entries(&provider_secret_keys)
                    .map_err(|_| invalid_configuration())?,
            )
        };
        let plugin_runtime = PluginRuntime::new()
            .map_err(|_| ContentWorkerError::new(ContentWorkerErrorKind::RuntimeUnavailable))?;
        let bundle = EmbeddedOfficialAiPlugin::load().map_err(|_| invalid_configuration())?;
        match bundle.signature_mode() {
            EmbeddedSignatureMode::Development => {
                tracing::warn!("embedded official AI plugin uses the development signing root");
            }
            EmbeddedSignatureMode::Official => {
                tracing::info!("embedded official AI plugin signature is official");
            }
        }
        let compiled = Arc::new(
            bundle
                .compile(&plugin_runtime)
                .map_err(|_| invalid_configuration())?,
        );
        let (handle, notify, shutdown_rx) = ContentRuntimeHandle::control();
        Ok((
            Self {
                setup,
                provider_keyring,
                bundle,
                plugin_runtime,
                compiled,
                notify,
                shutdown_rx,
            },
            handle,
        ))
    }

    pub async fn run(mut self) -> Result<(), ContentWorkerError> {
        let database = loop {
            if *self.shutdown_rx.borrow() {
                return Ok(());
            }
            if self.setup.is_ready()
                && let Ok(database) = self.setup.database()
            {
                break database;
            }
            if wait_or_shutdown(&self.notify, &mut self.shutdown_rx).await {
                return Ok(());
            }
        };

        let plugin_repository = Arc::new(PluginRegistryRepository::new(database.clone()));
        loop {
            if *self.shutdown_rx.borrow() {
                return Ok(());
            }
            match plugin_repository.sync_bundled(self.bundle.bundle()).await {
                Ok(_) => break,
                Err(error)
                    if matches!(
                        error.kind(),
                        PluginRegistryErrorKind::Database
                            | PluginRegistryErrorKind::RevisionConflict
                    ) =>
                {
                    tracing::warn!(kind = ?error.kind(), "official plugin synchronization will retry");
                    if wait_or_shutdown(&self.notify, &mut self.shutdown_rx).await {
                        return Ok(());
                    }
                }
                Err(_) => return Err(invalid_configuration()),
            }
        }

        let Some(keyring) = self.provider_keyring.take() else {
            tracing::info!(
                "content runtime is disabled because no provider secret keyring is configured"
            );
            while !*self.shutdown_rx.borrow() {
                if self.shutdown_rx.changed().await.is_err() {
                    break;
                }
            }
            return Ok(());
        };
        let provider_repository = Arc::new(ProviderRepository::new(database.clone(), keyring));
        let transport = HttpsProviderTransport::new().map_err(|_| runtime_unavailable())?;
        let client = Arc::new(ProviderClient::new(transport));
        let ai_broker: Arc<dyn AiCapabilityBroker> = Arc::new(ProviderAiBroker::new(
            Arc::clone(&provider_repository),
            client,
        ));
        let content_repository = Arc::new(ContentRepository::new(database));
        let processor = Arc::new(OfficialAiProcessor::new(
            Arc::clone(&content_repository),
            plugin_repository,
            provider_repository,
            self.plugin_runtime,
            self.compiled,
            ai_broker,
        )?);
        ContentRuntime::controlled(content_repository, processor, self.notify, self.shutdown_rx)
            .run()
            .await
    }
}

async fn wait_or_shutdown(notify: &Notify, shutdown_rx: &mut watch::Receiver<bool>) -> bool {
    tokio::select! {
        _ = notify.notified() => false,
        _ = tokio::time::sleep(Duration::from_secs(1)) => false,
        changed = shutdown_rx.changed() => changed.is_err() || *shutdown_rx.borrow(),
    }
}

const fn invalid_configuration() -> ContentWorkerError {
    ContentWorkerError::new(ContentWorkerErrorKind::InvalidConfiguration)
}

const fn runtime_unavailable() -> ContentWorkerError {
    ContentWorkerError::new(ContentWorkerErrorKind::RuntimeUnavailable)
}
