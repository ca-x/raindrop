#[allow(dead_code)]
mod support;

use raindrop::{
    db::{
        entities::{plugin_config, plugin_kv, user},
        migrate,
    },
    plugins::{
        CapabilityGrantInput, PluginRegistryErrorKind, PluginRegistryRepository, PluginSystemState,
    },
};
use sea_orm::{ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, sea_query::Expr};
use secrecy::SecretString;
use serde_json::json;
use support::{
    database::{USER_A_ID, USER_B_ID, connect_for_contract, insert_user},
    plugin::signed_bundle,
};

const PLUGIN_KEY: &str = "raindrop.ai-content";
const PROVIDER_ID: &str = "00000000-0000-4000-8000-000000000901";

#[tokio::test]
async fn bundled_sync_and_user_config_are_idempotent_revisioned_and_fail_closed() {
    let (_data, database, repository) = setup_registry("plugin-config.db").await;
    let first_bundle = signed_bundle("1.0.0", b"official component v1");
    let first = repository
        .sync_bundled(&first_bundle)
        .await
        .expect("first bundled sync should insert");
    assert_eq!(first.plugin_key(), PLUGIN_KEY);
    assert_eq!(first.version(), "1.0.0");
    assert_eq!(first.system_state(), PluginSystemState::Enabled);
    assert_eq!(first.revision(), 0);

    let replay = repository
        .sync_bundled(&first_bundle)
        .await
        .expect("exact bundled replay should reuse");
    assert_eq!(replay.id(), first.id());
    assert_eq!(replay.revision(), 0);

    let upgraded_bundle = signed_bundle("1.0.1", b"official component v1.0.1");
    let upgraded = repository
        .sync_bundled(&upgraded_bundle)
        .await
        .expect("verified bundled upgrade should replace descriptor");
    assert_eq!(upgraded.id(), first.id());
    assert_eq!(upgraded.version(), "1.0.1");
    assert_ne!(upgraded.component_digest(), first.component_digest());
    assert_eq!(upgraded.revision(), 1);
    assert_eq!(
        repository
            .get_installation(PLUGIN_KEY)
            .await
            .expect("installation should read")
            .revision(),
        1
    );

    let config = repository
        .replace_ai_config(PLUGIN_KEY, USER_A_ID, None, true, &valid_config())
        .await
        .expect("new user config should insert");
    assert!(config.is_enabled());
    assert_eq!(config.revision(), 0);
    assert_eq!(config.config_hash().len(), 64);
    assert_eq!(
        repository
            .get_ai_config(PLUGIN_KEY, USER_A_ID)
            .await
            .expect("config should read")
            .expect("config should exist")
            .id(),
        config.id()
    );
    assert!(
        repository
            .get_ai_config(PLUGIN_KEY, USER_B_ID)
            .await
            .expect("other user lookup should succeed")
            .is_none()
    );

    let updated = repository
        .replace_ai_config(PLUGIN_KEY, USER_A_ID, Some(0), false, &valid_config())
        .await
        .expect("matching revision should update");
    assert!(!updated.is_enabled());
    assert_eq!(updated.revision(), 1);
    assert_error_kind(
        repository
            .replace_ai_config(PLUGIN_KEY, USER_A_ID, Some(0), true, &valid_config())
            .await,
        PluginRegistryErrorKind::RevisionConflict,
    );

    plugin_config::Entity::update_many()
        .col_expr(
            plugin_config::Column::ConfigHash,
            Expr::value("0".repeat(64)),
        )
        .filter(plugin_config::Column::Id.eq(config.id()))
        .exec(&database)
        .await
        .expect("test corruption should update");
    assert_error_kind(
        repository.get_ai_config(PLUGIN_KEY, USER_A_ID).await,
        PluginRegistryErrorKind::CorruptData,
    );

    user::Entity::update_many()
        .col_expr(user::Column::IsDisabled, Expr::value(true))
        .filter(user::Column::Id.eq(USER_B_ID))
        .exec(&database)
        .await
        .expect("test user should disable");
    assert_error_kind(
        repository
            .replace_ai_config(PLUGIN_KEY, USER_B_ID, None, true, &valid_config())
            .await,
        PluginRegistryErrorKind::NotFound,
    );
}

#[tokio::test]
async fn capability_grants_are_exact_revisioned_revocable_and_tenant_scoped() {
    let (_data, _database, repository) = setup_registry("plugin-grants.db").await;
    repository
        .sync_bundled(&signed_bundle("1.0.0", b"grant component"))
        .await
        .unwrap();

    let created = repository
        .grant_capability(grant_input(None, br#"{"schemaVersion":1}"#.to_vec()))
        .await
        .expect("grant should insert");
    assert_eq!(created.capability(), "ai.generate_structured");
    assert_eq!(created.operation(), "SUMMARIZE");
    assert_eq!(created.resource_id(), PROVIDER_ID);
    assert_eq!(created.revision(), 0);
    assert!(!created.is_revoked());

    let updated = repository
        .grant_capability(grant_input(
            Some(0),
            br#"{"maxOutputTokens":1024,"schemaVersion":1}"#.to_vec(),
        ))
        .await
        .expect("matching revision should regrant");
    assert_eq!(updated.id(), created.id());
    assert_eq!(updated.revision(), 1);

    assert_error_kind(
        repository
            .grant_capability(grant_input(Some(0), br#"{"schemaVersion":1}"#.to_vec()))
            .await,
        PluginRegistryErrorKind::RevisionConflict,
    );
    assert_eq!(
        repository
            .list_active_grants(PLUGIN_KEY, USER_A_ID)
            .await
            .expect("active grants should list")
            .len(),
        1
    );
    assert!(
        repository
            .list_active_grants(PLUGIN_KEY, USER_B_ID)
            .await
            .expect("other user grants should list")
            .is_empty()
    );

    let revoked = repository
        .revoke_capability(PLUGIN_KEY, USER_A_ID, created.id(), 1)
        .await
        .expect("grant should revoke");
    assert!(revoked.is_revoked());
    assert_eq!(revoked.revision(), 2);
    assert!(
        repository
            .list_active_grants(PLUGIN_KEY, USER_A_ID)
            .await
            .unwrap()
            .is_empty()
    );

    let regranted = repository
        .grant_capability(grant_input(Some(2), br#"{"schemaVersion":1}"#.to_vec()))
        .await
        .expect("revoked grant should re-enable in place");
    assert_eq!(regranted.id(), created.id());
    assert_eq!(regranted.revision(), 3);
    assert!(!regranted.is_revoked());

    let secret = "rd-secret-capability";
    let mut secret_input = grant_input(None, format!(r#"{{"apiKey":"{secret}"}}"#).into_bytes());
    secret_input.resource_id = "00000000-0000-4000-8000-000000000902".to_owned();
    let error = repository
        .grant_capability(secret_input)
        .await
        .expect_err("secret-like constraints should fail");
    assert_eq!(error.kind(), PluginRegistryErrorKind::InvalidInput);
    assert!(!format!("{error:?} {error}").contains(secret));
}

#[tokio::test]
async fn plugin_kv_enforces_cas_value_count_and_total_byte_quotas() {
    let (_data, database, repository) = setup_registry("plugin-kv.db").await;
    repository
        .sync_bundled(&signed_bundle("1.0.0", b"KV component"))
        .await
        .unwrap();

    let created = repository
        .put_kv(
            PLUGIN_KEY,
            USER_A_ID,
            "cache.summary",
            None,
            b"value-v1".to_vec(),
        )
        .await
        .expect("KV should create");
    assert_eq!(created.revision(), 0);
    assert_eq!(created.value(), b"value-v1");
    assert_eq!(
        repository
            .get_kv(PLUGIN_KEY, USER_A_ID, "cache.summary")
            .await
            .expect("KV should read")
            .expect("KV should exist")
            .value(),
        b"value-v1"
    );
    assert!(
        repository
            .get_kv(PLUGIN_KEY, USER_B_ID, "cache.summary")
            .await
            .expect("other user KV read should succeed")
            .is_none()
    );

    let replaced = repository
        .put_kv(
            PLUGIN_KEY,
            USER_A_ID,
            "cache.summary",
            Some(0),
            b"value-v2-longer".to_vec(),
        )
        .await
        .expect("KV replacement should use matching revision");
    assert_eq!(replaced.revision(), 1);
    assert_error_kind(
        repository
            .put_kv(
                PLUGIN_KEY,
                USER_A_ID,
                "cache.summary",
                Some(0),
                b"stale".to_vec(),
            )
            .await,
        PluginRegistryErrorKind::RevisionConflict,
    );
    repository
        .delete_kv(PLUGIN_KEY, USER_A_ID, "cache.summary", 1)
        .await
        .expect("KV delete should use matching revision");

    for index in 0..16 {
        repository
            .put_kv(
                PLUGIN_KEY,
                USER_A_ID,
                &format!("total.{index:02}"),
                None,
                vec![index as u8; 64 * 1024],
            )
            .await
            .expect("exact total byte quota should fit");
    }
    assert_error_kind(
        repository
            .put_kv(PLUGIN_KEY, USER_A_ID, "total.overflow", None, vec![1])
            .await,
        PluginRegistryErrorKind::QuotaExceeded,
    );
    assert_error_kind(
        repository
            .put_kv(
                PLUGIN_KEY,
                USER_A_ID,
                "value.too-large",
                None,
                vec![0; 64 * 1024 + 1],
            )
            .await,
        PluginRegistryErrorKind::QuotaExceeded,
    );
    for index in 0..16 {
        repository
            .delete_kv(PLUGIN_KEY, USER_A_ID, &format!("total.{index:02}"), 0)
            .await
            .unwrap();
    }

    for index in 0..128 {
        repository
            .put_kv(
                PLUGIN_KEY,
                USER_A_ID,
                &format!("count.{index:03}"),
                None,
                vec![index as u8],
            )
            .await
            .expect("exact key-count quota should fit");
    }
    assert_error_kind(
        repository
            .put_kv(PLUGIN_KEY, USER_A_ID, "count.overflow", None, vec![1])
            .await,
        PluginRegistryErrorKind::QuotaExceeded,
    );

    plugin_kv::Entity::update_many()
        .col_expr(plugin_kv::Column::ValueSizeBytes, Expr::value(99))
        .filter(plugin_kv::Column::PluginId.eq(created.plugin_id()))
        .filter(plugin_kv::Column::OwnerUserId.eq(USER_A_ID))
        .filter(plugin_kv::Column::Key.eq("count.000"))
        .exec(&database)
        .await
        .expect("test KV corruption should update");
    assert_error_kind(
        repository.get_kv(PLUGIN_KEY, USER_A_ID, "count.000").await,
        PluginRegistryErrorKind::CorruptData,
    );
}

#[tokio::test]
async fn concurrent_kv_writers_converge_at_the_user_key_quota() {
    let (_data, _database, repository) = setup_registry("plugin-kv-concurrency.db").await;
    repository
        .sync_bundled(&signed_bundle("1.0.0", b"concurrency component"))
        .await
        .unwrap();
    for index in 0..127 {
        repository
            .put_kv(
                PLUGIN_KEY,
                USER_A_ID,
                &format!("slot.{index:03}"),
                None,
                vec![1],
            )
            .await
            .unwrap();
    }

    let left_repository = repository.clone();
    let right_repository = repository.clone();
    let (left, right) = tokio::join!(
        left_repository.put_kv(PLUGIN_KEY, USER_A_ID, "slot.left", None, vec![1]),
        right_repository.put_kv(PLUGIN_KEY, USER_A_ID, "slot.right", None, vec![1]),
    );
    let outcomes = [left, right];
    assert_eq!(outcomes.iter().filter(|result| result.is_ok()).count(), 1);
    assert_eq!(
        outcomes
            .iter()
            .filter(|result| {
                result
                    .as_ref()
                    .is_err_and(|error| error.kind() == PluginRegistryErrorKind::QuotaExceeded)
            })
            .count(),
        1
    );
}

async fn setup_registry(
    filename: &str,
) -> (
    tempfile::TempDir,
    DatabaseConnection,
    PluginRegistryRepository,
) {
    let data = tempfile::tempdir().unwrap();
    let url = format!("sqlite://{}?mode=rwc", data.path().join(filename).display());
    let database = connect_for_contract(SecretString::from(url)).await;
    migrate(&database).await.unwrap();
    insert_user(&database, USER_A_ID, "plugin-repository-a").await;
    insert_user(&database, USER_B_ID, "plugin-repository-b").await;
    let repository = PluginRegistryRepository::new(database.clone());
    (data, database, repository)
}

fn grant_input(expected_revision: Option<u64>, constraints_json: Vec<u8>) -> CapabilityGrantInput {
    CapabilityGrantInput {
        plugin_key: PLUGIN_KEY.to_owned(),
        owner_user_id: USER_A_ID.to_owned(),
        expected_revision,
        capability: "ai.generate_structured".to_owned(),
        operation: "SUMMARIZE".to_owned(),
        resource_type: "AI_PROVIDER".to_owned(),
        resource_id: PROVIDER_ID.to_owned(),
        constraints_json,
    }
}

fn valid_config() -> Vec<u8> {
    serde_json::to_vec(&json!({
        "schemaVersion": 1,
        "operations": {
            "summarize": {
                "enabled": true,
                "providerId": PROVIDER_ID,
                "style": "BALANCED",
                "maxOutputTokens": 1024,
                "mcp": {
                    "mode": "DISABLED",
                    "failurePolicy": "FAIL_OPEN",
                    "maxToolCalls": 0,
                    "tools": []
                }
            },
            "translate": {
                "enabled": true,
                "providerId": "00000000-0000-4000-8000-000000000902",
                "defaultTargetLocale": "zh-CN",
                "maxOutputTokens": 2048,
                "mcp": {
                    "mode": "DISABLED",
                    "failurePolicy": "FAIL_CLOSED",
                    "maxToolCalls": 0,
                    "tools": []
                }
            }
        },
        "automatic": {
            "enabled": false,
            "operations": ["SUMMARIZE"],
            "allSubscribedFeeds": false,
            "feedIds": [],
            "categoryIds": []
        }
    }))
    .unwrap()
}

fn assert_error_kind<T: std::fmt::Debug>(
    result: Result<T, raindrop::plugins::PluginRegistryError>,
    expected: PluginRegistryErrorKind,
) {
    assert_eq!(result.expect_err("operation should fail").kind(), expected);
}
