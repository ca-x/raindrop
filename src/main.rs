use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use raindrop::{
    app::{AppState, build_router},
    auth::{CreateAdminInput, PasswordService, create_admin},
    config::{BootstrapMode, ConfigArgs, SystemEnv, load},
    db::{DatabaseConfig, connect, entities::user, migrate},
    setup::SetupService,
};
use sea_orm::{EntityTrait, PaginatorTrait};
use secrecy::ExposeSecret;
use tokio::net::TcpListener;
use tracing::info;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> Result<()> {
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
            if user::Entity::find()
                .count(&database)
                .await
                .context("failed to inspect configured users")?
                == 0
            {
                let Some(admin) = bootstrap_admin else {
                    bail!(
                        "the configured database has no users; set the complete RAINDROP_BOOTSTRAP_ADMIN_* variables"
                    );
                };
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
            }
            SetupService::ready(data_dir, public_url, database)
        }
    };
    let listener = TcpListener::bind(address)
        .await
        .with_context(|| format!("failed to bind Raindrop to {address}"))?;

    info!(%address, version = env!("CARGO_PKG_VERSION"), "Raindrop listening");
    axum::serve(listener, build_router(AppState::new(setup)))
        .with_graceful_shutdown(shutdown_signal())
        .await
        .context("Raindrop HTTP server failed")?;

    Ok(())
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
