use std::{
    fs::{File, OpenOptions},
    path::{Path, PathBuf},
    sync::mpsc,
    time::Duration,
};

use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Method, Request},
};
use raindrop::{
    app::{AppState, build_router},
    auth::{CreateAdminInput, PasswordService, SessionService, create_admin},
    background::BackgroundRuntime,
    backups::BackupRuntime,
    config::{
        FeedRetentionConfig, load_existing_local_provider_secret_keys,
        load_or_create_local_provider_secret_keys, new_setup_token,
    },
    content::provider::ProviderSecretKeyring,
    db::{
        DatabaseConfig, connect,
        entities::{ai_provider, bootstrap_state, user},
        migrate,
    },
    feeds::install_ring_crypto_provider,
    setup::SetupService,
};
use sea_orm::{DatabaseConnection, EntityTrait, PaginatorTrait};
use secrecy::{ExposeSecret, SecretString};
use tokio::{runtime::Handle, sync::oneshot};
use tower::ServiceExt;
use zeroize::Zeroizing;

use crate::api::ApiError;

pub(crate) struct Local {
    pub(crate) user_id: String,
    feed_runtime: raindrop::feeds::FeedRuntimeHandle,
    router: Router,
    handle: Handle,
    cookie: Zeroizing<String>,
    pub(crate) csrf: Zeroizing<String>,
    pub(crate) database: DatabaseConnection,
    pub(crate) sync_lock: std::sync::Mutex<()>,
    shutdown: Option<oneshot::Sender<()>>,
}

impl Drop for Local {
    fn drop(&mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
    }
}

pub(crate) fn data_dir() -> Result<PathBuf, ApiError> {
    directories::ProjectDirs::from("io", "ca-x", "raindrop-desktop")
        .map(|dirs| dirs.data_local_dir().to_path_buf())
        .ok_or(ApiError::LocalData)
}

fn private_directory(path: &Path) -> Result<File, ApiError> {
    std::fs::create_dir_all(path).map_err(|_| ApiError::LocalData)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))
            .map_err(|_| ApiError::LocalData)?;
    }
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path.join("desktop.lock"))
        .map_err(|_| ApiError::LocalData)?;
    lock.try_lock().map_err(|_| ApiError::LocalBusy)?;
    Ok(lock)
}

impl Local {
    pub(crate) fn start(path: PathBuf) -> Result<Self, ApiError> {
        Self::start_with_background(path, true)
    }

    pub(crate) fn start_with_background(
        path: PathBuf,
        run_background: bool,
    ) -> Result<Self, ApiError> {
        let lock = private_directory(&path)?;
        let (sender, receiver) = mpsc::sync_channel(1);
        std::thread::Builder::new()
            .name("raindrop-local".into())
            .spawn(move || {
                let runtime = match tokio::runtime::Builder::new_multi_thread()
                    .worker_threads(2)
                    .enable_all()
                    .build()
                {
                    Ok(runtime) => runtime,
                    Err(_) => {
                        let _ = sender.send(Err(ApiError::LocalData));
                        return;
                    }
                };
                runtime.block_on(async move {
                    let initialized = initialize(&path).await;
                    let (database, setup, session, keyring, user_id) = match initialized {
                        Ok(initialized) => initialized,
                        Err(error) => {
                            let _ = sender.send(Err(error));
                            return;
                        }
                    };
                    let background = BackgroundRuntime::production(
                        setup.clone(),
                        FeedRetentionConfig::default(),
                        Some(keyring.clone()),
                    );
                    let backup = BackupRuntime::production(setup.clone(), Some(keyring.clone()));
                    let (Ok((background, background_handle)), Ok((backup, backup_handle))) =
                        (background, backup)
                    else {
                        let _ = sender.send(Err(ApiError::LocalData));
                        return;
                    };
                    let router = build_router(
                        AppState::with_runtime_services(
                            setup,
                            background_handle.feed(),
                            background_handle.content(),
                            Some(keyring),
                        )
                        .with_backup_runtime(backup_handle.clone()),
                    );
                    let background_task = run_background.then(|| tokio::spawn(background.run()));
                    let backup_task = run_background.then(|| tokio::spawn(backup.run()));
                    let (shutdown, mut stopped) = oneshot::channel();
                    let local = Local {
                        user_id,
                        feed_runtime: background_handle.feed(),
                        router,
                        handle: Handle::current(),
                        cookie: Zeroizing::new(format!(
                            "raindrop_session={}",
                            session.cookie_token.expose_secret()
                        )),
                        csrf: Zeroizing::new(session.csrf_token.expose_secret().to_owned()),
                        database: database.clone(),
                        sync_lock: std::sync::Mutex::new(()),
                        shutdown: Some(shutdown),
                    };
                    if sender.send(Ok(local)).is_ok() {
                        let _ = (&mut stopped).await;
                    }
                    background_handle.shutdown();
                    backup_handle.shutdown();
                    let _ = tokio::time::timeout(Duration::from_secs(5), async {
                        if let Some(task) = background_task {
                            let _ = task.await;
                        }
                        if let Some(task) = backup_task {
                            let _ = task.await;
                        }
                    })
                    .await;
                    let _ = SessionService::new(database.clone())
                        .revoke(&session.cookie_token)
                        .await;
                    let _ = database.close().await;
                    drop(lock);
                });
                runtime.shutdown_timeout(Duration::from_secs(2));
            })
            .map_err(|_| ApiError::LocalData)?;
        receiver.recv().map_err(|_| ApiError::LocalData)?
    }

    pub(crate) fn request(
        &self,
        method: Method,
        path: &str,
        body: Option<&str>,
    ) -> Result<(u16, Vec<u8>), ApiError> {
        let mut request = Request::builder()
            .method(method)
            .uri(path)
            .header("host", "localhost")
            .header("content-type", "application/json")
            .header("cookie", self.cookie.as_str())
            .header("x-csrf-token", self.csrf.as_str());
        if body.is_some() {
            request = request.header("accept", "application/json");
        }
        let request = request
            .body(Body::from(body.unwrap_or_default().to_owned()))
            .map_err(|_| ApiError::Response)?;
        self.handle.block_on(async {
            let response = self
                .router
                .clone()
                .oneshot(request)
                .await
                .map_err(|_| ApiError::LocalData)?;
            let status = response.status().as_u16();
            let bytes = to_bytes(response.into_body(), 8 * 1024 * 1024)
                .await
                .map_err(|_| ApiError::Response)?;
            Ok((status, bytes.to_vec()))
        })
    }

    pub(crate) fn block_on<T>(&self, future: impl std::future::Future<Output = T>) -> T {
        self.handle.block_on(future)
    }

    pub(crate) fn import_opml(&self, document: &[u8]) -> Result<usize, ApiError> {
        let document =
            raindrop::feeds::OpmlDocument::parse(document).map_err(|_| ApiError::Response)?;
        if document.invalid_count() != 0 {
            return Err(ApiError::Response);
        }
        let repository = raindrop::feeds::FeedRepository::new(self.database.clone());
        let result = self
            .block_on(repository.import_opml(&self.user_id, &document))
            .map_err(|_| ApiError::LocalData)?;
        self.feed_runtime.notify();
        Ok(result.imported_count)
    }
}

type Initialized = (
    DatabaseConnection,
    SetupService,
    raindrop::auth::CreatedSession,
    std::sync::Arc<ProviderSecretKeyring>,
    String,
);

async fn initialize(path: &Path) -> Result<Initialized, ApiError> {
    install_ring_crypto_provider().map_err(|_| ApiError::LocalData)?;
    let database_path = path.join("raindrop.sqlite3");
    let url = url::Url::from_file_path(&database_path).map_err(|_| ApiError::LocalData)?;
    let database_path = url.path();
    #[cfg(windows)]
    let database_path = database_path.strip_prefix('/').unwrap_or(database_path);
    let database = connect(
        &DatabaseConfig::new(SecretString::from(format!(
            "sqlite:{database_path}?mode=rwc"
        )))
        .with_sqlite_full_durability(),
    )
    .await
    .map_err(|_| ApiError::LocalData)?;
    migrate(&database).await.map_err(|_| ApiError::LocalData)?;
    let existing = bootstrap_state::Entity::find_by_id(1)
        .one(&database)
        .await
        .map_err(|_| ApiError::LocalData)?;
    let user_id = match existing {
        Some(existing) => {
            let account = user::Entity::find_by_id(&existing.administrator_user_id)
                .one(&database)
                .await
                .map_err(|_| ApiError::LocalData)?
                .ok_or(ApiError::LocalData)?;
            if account.is_disabled {
                return Err(ApiError::LocalData);
            }
            account.id
        }
        None => {
            create_admin(
                &database,
                &PasswordService::default(),
                CreateAdminInput {
                    username: "local".to_owned(),
                    password: new_setup_token(),
                    email: None,
                },
            )
            .await
            .map_err(|_| ApiError::LocalData)?
            .id
        }
    };
    let session = SessionService::new(database.clone())
        .create(&user_id)
        .await
        .map_err(|_| ApiError::LocalData)?;
    let setup = SetupService::ready(path, None, database.clone());
    let keys =
        match load_existing_local_provider_secret_keys(path).map_err(|_| ApiError::LocalData)? {
            Some(keys) => keys,
            None => {
                if ai_provider::Entity::find()
                    .count(&database)
                    .await
                    .map_err(|_| ApiError::LocalData)?
                    > 0
                {
                    return Err(ApiError::LocalData);
                }
                load_or_create_local_provider_secret_keys(path).map_err(|_| ApiError::LocalData)?
            }
        };
    let keyring = ProviderSecretKeyring::from_entries(&keys).map_err(|_| ApiError::LocalData)?;
    Ok((
        database,
        setup,
        session,
        std::sync::Arc::new(keyring),
        user_id,
    ))
}
