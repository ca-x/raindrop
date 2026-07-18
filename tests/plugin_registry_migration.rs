#[allow(dead_code)]
mod support;

use raindrop::db::{
    entities::{plugin_capability_grant, plugin_config, plugin_installation, plugin_kv, user},
    migrate, rollback,
};
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter,
    QueryResult, TryGetable,
};
use sea_orm_migration::SchemaManager;
use secrecy::SecretString;
use support::database::{HASH_A, HASH_B, USER_A_ID, connect_for_contract, insert_user};
use time::macros::datetime;

const PLUGIN_ID: &str = "00000000-0000-4000-8000-000000001101";
const CONFIG_ID: &str = "00000000-0000-4000-8000-000000001102";
const GRANT_ID: &str = "00000000-0000-4000-8000-000000001103";

const EXPECTED_INDEXES: &[(&str, &str)] = &[
    ("plugin_installations", "uq_plugin_installations_key"),
    ("plugin_installations", "idx_plugin_installations_state"),
    ("plugin_configs", "uq_plugin_configs_owner"),
    ("plugin_configs", "idx_plugin_configs_owner_enabled"),
    ("plugin_capability_grants", "uq_plugin_grants_key"),
    ("plugin_capability_grants", "idx_plugin_grants_owner_active"),
    ("plugin_capability_grants", "idx_plugin_grants_resource"),
    ("plugin_kv", "idx_plugin_kv_owner_updated"),
];

#[tokio::test]
async fn sqlite_plugin_registry_migration_entities_and_foreign_keys_round_trip() {
    let data = tempfile::tempdir().expect("temporary directory should create");
    let url = format!(
        "sqlite://{}?mode=rwc",
        data.path().join("plugin-registry.db").display()
    );
    let database = connect_for_contract(SecretString::from(url)).await;
    migrate(&database).await.expect("migrations should apply");

    assert_schema(&database, true).await;
    let now = datetime!(2026-07-19 12:00:00 UTC);
    insert_user(&database, USER_A_ID, "plugin-user").await;

    plugin_installation::ActiveModel {
        id: Set(PLUGIN_ID.to_owned()),
        plugin_key: Set("raindrop.ai-content".to_owned()),
        version: Set("1.0.0".to_owned()),
        abi_version: Set("raindrop:content-plugin@1.0.0".to_owned()),
        distribution: Set("BUNDLED_OFFICIAL".to_owned()),
        component_digest: Set(HASH_A.to_owned()),
        manifest_json: Set("{\"manifestVersion\":1}".to_owned()),
        signature_key_id: Set("raindrop-test-release-2026".to_owned()),
        signature: Set("test-signature".to_owned()),
        system_state: Set("ENABLED".to_owned()),
        revision: Set(0),
        installed_at: Set(now),
        updated_at: Set(now),
    }
    .insert(&database)
    .await
    .expect("plugin installation should insert");

    plugin_config::ActiveModel {
        id: Set(CONFIG_ID.to_owned()),
        plugin_id: Set(PLUGIN_ID.to_owned()),
        owner_user_id: Set(USER_A_ID.to_owned()),
        schema_version: Set(1),
        config_json: Set("{\"schemaVersion\":1}".to_owned()),
        config_hash: Set(HASH_B.to_owned()),
        is_enabled: Set(true),
        revision: Set(0),
        created_at: Set(now),
        updated_at: Set(now),
    }
    .insert(&database)
    .await
    .expect("plugin config should insert");

    plugin_capability_grant::ActiveModel {
        id: Set(GRANT_ID.to_owned()),
        plugin_id: Set(PLUGIN_ID.to_owned()),
        owner_user_id: Set(USER_A_ID.to_owned()),
        capability: Set("ai.generate_structured".to_owned()),
        operation: Set("SUMMARIZE".to_owned()),
        resource_type: Set("AI_PROVIDER".to_owned()),
        resource_id: Set("00000000-0000-4000-8000-000000000901".to_owned()),
        grant_key_hash: Set(HASH_A.to_owned()),
        constraints_json: Set("{\"schemaVersion\":1}".to_owned()),
        revision: Set(0),
        created_at: Set(now),
        updated_at: Set(now),
        revoked_at: Set(None),
    }
    .insert(&database)
    .await
    .expect("plugin grant should insert");

    plugin_kv::ActiveModel {
        plugin_id: Set(PLUGIN_ID.to_owned()),
        owner_user_id: Set(USER_A_ID.to_owned()),
        key: Set("cache.summary".to_owned()),
        value: Set(b"opaque-plugin-value".to_vec()),
        value_size_bytes: Set(19),
        revision: Set(0),
        created_at: Set(now),
        updated_at: Set(now),
    }
    .insert(&database)
    .await
    .expect("plugin KV should insert");

    let installation = plugin_installation::Entity::find_by_id(PLUGIN_ID)
        .one(&database)
        .await
        .expect("installation should query")
        .expect("installation should exist");
    assert_eq!(installation.plugin_key, "raindrop.ai-content");
    assert_eq!(installation.revision, 0);

    let config = plugin_config::Entity::find_by_id(CONFIG_ID)
        .one(&database)
        .await
        .expect("config should query")
        .expect("config should exist");
    assert!(config.is_enabled);

    let grant = plugin_capability_grant::Entity::find_by_id(GRANT_ID)
        .one(&database)
        .await
        .expect("grant should query")
        .expect("grant should exist");
    assert_eq!(grant.capability, "ai.generate_structured");

    let stored_kv = plugin_kv::Entity::find_by_id((
        PLUGIN_ID.to_owned(),
        USER_A_ID.to_owned(),
        "cache.summary".to_owned(),
    ))
    .one(&database)
    .await
    .expect("KV should query")
    .expect("KV should exist");
    assert_eq!(stored_kv.value, b"opaque-plugin-value");

    let duplicate_config = plugin_config::ActiveModel {
        id: Set("00000000-0000-4000-8000-000000001104".to_owned()),
        plugin_id: Set(PLUGIN_ID.to_owned()),
        owner_user_id: Set(USER_A_ID.to_owned()),
        schema_version: Set(1),
        config_json: Set("{\"schemaVersion\":1}".to_owned()),
        config_hash: Set(HASH_B.to_owned()),
        is_enabled: Set(true),
        revision: Set(0),
        created_at: Set(now),
        updated_at: Set(now),
    }
    .insert(&database)
    .await;
    assert!(duplicate_config.is_err());

    assert!(
        plugin_installation::Entity::delete_by_id(PLUGIN_ID)
            .exec(&database)
            .await
            .is_err(),
        "installation deletion should be restricted while child rows exist"
    );

    user::Entity::delete_by_id(USER_A_ID)
        .exec(&database)
        .await
        .expect("owner should delete");
    assert_eq!(
        plugin_config::Entity::find()
            .filter(plugin_config::Column::OwnerUserId.eq(USER_A_ID))
            .count(&database)
            .await
            .expect("configs should count"),
        0
    );
    assert_eq!(
        plugin_capability_grant::Entity::find()
            .filter(plugin_capability_grant::Column::OwnerUserId.eq(USER_A_ID))
            .count(&database)
            .await
            .expect("grants should count"),
        0
    );
    assert_eq!(
        plugin_kv::Entity::find()
            .filter(plugin_kv::Column::OwnerUserId.eq(USER_A_ID))
            .count(&database)
            .await
            .expect("KV rows should count"),
        0
    );
    assert!(
        plugin_installation::Entity::find_by_id(PLUGIN_ID)
            .one(&database)
            .await
            .expect("installation should query")
            .is_some()
    );

    rollback(&database).await.expect("rollback should succeed");
    assert_schema(&database, false).await;
    migrate(&database).await.expect("migrations should reapply");
    assert_schema(&database, true).await;
    database.close().await.expect("database should close");
}

async fn assert_schema(database: &sea_orm::DatabaseConnection, expected: bool) {
    let manager = SchemaManager::new(database);
    for table in [
        "plugin_installations",
        "plugin_configs",
        "plugin_capability_grants",
        "plugin_kv",
    ] {
        assert_eq!(
            manager
                .has_table(table)
                .await
                .expect("table should inspect"),
            expected,
            "table {table} presence"
        );
    }
    if !expected {
        return;
    }

    for (table, columns) in [
        (
            "plugin_installations",
            &[
                "id",
                "plugin_key",
                "version",
                "abi_version",
                "distribution",
                "component_digest",
                "manifest_json",
                "signature_key_id",
                "signature",
                "system_state",
                "revision",
                "installed_at",
                "updated_at",
            ][..],
        ),
        (
            "plugin_configs",
            &[
                "id",
                "plugin_id",
                "owner_user_id",
                "schema_version",
                "config_json",
                "config_hash",
                "is_enabled",
                "revision",
                "created_at",
                "updated_at",
            ][..],
        ),
        (
            "plugin_capability_grants",
            &[
                "id",
                "plugin_id",
                "owner_user_id",
                "capability",
                "operation",
                "resource_type",
                "resource_id",
                "grant_key_hash",
                "constraints_json",
                "revision",
                "created_at",
                "updated_at",
                "revoked_at",
            ][..],
        ),
        (
            "plugin_kv",
            &[
                "plugin_id",
                "owner_user_id",
                "key",
                "value",
                "value_size_bytes",
                "revision",
                "created_at",
                "updated_at",
            ][..],
        ),
    ] {
        for column in columns {
            assert!(
                manager
                    .has_column(table, column)
                    .await
                    .unwrap_or_else(|error| panic!(
                        "column {table}.{column} should inspect: {error}"
                    )),
                "column {table}.{column} should exist"
            );
        }
    }

    for (table, index) in EXPECTED_INDEXES {
        assert!(
            manager
                .has_index(table, index)
                .await
                .unwrap_or_else(|error| panic!("index {index} should inspect: {error}")),
            "index {index} should exist"
        );
    }
}

#[allow(dead_code)]
fn index_name(row: QueryResult) -> String {
    String::try_get(&row, "", "name").expect("index name should decode")
}
