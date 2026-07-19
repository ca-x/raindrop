#[allow(dead_code)]
mod support;

use std::time::Duration;

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use raindrop::{
    content::{
        jobs::{
            ArtifactIdentity, ArtifactIdentityInput, ArtifactKind, ContentJobOperation,
            ContentJobTrigger, ContentRepository, EnqueueContentJob, EnqueueContentJobInput,
            EnqueueResult, JobStatus,
        },
        provider::ProviderKind,
        worker::ProductionContentRuntime,
    },
    db::{entities::plugin_installation, migrate},
    plugins::{PluginRegistryErrorKind, PluginRegistryRepository, PluginSystemState},
    setup::{SetupCompleteInput, SetupService},
};
use sea_orm::{ActiveModelTrait, ActiveValue::Set};
use secrecy::SecretString;
use support::database::{
    ENTRY_A_ID, HASH_A, HASH_B, HASH_C, HASH_D, SUBSCRIPTION_A_ID, USER_A_ID, connect_for_contract,
    insert_entry, insert_feed, insert_subscription, insert_user,
};
use time::macros::datetime;

const PROVIDER_ID: &str = "00000000-0000-4000-8000-000000000901";

#[tokio::test]
async fn setup_required_runtime_shuts_down_without_starting_work() {
    let data = tempfile::tempdir().expect("temporary production runtime directory");
    let setup = SetupService::required(
        data.path(),
        SecretString::from("rd_setup_content_runtime"),
        None,
    );
    let (runtime, handle) =
        ProductionContentRuntime::new(setup, Vec::new()).expect("production runtime composition");

    let task = tokio::spawn(runtime.run());
    handle.shutdown();

    tokio::time::timeout(Duration::from_secs(2), task)
        .await
        .expect("setup-waiting runtime should stop promptly")
        .expect("setup-waiting runtime task should join")
        .expect("setup-waiting runtime shutdown should succeed");
}

#[tokio::test]
async fn setup_completion_synchronizes_official_installation_without_restart() {
    let data = tempfile::tempdir().expect("temporary production runtime directory");
    let token = "rd_setup_content_transition";
    let setup = SetupService::required(data.path().join("state"), SecretString::from(token), None);
    let (runtime, handle) = ProductionContentRuntime::new(setup.clone(), Vec::new())
        .expect("production runtime composition");
    let task = tokio::spawn(runtime.run());

    let database_url = format!(
        "sqlite://{}?mode=rwc",
        data.path().join("content-transition.db").display()
    );
    setup
        .complete(
            token,
            SetupCompleteInput {
                database_url: SecretString::from(database_url),
                username: "Reader".to_owned(),
                password: SecretString::from("correct horse battery staple"),
                email: None,
            },
        )
        .await
        .expect("setup should become ready");

    let repository = PluginRegistryRepository::new(
        setup
            .database()
            .expect("ready setup should expose its database"),
    );
    let installation = tokio::time::timeout(Duration::from_secs(4), async {
        loop {
            match repository.get_installation("raindrop.ai-content").await {
                Ok(installation) => return installation,
                Err(error) if error.kind() == PluginRegistryErrorKind::NotFound => {
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
                Err(error) => panic!("unexpected installation lookup error: {error:?}"),
            }
        }
    })
    .await
    .expect("official installation should be synchronized");
    assert_eq!(installation.plugin_key(), "raindrop.ai-content");
    assert_eq!(installation.version(), "1.0.0");
    assert_eq!(installation.abi_version(), "raindrop:content-plugin@1.0.0");
    assert_eq!(installation.system_state(), PluginSystemState::Enabled);

    handle.shutdown();
    tokio::time::timeout(Duration::from_secs(2), task)
        .await
        .expect("ready inert runtime should stop promptly")
        .expect("ready inert runtime task should join")
        .expect("ready inert runtime shutdown should succeed");
}

#[tokio::test]
async fn valid_keyring_starts_and_stops_production_lanes() {
    let fixture = ProductionFixture::new("valid-keyring").await;
    let job_id = fixture.enqueue("valid-keyring").await;
    let key = SecretString::from(format!("primary:{}", URL_SAFE_NO_PAD.encode([0x41_u8; 32])));
    let (runtime, handle) = ProductionContentRuntime::new(fixture.setup.clone(), vec![key])
        .expect("production runtime composition");
    let task = tokio::spawn(runtime.run());
    handle.notify();

    fixture
        .wait_for_status(&job_id, JobStatus::Failed, Duration::from_secs(4))
        .await;

    handle.shutdown();
    tokio::time::timeout(Duration::from_secs(2), task)
        .await
        .expect("active production runtime should stop promptly")
        .expect("active production runtime task should join")
        .expect("active production runtime shutdown should succeed");
}

#[tokio::test]
async fn empty_keyring_synchronizes_installation_without_claiming_jobs() {
    let fixture = ProductionFixture::new("empty-keyring").await;
    let job_id = fixture.enqueue("empty-keyring").await;
    let (runtime, handle) = ProductionContentRuntime::new(fixture.setup.clone(), Vec::new())
        .expect("production runtime composition");
    let task = tokio::spawn(runtime.run());
    handle.notify();

    fixture.wait_for_installation().await;
    tokio::time::pause();
    handle.notify();
    tokio::time::advance(Duration::from_secs(10)).await;
    for _ in 0..16 {
        tokio::task::yield_now().await;
    }
    tokio::time::resume();
    let job = fixture
        .repository
        .get_job(USER_A_ID, &job_id)
        .await
        .expect("inert production job snapshot");
    assert_eq!(job.status(), JobStatus::Queued);
    assert_eq!(job.attempts(), 0);

    handle.shutdown();
    tokio::time::timeout(Duration::from_secs(2), task)
        .await
        .expect("inert production runtime should stop promptly")
        .expect("inert production runtime task should join")
        .expect("inert production runtime shutdown should succeed");
}

#[tokio::test]
async fn corrupt_installation_fails_closed_with_redacted_configuration_error() {
    let fixture = ProductionFixture::new("corrupt-installation").await;
    let database = fixture
        .setup
        .database()
        .expect("ready setup should expose its database");
    let sentinel = "sensitive-corrupt-distribution";
    let now = datetime!(2026-07-19 12:00:00 UTC);
    plugin_installation::ActiveModel {
        id: Set("00000000-0000-4000-8000-000000000990".to_owned()),
        plugin_key: Set("raindrop.ai-content".to_owned()),
        version: Set("1.0.0".to_owned()),
        abi_version: Set("raindrop:content-plugin@1.0.0".to_owned()),
        distribution: Set(sentinel.to_owned()),
        component_digest: Set(HASH_A.to_owned()),
        manifest_json: Set("{}".to_owned()),
        signature_key_id: Set("fixture-key".to_owned()),
        signature: Set("fixture-signature".to_owned()),
        system_state: Set("ENABLED".to_owned()),
        revision: Set(0),
        installed_at: Set(now),
        updated_at: Set(now),
    }
    .insert(&database)
    .await
    .expect("corrupt installation fixture should insert");

    let (runtime, _) = ProductionContentRuntime::new(fixture.setup, Vec::new())
        .expect("embedded production runtime should compose before database synchronization");
    let error = tokio::time::timeout(Duration::from_secs(2), runtime.run())
        .await
        .expect("structural synchronization failure should be immediate")
        .expect_err("corrupt installation must fail closed");
    assert_eq!(
        error.kind(),
        raindrop::content::worker::ContentWorkerErrorKind::InvalidConfiguration
    );
    assert!(!format!("{error:?}").contains(sentinel));
    assert!(!error.to_string().contains(sentinel));
}

struct ProductionFixture {
    _data: tempfile::TempDir,
    setup: SetupService,
    repository: ContentRepository,
}

impl ProductionFixture {
    async fn new(name: &str) -> Self {
        let data = tempfile::tempdir().expect("temporary production runtime fixture");
        let database_url = format!(
            "sqlite://{}?mode=rwc",
            data.path()
                .join(format!("content-production-{name}.db"))
                .display()
        );
        let database = connect_for_contract(SecretString::from(database_url)).await;
        migrate(&database)
            .await
            .expect("production runtime database migration");
        let now = datetime!(2026-07-19 12:00:00 UTC);
        insert_user(&database, USER_A_ID, &format!("production-{name}")).await;
        insert_feed(&database, now).await;
        insert_subscription(&database, SUBSCRIPTION_A_ID, USER_A_ID, now).await;
        insert_entry(
            &database,
            ENTRY_A_ID,
            1,
            "production-content-entry",
            HASH_A,
            Some(1_752_926_400_000_000),
            now,
        )
        .await;
        Self {
            setup: SetupService::ready(data.path(), None, database.clone()),
            repository: ContentRepository::new(database),
            _data: data,
        }
    }

    async fn enqueue(&self, key: &str) -> String {
        let identity = ArtifactIdentity::new(ArtifactIdentityInput {
            user_id: USER_A_ID.to_owned(),
            entry_id: ENTRY_A_ID.to_owned(),
            kind: ArtifactKind::AiSummary,
            target_locale: None,
            entry_content_hash: HASH_D.to_owned(),
            input_hash: HASH_B.to_owned(),
            config_hash: HASH_C.to_owned(),
            plugin_key: "raindrop.ai-content".to_owned(),
            plugin_version: "1.0.0".to_owned(),
            component_digest: HASH_A.to_owned(),
            provider_binding_id: PROVIDER_ID.to_owned(),
            provider_kind: ProviderKind::OpenAiResponses,
            provider_model: "gpt-5-mini".to_owned(),
            provider_revision: 0,
            prompt_version: "raindrop-summary-v1".to_owned(),
            schema_id: "raindrop://schemas/artifacts/ai-summary/v1".to_owned(),
            mcp_provenance_hash: HASH_A.to_owned(),
        })
        .expect("production artifact identity");
        let request = EnqueueContentJob::new(EnqueueContentJobInput {
            operation: ContentJobOperation::Summarize,
            trigger: ContentJobTrigger::ManualApi,
            identity,
            idempotency_key: format!("production-{key}"),
            call_chain_id: format!("production-chain-{key}"),
            remaining_depth: 2,
        })
        .expect("production enqueue request");
        match self
            .repository
            .enqueue(request)
            .await
            .expect("production content enqueue")
        {
            EnqueueResult::Queued(job) => job.id().to_owned(),
            other => panic!("expected queued production job, got {other:?}"),
        }
    }

    async fn wait_for_status(&self, job_id: &str, status: JobStatus, wait: Duration) {
        tokio::time::timeout(wait, async {
            loop {
                let job = self
                    .repository
                    .get_job(USER_A_ID, job_id)
                    .await
                    .expect("production job snapshot");
                if job.status() == status {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("production job should reach expected status");
    }

    async fn wait_for_installation(&self) {
        let repository = PluginRegistryRepository::new(
            self.setup
                .database()
                .expect("ready setup should expose its database"),
        );
        tokio::time::timeout(Duration::from_secs(4), async {
            loop {
                match repository.get_installation("raindrop.ai-content").await {
                    Ok(_) => return,
                    Err(error) if error.kind() == PluginRegistryErrorKind::NotFound => {
                        tokio::time::sleep(Duration::from_millis(10)).await;
                    }
                    Err(error) => panic!("unexpected installation lookup error: {error:?}"),
                }
            }
        })
        .await
        .expect("official installation should be synchronized");
    }
}
