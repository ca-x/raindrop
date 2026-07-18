use std::{
    future::{Future, IntoFuture},
    path::PathBuf,
    sync::Arc,
};

use anyhow::{Context, Result, anyhow};
use raindrop::{
    app::{AppState, build_router},
    auth::{CreateAdminInput, PasswordService, create_admin},
    config::{BootstrapMode, ConfigArgs, FeedRetentionConfig, SystemEnv, load, new_setup_token},
    db::{DatabaseConfig, connect, migrate},
    feeds::{
        FeedExecutor, FeedRepository, FeedRetentionPolicy, FeedRuntime, FeedRuntimeHandle,
        FeedServiceError, FeedUrlPolicy, HttpFeedTransport,
    },
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

    let loaded = load(
        &ConfigArgs {
            data_dir: PathBuf::from("data"),
            config_path: None,
        },
        &SystemEnv,
    )
    .context("failed to load Raindrop configuration")?;
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
    let (feed_runtime, feed_runtime_handle) =
        production_feed_runtime(setup.clone(), feed_retention);
    let feed_runtime_task = tokio::spawn(feed_runtime.run());

    info!(%address, version = env!("CARGO_PKG_VERSION"), "Raindrop listening");
    let (server_shutdown_tx, server_shutdown_rx) = oneshot::channel();
    let server = axum::serve(
        listener,
        build_router(AppState::with_feed_runtime(
            setup,
            feed_runtime_handle.clone(),
        ))
        .into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .with_graceful_shutdown(async move {
        let _ = server_shutdown_rx.await;
    });
    let runtime_shutdown = feed_runtime_handle.clone();
    coordinate_server_and_feed_runtime(
        shutdown_signal(),
        move || runtime_shutdown.shutdown(),
        feed_runtime_task,
        server_shutdown_tx,
        server,
    )
    .await
}

fn production_feed_runtime(
    setup: SetupService,
    retention: FeedRetentionConfig,
) -> (FeedRuntime<HttpFeedTransport>, FeedRuntimeHandle) {
    let (runtime, handle) = FeedRuntime::new(setup, |database| {
        let url_policy = FeedUrlPolicy::new(false);
        let transport =
            HttpFeedTransport::new(url_policy).map_err(FeedServiceError::ExecutorInitialization)?;
        Ok(Arc::new(FeedExecutor::new(
            FeedRepository::new(database),
            url_policy,
            transport,
        )))
    });
    (
        runtime.with_retention_policy(FeedRetentionPolicy::new(retention.orphan_grace)),
        handle,
    )
}

async fn coordinate_server_and_feed_runtime<Signal, Shutdown, Server>(
    signal: Signal,
    mut request_runtime_shutdown: Shutdown,
    runtime_task: JoinHandle<Result<(), FeedServiceError>>,
    server_shutdown_tx: oneshot::Sender<()>,
    server: Server,
) -> Result<()>
where
    Signal: Future<Output = ()>,
    Shutdown: FnMut(),
    Server: IntoFuture<Output = std::io::Result<()>>,
{
    let server = server.into_future();
    let mut runtime_task = runtime_task;
    let mut server_shutdown_tx = Some(server_shutdown_tx);
    tokio::pin!(signal);
    tokio::pin!(server);

    enum FirstCompletion {
        Server(std::io::Result<()>),
        Signal,
        Runtime(Result<Result<(), FeedServiceError>, tokio::task::JoinError>),
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

fn joined_runtime_result(
    result: Result<Result<(), FeedServiceError>, tokio::task::JoinError>,
) -> Result<()> {
    result
        .context("Raindrop feed runtime task failed")?
        .context("Raindrop feed runtime failed")
}

fn unexpected_runtime_result(
    result: Result<Result<(), FeedServiceError>, tokio::task::JoinError>,
) -> Result<()> {
    match result {
        Ok(Ok(())) => Err(anyhow!("Raindrop feed runtime stopped unexpectedly")),
        Ok(Err(error)) => Err(error).context("Raindrop feed runtime failed"),
        Err(error) => Err(error).context("Raindrop feed runtime task failed"),
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
    async fn coordinator_runtime_error_early_ok_and_panic_stop_server_first() {
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
            coordinate_server_and_feed_runtime(
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
                    "Raindrop feed runtime stopped unexpectedly"
                );
            }
            InjectedRuntimeCompletion::Error => {
                assert_eq!(error.to_string(), "Raindrop feed runtime failed");
                assert!(error.downcast_ref::<FeedServiceError>().is_some());
            }
            InjectedRuntimeCompletion::Panic => {
                assert_eq!(error.to_string(), "Raindrop feed runtime task failed");
                assert!(error.downcast_ref::<tokio::task::JoinError>().is_some());
            }
        }
    }

    #[tokio::test]
    async fn production_feed_runtime_stays_inert_while_setup_is_required() {
        let data = tempfile::tempdir().expect("temporary directory should be created");
        let setup = SetupService::required(data.path(), SecretString::from("setup-token"), None);
        let (runtime, handle) = production_feed_runtime(setup, FeedRetentionConfig::default());

        handle.shutdown();
        runtime
            .run()
            .await
            .expect("pre-start production runtime should stop without constructing transport");
    }

    #[tokio::test]
    async fn coordinator_requests_runtime_shutdown_before_server_drain() {
        let data = tempfile::tempdir().expect("temporary directory should be created");
        let setup = SetupService::required(data.path(), SecretString::from("setup-token"), None);
        let (runtime, handle) = production_feed_runtime(setup, FeedRetentionConfig::default());
        let runtime_stopped = Arc::new(AtomicBool::new(false));
        let observed_runtime_stopped = runtime_stopped.clone();
        let runtime_task = tokio::spawn(async move {
            let result = runtime.run().await;
            observed_runtime_stopped.store(true, Ordering::SeqCst);
            result
        });
        let shutdown_calls = Arc::new(AtomicUsize::new(0));
        let observed_shutdown_calls = shutdown_calls.clone();
        let shutdown_handle = handle.clone();
        let request_runtime_shutdown = move || {
            observed_shutdown_calls.fetch_add(1, Ordering::SeqCst);
            shutdown_handle.shutdown();
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

        coordinate_server_and_feed_runtime(
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
        let data = tempfile::tempdir().expect("temporary directory should be created");
        let setup = SetupService::required(data.path(), SecretString::from("setup-token"), None);
        let (runtime, handle) = production_feed_runtime(setup, FeedRetentionConfig::default());
        let runtime_stopped = Arc::new(AtomicBool::new(false));
        let observed_runtime_stopped = runtime_stopped.clone();
        let runtime_task = tokio::spawn(async move {
            let result = runtime.run().await;
            observed_runtime_stopped.store(true, Ordering::SeqCst);
            result
        });
        let shutdown_calls = Arc::new(AtomicUsize::new(0));
        let observed_shutdown_calls = shutdown_calls.clone();
        let request_runtime_shutdown = move || {
            observed_shutdown_calls.fetch_add(1, Ordering::SeqCst);
            handle.shutdown();
        };
        let (server_shutdown_tx, _server_shutdown_rx) = oneshot::channel();
        let server = async { Err(io::Error::other("injected server failure")) };

        let error = coordinate_server_and_feed_runtime(
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
