use axum::{Json, Router, extract::FromRef, routing::get};
use serde::Serialize;
use std::{
    collections::{HashMap, VecDeque},
    future::Future,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tokio::sync::Semaphore;

use crate::{
    api::{self, AccountThrottle, RateLimiter, UserMutationLimiter},
    auth::SessionService,
    content::{provider::ProviderSecretKeyring, worker::ContentRuntimeHandle},
    feeds::{
        FeedRuntime, FeedRuntimeHandle, FeedServiceError, FeedTransport, HttpFeedTransport,
        InertImageDto,
    },
    setup::SetupService,
    web,
};

#[derive(Clone)]
pub struct AppState {
    pub version: &'static str,
    pub(crate) setup: SetupService,
    pub(crate) login_limiter: RateLimiter,
    pub(crate) login_authentication_semaphore: Arc<Semaphore>,
    pub(crate) login_account_throttle: AccountThrottle,
    pub(crate) setup_limiter: RateLimiter,
    pub(crate) media_fetch_semaphore: Arc<Semaphore>,
    pub(crate) media_transport: Option<Arc<dyn FeedTransport>>,
    entry_image_cache: Arc<Mutex<EntryImageCache>>,
    pub feed_runtime: FeedRuntimeHandle,
    pub content_runtime: ContentRuntimeHandle,
    pub(crate) provider_keyring: Option<Arc<ProviderSecretKeyring>>,
    pub organization_mutation_limiter: UserMutationLimiter,
    pub preferences_mutation_limiter: UserMutationLimiter,
    pub subscription_mutation_limiter: UserMutationLimiter,
    pub provider_mutation_limiter: UserMutationLimiter,
    pub content_mutation_limiter: UserMutationLimiter,
}

impl AppState {
    #[must_use]
    pub fn new(setup: SetupService) -> Self {
        let (_runtime, handle) = FeedRuntime::<HttpFeedTransport>::new(setup.clone(), |_| {
            Err(FeedServiceError::CorruptFeed)
        });
        Self::with_feed_runtime(setup, handle)
    }

    #[must_use]
    pub fn with_feed_runtime(setup: SetupService, feed_runtime: FeedRuntimeHandle) -> Self {
        Self::with_runtimes(setup, feed_runtime, ContentRuntimeHandle::inert())
    }

    #[must_use]
    pub fn with_runtimes(
        setup: SetupService,
        feed_runtime: FeedRuntimeHandle,
        content_runtime: ContentRuntimeHandle,
    ) -> Self {
        Self::with_runtime_services(setup, feed_runtime, content_runtime, None)
    }

    #[must_use]
    pub fn with_runtime_services(
        setup: SetupService,
        feed_runtime: FeedRuntimeHandle,
        content_runtime: ContentRuntimeHandle,
        provider_keyring: Option<Arc<ProviderSecretKeyring>>,
    ) -> Self {
        Self {
            version: env!("CARGO_PKG_VERSION"),
            setup,
            login_limiter: RateLimiter::new(4_096, std::time::Duration::from_secs(15 * 60)),
            login_authentication_semaphore: Arc::new(Semaphore::new(4)),
            login_account_throttle: AccountThrottle::new(
                std::time::Duration::from_secs(15 * 60),
                std::time::Duration::from_millis(5),
                std::time::Duration::from_millis(100),
                10_000,
            ),
            setup_limiter: RateLimiter::new(30, std::time::Duration::from_secs(15 * 60)),
            media_fetch_semaphore: Arc::new(Semaphore::new(32)),
            media_transport: None,
            entry_image_cache: Arc::new(Mutex::new(EntryImageCache::default())),
            feed_runtime,
            content_runtime,
            provider_keyring,
            organization_mutation_limiter: UserMutationLimiter::new(),
            preferences_mutation_limiter: UserMutationLimiter::new(),
            subscription_mutation_limiter: UserMutationLimiter::new(),
            provider_mutation_limiter: UserMutationLimiter::new(),
            content_mutation_limiter: UserMutationLimiter::new(),
        }
    }

    #[must_use]
    pub fn with_provider_keyring(
        mut self,
        provider_keyring: Option<Arc<ProviderSecretKeyring>>,
    ) -> Self {
        self.provider_keyring = provider_keyring;
        self
    }

    #[must_use]
    pub fn with_media_transport(mut self, media_transport: Arc<dyn FeedTransport>) -> Self {
        self.media_transport = Some(media_transport);
        self
    }

    pub async fn commit_and_notify_feed_runtime<F, T, E>(&self, command: F) -> Result<T, E>
    where
        F: Future<Output = Result<T, E>>,
    {
        notify_after_commit(command, || self.feed_runtime.notify()).await
    }

    #[must_use]
    pub(crate) fn provider_keyring(&self) -> Option<Arc<ProviderSecretKeyring>> {
        self.provider_keyring.clone()
    }

    #[must_use]
    pub(crate) fn media_transport(&self) -> Option<Arc<dyn FeedTransport>> {
        self.media_transport.clone()
    }

    pub(crate) fn cache_entry_images(
        &self,
        user_id: &str,
        entry_id: &str,
        images: Vec<InertImageDto>,
    ) {
        self.entry_image_cache
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(user_id, entry_id, images);
    }

    pub(crate) fn cached_entry_image_source(
        &self,
        user_id: &str,
        entry_id: &str,
        image_index: u32,
    ) -> Option<String> {
        self.entry_image_cache
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .source(user_id, entry_id, image_index)
    }

    #[must_use]
    pub fn for_test() -> Self {
        Self::new(SetupService::required(
            std::path::Path::new("."),
            secrecy::SecretString::from("health-test-setup-token".to_owned()),
            None,
        ))
    }
}

const ENTRY_IMAGE_CACHE_CAPACITY: usize = 32;
const ENTRY_IMAGE_CACHE_TTL: Duration = Duration::from_secs(10 * 60);

#[derive(Default)]
struct EntryImageCache {
    entries: HashMap<(String, String), CachedEntryImages>,
    insertion_order: VecDeque<(String, String)>,
}

struct CachedEntryImages {
    images: Vec<InertImageDto>,
    expires_at: Instant,
}

impl EntryImageCache {
    fn insert(&mut self, user_id: &str, entry_id: &str, images: Vec<InertImageDto>) {
        let key = (user_id.to_owned(), entry_id.to_owned());
        self.remove(&key);
        while self.entries.len() >= ENTRY_IMAGE_CACHE_CAPACITY {
            let Some(oldest) = self.insertion_order.pop_front() else {
                break;
            };
            self.entries.remove(&oldest);
        }
        self.insertion_order.push_back(key.clone());
        self.entries.insert(
            key,
            CachedEntryImages {
                images,
                expires_at: Instant::now() + ENTRY_IMAGE_CACHE_TTL,
            },
        );
    }

    fn source(&mut self, user_id: &str, entry_id: &str, image_index: u32) -> Option<String> {
        let key = (user_id.to_owned(), entry_id.to_owned());
        if self
            .entries
            .get(&key)
            .is_some_and(|cached| Instant::now() >= cached.expires_at)
        {
            self.remove(&key);
            return None;
        }
        self.entries.get(&key).and_then(|cached| {
            cached
                .images
                .iter()
                .find(|image| image.image_index == image_index)
                .map(|image| image.source_url.clone())
        })
    }

    fn remove(&mut self, key: &(String, String)) {
        self.entries.remove(key);
        self.insertion_order.retain(|stored| stored != key);
    }
}

async fn notify_after_commit<F, T, E, N>(command: F, notify: N) -> Result<T, E>
where
    F: Future<Output = Result<T, E>>,
    N: FnOnce(),
{
    let committed = command.await?;
    notify();
    Ok(committed)
}

impl FromRef<AppState> for SessionService {
    fn from_ref(state: &AppState) -> Self {
        state.setup.sessions()
    }
}

#[derive(Serialize)]
struct HealthResponse {
    status: &'static str,
}

pub fn build_router(state: AppState) -> Router {
    Router::new()
        .route("/api/v1/health/live", get(live_health))
        .merge(api::router())
        .fallback(web::serve)
        .with_state(state)
}

async fn live_health() -> Json<HealthResponse> {
    Json(HealthResponse { status: "ok" })
}

#[cfg(test)]
mod tests {
    use secrecy::SecretString;

    use crate::feeds::{FeedRuntime, FeedServiceError, HttpFeedTransport};

    use super::*;

    #[test]
    fn app_state_new_provides_inert_runtime_and_exact_mutation_limit() {
        let data = tempfile::tempdir().expect("temporary directory should be created");
        let state = AppState::new(SetupService::required(
            data.path(),
            SecretString::from("setup-token"),
            None,
        ));

        state.feed_runtime.notify();
        state.feed_runtime.shutdown();
        state.content_runtime.notify();
        state.content_runtime.shutdown();
        for _ in 0..30 {
            state
                .subscription_mutation_limiter
                .check("user")
                .expect("the exact production mutation budget should be admitted");
            state
                .organization_mutation_limiter
                .check("user")
                .expect("the exact organization mutation budget should be admitted");
            state
                .preferences_mutation_limiter
                .check("user")
                .expect("the exact preference mutation budget should be admitted");
            state
                .provider_mutation_limiter
                .check("user")
                .expect("the exact provider mutation budget should be admitted");
            state
                .content_mutation_limiter
                .check("user")
                .expect("the exact content mutation budget should be admitted");
        }
        assert!(state.subscription_mutation_limiter.check("user").is_err());
        assert!(state.organization_mutation_limiter.check("user").is_err());
        assert!(state.preferences_mutation_limiter.check("user").is_err());
        assert!(state.provider_mutation_limiter.check("user").is_err());
        assert!(state.content_mutation_limiter.check("user").is_err());
    }

    #[tokio::test]
    async fn app_state_with_feed_runtime_preserves_the_production_handle() {
        let data = tempfile::tempdir().expect("temporary directory should be created");
        let setup = SetupService::required(data.path(), SecretString::from("setup-token"), None);
        let (runtime, handle) = FeedRuntime::<HttpFeedTransport>::new(setup.clone(), |_| {
            Err(FeedServiceError::CorruptFeed)
        });
        let state = AppState::with_feed_runtime(setup, handle);

        state.feed_runtime.shutdown();
        runtime
            .run()
            .await
            .expect("pre-start shutdown should stop the production runtime cleanly");
    }

    #[tokio::test]
    async fn app_state_with_runtimes_preserves_both_production_handles() {
        let data = tempfile::tempdir().expect("temporary directory should be created");
        let setup = SetupService::required(data.path(), SecretString::from("setup-token"), None);
        let (feed, feed_handle) = FeedRuntime::<HttpFeedTransport>::new(setup.clone(), |_| {
            Err(FeedServiceError::CorruptFeed)
        });
        let (content_handle, _, mut content_shutdown) =
            crate::content::worker::ContentRuntimeHandle::control();
        let state = AppState::with_runtimes(setup, feed_handle, content_handle);

        state.feed_runtime.shutdown();
        state.content_runtime.shutdown();

        feed.run()
            .await
            .expect("pre-start Feed shutdown should stop cleanly");
        content_shutdown
            .changed()
            .await
            .expect("Content shutdown sender should remain available");
        assert!(*content_shutdown.borrow());
    }

    #[tokio::test]
    async fn app_state_with_runtime_services_preserves_shared_provider_keyring() {
        use crate::content::provider::ProviderSecretKeyring;
        use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};

        let data = tempfile::tempdir().expect("temporary directory should be created");
        let setup = SetupService::required(data.path(), SecretString::from("setup-token"), None);
        let (feed, feed_handle) = FeedRuntime::<HttpFeedTransport>::new(setup.clone(), |_| {
            Err(FeedServiceError::CorruptFeed)
        });
        let key = SecretString::from(format!("primary:{}", URL_SAFE_NO_PAD.encode([0x41_u8; 32])));
        let keyring = Arc::new(
            ProviderSecretKeyring::from_entries(&[key])
                .expect("shared provider keyring should construct"),
        );
        let state = AppState::with_runtime_services(
            setup,
            feed_handle,
            ContentRuntimeHandle::inert(),
            Some(Arc::clone(&keyring)),
        );

        let stored_keyring = state.provider_keyring().expect("keyring should be present");
        assert!(Arc::ptr_eq(&stored_keyring, &keyring,));
        state.feed_runtime.shutdown();
        feed.run().await.expect("feed runtime should stop cleanly");
    }

    #[tokio::test]
    async fn post_commit_notification_runs_after_successful_command() {
        let events = Arc::new(std::sync::Mutex::new(Vec::new()));
        let command_events = events.clone();
        let notify_events = events.clone();

        let committed = notify_after_commit(
            async move {
                command_events
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .push("command");
                Ok::<_, ()>("committed")
            },
            move || {
                notify_events
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .push("notify");
            },
        )
        .await
        .expect("successful command should preserve its result");

        assert_eq!(committed, "committed");
        assert_eq!(
            *events
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
            ["command", "notify"]
        );
    }

    #[tokio::test]
    async fn failed_command_does_not_notify_feed_runtime() {
        let notifications = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let observed_notifications = notifications.clone();

        let result = notify_after_commit(async { Err::<(), _>("command failed") }, move || {
            observed_notifications.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        })
        .await;

        assert_eq!(result, Err("command failed"));
        assert_eq!(notifications.load(std::sync::atomic::Ordering::SeqCst), 0);
    }
}
