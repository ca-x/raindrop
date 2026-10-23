#[allow(dead_code)]
mod support;

use raindrop::{
    db::{entities::plugin_installation, migrate, rollback},
    plugins::{
        CapabilityGrantInput, PluginRegistryErrorKind, PluginRegistryRepository, PluginSystemState,
    },
};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, sea_query::Expr};
use secrecy::SecretString;
use serde_json::json;
use support::{
    database::{USER_A_ID, USER_B_ID, connect_for_contract, insert_user},
    plugin::{signed_bundle, signed_legacy_bundle},
};

const PLUGIN_KEY: &str = "raindrop.ai-content";
const PROVIDER_ID: &str = "00000000-0000-4000-8000-000000000901";

#[tokio::test]
async fn sqlite_plugin_registry_backend_contract() {
    let data = tempfile::tempdir().unwrap();
    let url = format!(
        "sqlite://{}?mode=rwc",
        data.path().join("plugin-registry-backend.db").display()
    );
    backend_contract(&url).await;
}

#[tokio::test]
async fn postgres_plugin_registry_backend_contract() {
    let Ok(url) = std::env::var("RAINDROP_TEST_POSTGRES_URL") else {
        eprintln!("postgres plugin registry contract skipped: database URL is not configured");
        return;
    };
    backend_contract(&url).await;
}

#[tokio::test]
async fn mysql_plugin_registry_backend_contract() {
    let Ok(url) = std::env::var("RAINDROP_TEST_MYSQL_URL") else {
        eprintln!("mysql plugin registry contract skipped: database URL is not configured");
        return;
    };
    backend_contract(&url).await;
}

async fn backend_contract(url: &str) {
    let database = connect_for_contract(SecretString::from(url.to_owned())).await;
    rollback(&database)
        .await
        .expect("dedicated plugin registry database should reset");
    migrate(&database)
        .await
        .expect("plugin registry migrations should apply");
    insert_user(&database, USER_A_ID, "plugin-backend-user").await;
    insert_user(&database, USER_B_ID, "plugin-backend-user-b").await;

    let repository = PluginRegistryRepository::new(database.clone());
    let installation = repository
        .sync_bundled(&signed_legacy_bundle("1.0.0", b"legacy backend component"))
        .await
        .expect("backend bundled plugin should sync");
    assert_eq!(installation.plugin_key(), PLUGIN_KEY);

    let config = repository
        .replace_ai_config(PLUGIN_KEY, USER_A_ID, None, true, &valid_config())
        .await
        .expect("backend config should persist");
    assert_eq!(config.revision(), 0);

    let grant = repository
        .grant_capability(CapabilityGrantInput {
            plugin_key: PLUGIN_KEY.to_owned(),
            owner_user_id: USER_A_ID.to_owned(),
            expected_revision: None,
            capability: "ai.generate_structured".to_owned(),
            operation: "SUMMARIZE".to_owned(),
            resource_type: "AI_PROVIDER".to_owned(),
            resource_id: PROVIDER_ID.to_owned(),
            constraints_json: br#"{"schemaVersion":1}"#.to_vec(),
        })
        .await
        .expect("backend grant should persist");
    assert_eq!(grant.revision(), 0);

    let kv = repository
        .put_kv(
            PLUGIN_KEY,
            USER_A_ID,
            "backend.value",
            None,
            b"portable".to_vec(),
        )
        .await
        .expect("backend KV should persist");
    assert_eq!(kv.value(), b"portable");
    assert_eq!(
        repository
            .get_tambo_preferences(USER_A_ID)
            .await
            .unwrap()
            .revision,
        None
    );
    let interaction = repository
        .replace_tambo_preferences(USER_A_ID, None, true)
        .await
        .unwrap();
    assert_eq!(interaction.revision, Some(0));
    assert!(
        !repository
            .get_tambo_preferences(USER_B_ID)
            .await
            .unwrap()
            .enabled
    );
    let conflict = repository
        .replace_tambo_preferences(USER_A_ID, None, false)
        .await
        .unwrap_err();
    assert_eq!(conflict.kind(), PluginRegistryErrorKind::RevisionConflict);

    plugin_installation::Entity::update_many()
        .col_expr(
            plugin_installation::Column::SystemState,
            Expr::value("DISABLED"),
        )
        .filter(plugin_installation::Column::Id.eq(installation.id()))
        .exec(&database)
        .await
        .unwrap();
    let upgraded = repository
        .sync_bundled(&signed_bundle("1.0.0", b"native backend identity"))
        .await
        .unwrap();
    assert_eq!(upgraded.id(), installation.id());
    assert_eq!(upgraded.system_state(), PluginSystemState::Disabled);
    assert_eq!(upgraded.abi_version(), raindrop_ai_content::ABI);
    assert_eq!(
        repository
            .get_ai_config(PLUGIN_KEY, USER_A_ID)
            .await
            .unwrap()
            .unwrap()
            .config_hash(),
        config.config_hash()
    );
    assert_eq!(
        repository
            .get_kv(PLUGIN_KEY, USER_A_ID, "backend.value")
            .await
            .unwrap()
            .unwrap()
            .value(),
        b"portable"
    );
    assert!(
        repository
            .get_tambo_preferences(USER_A_ID)
            .await
            .unwrap()
            .enabled
    );
    assert_eq!(
        repository
            .list_active_grants(PLUGIN_KEY, USER_A_ID)
            .await
            .expect("backend grants should list")
            .len(),
        1
    );

    drop(repository);
    database.close().await.expect("database should close");
    let reopened = connect_for_contract(SecretString::from(url.to_owned())).await;
    let repository = PluginRegistryRepository::new(reopened.clone());
    assert_eq!(
        repository.get_tambo_preferences(USER_A_ID).await.unwrap(),
        interaction
    );
    let disabled = repository
        .replace_tambo_preferences(USER_A_ID, interaction.revision, false)
        .await
        .unwrap();
    assert_eq!(disabled.revision, Some(1));
    assert!(!disabled.enabled);
    assert_eq!(
        repository
            .get_ai_config(PLUGIN_KEY, USER_A_ID)
            .await
            .unwrap()
            .unwrap()
            .revision(),
        0
    );
    assert_eq!(
        repository
            .list_active_grants(PLUGIN_KEY, USER_A_ID)
            .await
            .unwrap()
            .len(),
        1
    );
    reopened.close().await.unwrap();
}

fn valid_config() -> Vec<u8> {
    serde_json::to_vec(&json!({
        "schemaVersion": 1,
        "operations": {
            "summarize": {
                "enabled": true,
                "providerId": PROVIDER_ID,
                "style": "CONCISE",
                "maxOutputTokens": 512,
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
                "maxOutputTokens": 1024,
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
