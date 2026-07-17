use std::{path::PathBuf, sync::Arc};

use anyhow::{Context, Result};
use raindrop::{
    app::{AppState, build_router},
    auth::{CreateAdminInput, PasswordService, create_admin},
    config::{BootstrapMode, ConfigArgs, SystemEnv, load, new_setup_token},
    db::{DatabaseConfig, connect, migrate},
    feeds::{
        FeedExecutor, FeedRepository, FeedRuntime, FeedRuntimeHandle, FeedServiceError,
        FeedUrlPolicy, HttpFeedTransport,
    },
    setup::SetupService,
};
use secrecy::ExposeSecret;
use tokio::net::TcpListener;
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
    let (feed_runtime, feed_runtime_handle) = production_feed_runtime(setup.clone());
    let feed_runtime_task = tokio::spawn(feed_runtime.run());

    info!(%address, version = env!("CARGO_PKG_VERSION"), "Raindrop listening");
    let runtime_shutdown = feed_runtime_handle.clone();
    let server_result = axum::serve(
        listener,
        build_router(AppState::with_feed_runtime(
            setup,
            feed_runtime_handle.clone(),
        ))
        .into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .with_graceful_shutdown(async move {
        shutdown_signal().await;
        runtime_shutdown.shutdown();
    })
    .await;
    feed_runtime_handle.shutdown();
    let runtime_result = feed_runtime_task
        .await
        .context("Raindrop feed runtime task failed")?;
    server_result.context("Raindrop HTTP server failed")?;
    runtime_result.context("Raindrop feed runtime failed")?;

    Ok(())
}

fn production_feed_runtime(
    setup: SetupService,
) -> (FeedRuntime<HttpFeedTransport>, FeedRuntimeHandle) {
    FeedRuntime::new(setup, |database| {
        let url_policy = FeedUrlPolicy::new(false);
        let transport =
            HttpFeedTransport::new(url_policy).map_err(FeedServiceError::ExecutorInitialization)?;
        Ok(Arc::new(FeedExecutor::new(
            FeedRepository::new(database),
            url_policy,
            transport,
        )))
    })
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
    use secrecy::SecretString;

    use super::*;

    #[tokio::test]
    async fn production_feed_runtime_stays_inert_while_setup_is_required() {
        let data = tempfile::tempdir().expect("temporary directory should be created");
        let setup = SetupService::required(data.path(), SecretString::from("setup-token"), None);
        let (runtime, handle) = production_feed_runtime(setup);

        handle.shutdown();
        runtime
            .run()
            .await
            .expect("pre-start production runtime should stop without constructing transport");
    }
}
