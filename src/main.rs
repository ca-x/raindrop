use std::{
    error::Error,
    future::{Future, IntoFuture},
    path::PathBuf,
    sync::Arc,
};

use anyhow::{Context, Result, anyhow};
use raindrop::{
    app::{AppState, build_router},
    auth::{CreateAdminInput, PasswordService, create_admin},
    background::BackgroundRuntime,
    config::{BootstrapMode, ConfigArgs, SystemEnv, load, new_setup_token},
    content::provider::ProviderSecretKeyring,
    db::{DatabaseConfig, connect, migrate},
    setup::SetupService,
};
use secrecy::ExposeSecret;
use tokio::{net::TcpListener, sync::oneshot, task::JoinHandle};
use tracing::info;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> Result<()> {
    if std::env::args_os()
        .nth(1)
        .is_some_and(|argument| argument == "--version" || argument == "-V")
    {
        println!("raindrop {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }

    raindrop::feeds::install_ring_crypto_provider()
        .context("failed to install the required ring TLS crypto provider")?;

    init_tracing();

    let mut loaded = load(
        &ConfigArgs {
            data_dir: PathBuf::from("data"),
            config_path: None,
        },
        &SystemEnv,
    )
    .context("failed to load Raindrop configuration")?;
    let provider_secret_keys = loaded.runtime.take_provider_secret_keys();
    let provider_keyring = if provider_secret_keys.is_empty() {
        None
    } else {
        Some(Arc::new(
            ProviderSecretKeyring::from_entries(&provider_secret_keys)
                .context("failed to initialize the configured provider secret keyring")?,
        ))
    };
    drop(provider_secret_keys);
    let address = loaded.runtime.bind;
    let data_dir = loaded.runtime.data_dir.clone();
    let public_url = loaded.runtime.public_url.clone();
    let feed_retention = loaded.runtime.feed_retention();
    let database_url = loaded.runtime.database_url;
    let bootstrap_admin = loaded.runtime.bootstrap_admin;
    let setup = match loaded.mode {
        BootstrapMode::SetupRequired { token } => {
            eprintln!("Raindrop setup token: {}", token.expose_secret());
            SetupService::required(data_dir, token, public_url)
        }
        BootstrapMode::Ready => {
            let database_url =
                database_url.context("ready configuration is missing RAINDROP_DATABASE_URL")?;
            let database = connect(&DatabaseConfig::new(database_url))
                .await
                .context("failed to connect to the configured database")?;
            migrate(&database)
                .await
                .context("failed to migrate the configured database")?;
            let token = new_setup_token();
            let mut setup = SetupService::from_configured_database(
                data_dir.clone(),
                token.clone(),
                public_url.clone(),
                database.clone(),
            )
            .await
            .context("failed to inspect configured bootstrap state")?;
            if setup.setup_mode() == Some(raindrop::setup::SetupMode::AdminOnly)
                && let Some(admin) = bootstrap_admin
            {
                create_admin(
                    &database,
                    &PasswordService::default(),
                    CreateAdminInput {
                        username: admin.username,
                        password: admin.password,
                        email: admin.email,
                    },
                )
                .await
                .context("failed to create the bootstrap administrator")?;
                setup = SetupService::from_configured_database(
                    data_dir,
                    token.clone(),
                    public_url,
                    database,
                )
                .await
                .context(
                    "failed to inspect configured bootstrap state after administrator creation",
                )?;
            }
            if setup.setup_mode() == Some(raindrop::setup::SetupMode::AdminOnly) {
                eprintln!("Raindrop setup token: {}", token.expose_secret());
            }
            setup
        }
    };
    let listener = TcpListener::bind(address)
        .await
        .with_context(|| format!("failed to bind Raindrop to {address}"))?;
    let (background_runtime, background_handle) =
        BackgroundRuntime::production(setup.clone(), feed_retention, provider_keyring.clone())
            .context("failed to compose Raindrop background runtime")?;
    let background_runtime_task = tokio::spawn(background_runtime.run());

    info!(%address, version = env!("CARGO_PKG_VERSION"), "Raindrop listening");
    let (server_shutdown_tx, server_shutdown_rx) = oneshot::channel();
    let server = axum::serve(
        listener,
        build_router(AppState::with_runtime_services(
            setup,
            background_handle.feed(),
            background_handle.content(),
            provider_keyring,
        ))
        .into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .with_graceful_shutdown(async move {
        let _ = server_shutdown_rx.await;
    });
    let runtime_shutdown = background_handle.clone();
    coordinate_server_and_background_runtime(
        shutdown_signal(),
        move || runtime_shutdown.shutdown(),
        background_runtime_task,
        server_shutdown_tx,
        server,
    )
    .await
}

async fn coordinate_server_and_background_runtime<Signal, Shutdown, Server, RuntimeError>(
    signal: Signal,
    mut request_runtime_shutdown: Shutdown,
    runtime_task: JoinHandle<Result<(), RuntimeError>>,
    server_shutdown_tx: oneshot::Sender<()>,
    server: Server,
) -> Result<()>
where
    Signal: Future<Output = ()>,
    Shutdown: FnMut(),
    Server: IntoFuture<Output = std::io::Result<()>>,
    RuntimeError: Error + Send + Sync + 'static,
{
    let server = server.into_future();
    let mut runtime_task = runtime_task;
    let mut server_shutdown_tx = Some(server_shutdown_tx);
    tokio::pin!(signal);
    tokio::pin!(server);

    enum FirstCompletion<E> {
        Server(std::io::Result<()>),
        Signal,
        Runtime(Result<Result<(), E>, tokio::task::JoinError>),
    }

    let first = tokio::select! {
        biased;
        result = &mut runtime_task => FirstCompletion::Runtime(result),
        result = &mut server => FirstCompletion::Server(result),
        () = &mut signal => FirstCompletion::Signal,
    };

    match first {
        FirstCompletion::Signal => {
            request_runtime_shutdown();
            let runtime_result = runtime_task.await;
            if let Some(shutdown) = server_shutdown_tx.take() {
                let _ = shutdown.send(());
            }
            let server_result = server.await;
            server_result.context("Raindrop HTTP server failed")?;
            joined_runtime_result(runtime_result)
        }
        FirstCompletion::Server(server_result) => {
            request_runtime_shutdown();
            let runtime_result = runtime_task.await;
            server_result.context("Raindrop HTTP server failed")?;
            joined_runtime_result(runtime_result)
        }
        FirstCompletion::Runtime(runtime_result) => {
            request_runtime_shutdown();
            if let Some(shutdown) = server_shutdown_tx.take() {
                let _ = shutdown.send(());
            }
            if let Err(error) = server.await {
                tracing::error!(%error, "Raindrop HTTP server drain failed after runtime exit");
            }
            unexpected_runtime_result(runtime_result)
        }
    }
}

fn joined_runtime_result<RuntimeError>(
    result: Result<Result<(), RuntimeError>, tokio::task::JoinError>,
) -> Result<()>
where
    RuntimeError: Error + Send + Sync + 'static,
{
    match result {
        Ok(Ok(())) => Ok(()),
        Ok(Err(error)) => {
            Err(anyhow::Error::new(error)).context("Raindrop background runtime failed")
        }
        Err(error) => Err(error).context("Raindrop background runtime task failed"),
    }
}

fn unexpected_runtime_result<RuntimeError>(
    result: Result<Result<(), RuntimeError>, tokio::task::JoinError>,
) -> Result<()>
where
    RuntimeError: Error + Send + Sync + 'static,
{
    match result {
        Ok(Ok(())) => Err(anyhow!("Raindrop background runtime stopped unexpectedly")),
        Ok(Err(error)) => {
            Err(anyhow::Error::new(error)).context("Raindrop background runtime failed")
        }
        Err(error) => Err(error).context("Raindrop background runtime task failed"),
    }
}

fn init_tracing() {
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("raindrop=info,tower_http=info"));
    tracing_subscriber::fmt().with_env_filter(filter).init();
}

async fn shutdown_signal() {
    let ctrl_c = async {
        if let Err(error) = tokio::signal::ctrl_c().await {
            tracing::error!(%error, "failed to install Ctrl-C handler");
        }
    };

    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut signal) => {
                signal.recv().await;
            }
            Err(error) => tracing::error!(%error, "failed to install SIGTERM handler"),
        }
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        () = ctrl_c => {},
        () = terminate => {},
    }
}

#[cfg(test)]
mod tests {
    use std::{
        future, io,
        sync::{
            Arc,
            atomic::{AtomicBool, AtomicUsize, Ordering},
        },
        time::Duration,
    };

    use raindrop::{config::FeedRetentionConfig, feeds::FeedServiceError};
    use secrecy::SecretString;
    use tokio::sync::oneshot;

    use super::*;

    #[derive(Clone, Copy, Debug)]
    enum InjectedRuntimeCompletion {
        EarlyOk,
        Error,
        Panic,
    }

    #[tokio::test]
    async fn coordinator_background_error_early_ok_and_panic_stop_server_first() {
        for completion in [
            InjectedRuntimeCompletion::EarlyOk,
            InjectedRuntimeCompletion::Error,
            InjectedRuntimeCompletion::Panic,
        ] {
            assert_runtime_completion_stops_server(completion).await;
        }
    }

    async fn assert_runtime_completion_stops_server(completion: InjectedRuntimeCompletion) {
        let runtime_task = tokio::spawn(async move {
            match completion {
                InjectedRuntimeCompletion::EarlyOk => Ok(()),
                InjectedRuntimeCompletion::Error => Err(FeedServiceError::RuntimeSupervision),
                InjectedRuntimeCompletion::Panic => panic!("injected runtime panic"),
            }
        });
        let shutdown_calls = Arc::new(AtomicUsize::new(0));
        let observed_shutdown_calls = shutdown_calls.clone();
        let request_runtime_shutdown = move || {
            observed_shutdown_calls.fetch_add(1, Ordering::SeqCst);
        };
        let server_stopped = Arc::new(AtomicBool::new(false));
        let observed_server_stopped = server_stopped.clone();
        let (server_shutdown_tx, server_shutdown_rx) = oneshot::channel();
        let server = async move {
            server_shutdown_rx
                .await
                .map_err(|_| io::Error::other("server shutdown sender dropped"))?;
            observed_server_stopped.store(true, Ordering::SeqCst);
            Ok(())
        };

        let error = tokio::time::timeout(
            Duration::from_secs(1),
            coordinate_server_and_background_runtime(
                future::pending(),
                request_runtime_shutdown,
                runtime_task,
                server_shutdown_tx,
                server,
            ),
        )
        .await
        .expect("runtime completion should immediately stop the HTTP server")
        .expect_err("runtime completion before signal/server should fail coordination");

        assert!(server_stopped.load(Ordering::SeqCst));
        assert_eq!(shutdown_calls.load(Ordering::SeqCst), 1);
        match completion {
            InjectedRuntimeCompletion::EarlyOk => {
                assert_eq!(
                    error.to_string(),
                    "Raindrop background runtime stopped unexpectedly"
                );
            }
            InjectedRuntimeCompletion::Error => {
                assert_eq!(error.to_string(), "Raindrop background runtime failed");
                assert!(error.downcast_ref::<FeedServiceError>().is_some());
            }
            InjectedRuntimeCompletion::Panic => {
                assert_eq!(error.to_string(), "Raindrop background runtime task failed");
                assert!(error.downcast_ref::<tokio::task::JoinError>().is_some());
            }
        }
    }

    #[tokio::test]
    async fn production_background_runtime_stays_inert_while_setup_is_required() {
        let data = tempfile::tempdir().expect("temporary directory should be created");
        let setup = SetupService::required(data.path(), SecretString::from("setup-token"), None);
        let (runtime, handle) =
            BackgroundRuntime::production(setup, FeedRetentionConfig::default(), None)
                .expect("production background runtime should compose");

        handle.shutdown();
        runtime
            .run()
            .await
            .expect("pre-start production group should stop without constructing transport");
    }

    #[tokio::test]
    async fn coordinator_requests_runtime_shutdown_before_server_drain() {
        let runtime_stopped = Arc::new(AtomicBool::new(false));
        let observed_runtime_stopped = runtime_stopped.clone();
        let (runtime_shutdown_tx, runtime_shutdown_rx) = oneshot::channel();
        let runtime_task = tokio::spawn(async move {
            runtime_shutdown_rx
                .await
                .expect("runtime shutdown sender should remain open");
            observed_runtime_stopped.store(true, Ordering::SeqCst);
            Ok::<_, FeedServiceError>(())
        });
        let shutdown_calls = Arc::new(AtomicUsize::new(0));
        let observed_shutdown_calls = shutdown_calls.clone();
        let mut runtime_shutdown_tx = Some(runtime_shutdown_tx);
        let request_runtime_shutdown = move || {
            observed_shutdown_calls.fetch_add(1, Ordering::SeqCst);
            if let Some(shutdown) = runtime_shutdown_tx.take() {
                let _ = shutdown.send(());
            }
        };
        let (signal_tx, signal_rx) = oneshot::channel();
        let (server_shutdown_tx, server_shutdown_rx) = oneshot::channel();
        signal_tx
            .send(())
            .expect("test shutdown signal receiver should remain open");
        let server_shutdown_calls = shutdown_calls.clone();
        let server_runtime_stopped = runtime_stopped.clone();
        let server = async move {
            server_shutdown_rx
                .await
                .map_err(|_| io::Error::other("server shutdown sender dropped"))?;
            assert_eq!(server_shutdown_calls.load(Ordering::SeqCst), 1);
            tokio::time::timeout(Duration::from_secs(1), async {
                while !server_runtime_stopped.load(Ordering::SeqCst) {
                    tokio::task::yield_now().await;
                }
            })
            .await
            .map_err(|_| io::Error::other("runtime did not stop before server drain"))?;
            Ok(())
        };

        coordinate_server_and_background_runtime(
            async {
                signal_rx
                    .await
                    .expect("test shutdown signal sender should remain open");
            },
            request_runtime_shutdown,
            runtime_task,
            server_shutdown_tx,
            server,
        )
        .await
        .expect("coordinated shutdown should succeed");

        assert_eq!(shutdown_calls.load(Ordering::SeqCst), 1);
        assert!(runtime_stopped.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn coordinator_server_error_still_shuts_down_and_joins_runtime() {
        let runtime_stopped = Arc::new(AtomicBool::new(false));
        let observed_runtime_stopped = runtime_stopped.clone();
        let (runtime_shutdown_tx, runtime_shutdown_rx) = oneshot::channel();
        let runtime_task = tokio::spawn(async move {
            runtime_shutdown_rx
                .await
                .expect("runtime shutdown sender should remain open");
            observed_runtime_stopped.store(true, Ordering::SeqCst);
            Ok::<_, FeedServiceError>(())
        });
        let shutdown_calls = Arc::new(AtomicUsize::new(0));
        let observed_shutdown_calls = shutdown_calls.clone();
        let mut runtime_shutdown_tx = Some(runtime_shutdown_tx);
        let request_runtime_shutdown = move || {
            observed_shutdown_calls.fetch_add(1, Ordering::SeqCst);
            if let Some(shutdown) = runtime_shutdown_tx.take() {
                let _ = shutdown.send(());
            }
        };
        let (server_shutdown_tx, _server_shutdown_rx) = oneshot::channel();
        let server = async { Err(io::Error::other("injected server failure")) };

        let error = coordinate_server_and_background_runtime(
            future::pending(),
            request_runtime_shutdown,
            runtime_task,
            server_shutdown_tx,
            server,
        )
        .await
        .expect_err("server failure should be returned after runtime cleanup");

        assert_eq!(error.to_string(), "Raindrop HTTP server failed");
        assert_eq!(shutdown_calls.load(Ordering::SeqCst), 1);
        assert!(runtime_stopped.load(Ordering::SeqCst));
    }
}
