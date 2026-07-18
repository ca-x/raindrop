#[allow(dead_code)]
mod support;

use raindrop::db::{
    entities::{ai_provider, user},
    migrate, rollback,
};
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, DatabaseConnection, EntityTrait, PaginatorTrait,
};
use sea_orm_migration::SchemaManager;
use secrecy::SecretString;
use support::database::{
    AI_PROVIDER_INSTANCE_ID, AI_PROVIDER_USER_ID, USER_A_ID, connect_for_contract, insert_user,
};
use tempfile::tempdir;
use time::{OffsetDateTime, macros::datetime};

const STORED_AT: OffsetDateTime = datetime!(2040-02-03 04:05:06.123456 UTC);
const ENCRYPTED_SECRET: &str = "rdsec1.primary.AAAAAAAAAAAAAAAA.ciphertext-placeholder";

#[tokio::test]
async fn sqlite_ai_provider_storage_contract() {
    let data = tempdir().expect("temporary directory should be created");
    let url = format!(
        "sqlite://{}?mode=rwc",
        data.path().join("ai-provider.db").display()
    );

    ai_provider_storage_contract(SecretString::from(url)).await;
}

#[tokio::test]
async fn postgres_ai_provider_storage_contract() {
    let Ok(url) = std::env::var("RAINDROP_TEST_POSTGRES_URL") else {
        eprintln!(
            "postgres AI provider storage contract skipped: test database URL is not configured"
        );
        return;
    };

    ai_provider_storage_contract(SecretString::from(url)).await;
}

#[tokio::test]
async fn mysql_ai_provider_storage_contract() {
    let Ok(url) = std::env::var("RAINDROP_TEST_MYSQL_URL") else {
        eprintln!(
            "mysql AI provider storage contract skipped: test database URL is not configured"
        );
        return;
    };

    ai_provider_storage_contract(SecretString::from(url)).await;
}

async fn ai_provider_storage_contract(database_url: SecretString) {
    let database = connect_for_contract(database_url).await;
    rollback(&database)
        .await
        .unwrap_or_else(|error| panic!("provider contract database should reset: {error:?}"));
    migrate(&database)
        .await
        .unwrap_or_else(|error| panic!("provider migrations should apply: {error:?}"));
    migrate(&database)
        .await
        .unwrap_or_else(|error| panic!("provider migrations should be idempotent: {error:?}"));

    assert_schema(&database).await;
    insert_user(&database, USER_A_ID, "provider-owner").await;

    provider_model(AI_PROVIDER_INSTANCE_ID, None, "Instance provider")
        .insert(&database)
        .await
        .expect("instance provider should insert");
    provider_model(AI_PROVIDER_USER_ID, Some(USER_A_ID), "User provider")
        .insert(&database)
        .await
        .expect("user provider should insert");

    let stored = ai_provider::Entity::find_by_id(AI_PROVIDER_USER_ID)
        .one(&database)
        .await
        .expect("provider should query")
        .expect("provider should exist");
    assert_eq!(stored.owner_user_id.as_deref(), Some(USER_A_ID));
    assert_eq!(stored.display_name, "User provider");
    assert_eq!(stored.kind, "OPENAI_RESPONSES");
    assert_eq!(stored.endpoint, "https://api.openai.com/");
    assert_eq!(stored.model, "gpt-test-model");
    assert_eq!(stored.encrypted_secret, ENCRYPTED_SECRET);
    assert_ne!(stored.encrypted_secret, "plaintext-provider-credential");
    assert!(stored.supports_usage);
    assert!(stored.supports_idempotency);
    assert!(!stored.supports_streaming);
    assert_eq!(stored.max_concurrency, 2);
    assert_eq!(stored.requests_per_minute, Some(60));
    assert_eq!(stored.max_input_tokens_per_request, 128_000);
    assert_eq!(stored.max_output_tokens_per_request, 16_384);
    assert_eq!(stored.input_cost_micros_per_million_tokens, Some(2_500));
    assert_eq!(stored.output_cost_micros_per_million_tokens, Some(10_000));
    assert_eq!(stored.max_cost_micros_per_request, Some(250_000));
    assert!(stored.is_enabled);
    assert_eq!(stored.revision, 0);
    assert_eq!(stored.created_at, STORED_AT);
    assert_eq!(stored.updated_at, STORED_AT);

    assert!(
        provider_model(
            "00000000-0000-4000-8000-000000000903",
            Some("00000000-0000-4000-8000-000000000999"),
            "Missing owner",
        )
        .insert(&database)
        .await
        .is_err()
    );
    assert!(
        provider_model(AI_PROVIDER_INSTANCE_ID, None, "Duplicate ID")
            .insert(&database)
            .await
            .is_err()
    );

    user::Entity::delete_by_id(USER_A_ID)
        .exec(&database)
        .await
        .expect("owner should delete");
    assert_eq!(
        ai_provider::Entity::find_by_id(AI_PROVIDER_USER_ID)
            .count(&database)
            .await
            .expect("user providers should count"),
        0
    );
    assert_eq!(
        ai_provider::Entity::find_by_id(AI_PROVIDER_INSTANCE_ID)
            .count(&database)
            .await
            .expect("instance providers should count"),
        1
    );

    rollback(&database)
        .await
        .expect("provider migrations should roll back");
    migrate(&database)
        .await
        .expect("provider migrations should recreate after rollback");
    assert_schema(&database).await;
}

async fn assert_schema(database: &DatabaseConnection) {
    let manager = SchemaManager::new(database);
    assert!(
        manager
            .has_table("ai_providers")
            .await
            .expect("table check")
    );
    for column in [
        "id",
        "owner_user_id",
        "display_name",
        "kind",
        "endpoint",
        "model",
        "encrypted_secret",
        "supports_usage",
        "supports_idempotency",
        "supports_streaming",
        "max_concurrency",
        "requests_per_minute",
        "max_input_tokens_per_request",
        "max_output_tokens_per_request",
        "input_cost_micros_per_million_tokens",
        "output_cost_micros_per_million_tokens",
        "max_cost_micros_per_request",
        "is_enabled",
        "revision",
        "created_at",
        "updated_at",
    ] {
        assert!(
            manager
                .has_column("ai_providers", column)
                .await
                .unwrap_or_else(|error| panic!("column {column} should inspect: {error:?}")),
            "column {column} should exist"
        );
    }
    assert!(
        manager
            .has_index("ai_providers", "idx_ai_providers_owner_enabled")
            .await
            .expect("owner index should inspect")
    );
    assert!(
        manager
            .has_index("ai_providers", "idx_ai_providers_kind")
            .await
            .expect("kind index should inspect")
    );
}

fn provider_model(
    id: &str,
    owner_user_id: Option<&str>,
    display_name: &str,
) -> ai_provider::ActiveModel {
    ai_provider::ActiveModel {
        id: Set(id.to_owned()),
        owner_user_id: Set(owner_user_id.map(str::to_owned)),
        display_name: Set(display_name.to_owned()),
        kind: Set("OPENAI_RESPONSES".to_owned()),
        endpoint: Set("https://api.openai.com/".to_owned()),
        model: Set("gpt-test-model".to_owned()),
        encrypted_secret: Set(ENCRYPTED_SECRET.to_owned()),
        supports_usage: Set(true),
        supports_idempotency: Set(true),
        supports_streaming: Set(false),
        max_concurrency: Set(2),
        requests_per_minute: Set(Some(60)),
        max_input_tokens_per_request: Set(128_000),
        max_output_tokens_per_request: Set(16_384),
        input_cost_micros_per_million_tokens: Set(Some(2_500)),
        output_cost_micros_per_million_tokens: Set(Some(10_000)),
        max_cost_micros_per_request: Set(Some(250_000)),
        is_enabled: Set(true),
        revision: Set(0),
        created_at: Set(STORED_AT),
        updated_at: Set(STORED_AT),
    }
}
