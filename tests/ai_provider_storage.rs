#[allow(dead_code)]
mod support;

use std::sync::Arc;

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use raindrop::{
    content::provider::{
        CreateProvider, ProviderCapabilities, ProviderCoreErrorKind, ProviderEndpoint,
        ProviderKind, ProviderPolicy, ProviderRepository, ProviderScope, ProviderSecretKeyring,
        UpdateProvider,
    },
    db::{
        entities::{ai_provider, user},
        migrate, rollback,
    },
};
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, DatabaseConnection, EntityTrait,
    IntoActiveModel, PaginatorTrait, QueryFilter,
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

#[test]
fn provider_model_kind_endpoint_and_policy_contract() {
    for (kind, storage, default_endpoint) in [
        (
            ProviderKind::AnthropicMessages,
            "ANTHROPIC_MESSAGES",
            "https://api.anthropic.com/",
        ),
        (
            ProviderKind::OpenAiResponses,
            "OPENAI_RESPONSES",
            "https://api.openai.com/",
        ),
        (
            ProviderKind::OpenAiChatCompletions,
            "OPENAI_CHAT_COMPLETIONS",
            "https://api.openai.com/",
        ),
        (
            ProviderKind::GoogleGemini,
            "GOOGLE_GEMINI",
            "https://generativelanguage.googleapis.com/",
        ),
    ] {
        assert_eq!(kind.as_storage(), storage);
        assert_eq!(ProviderKind::from_storage(storage).unwrap(), kind);
        assert_eq!(kind.default_endpoint(), default_endpoint);
        assert_eq!(
            ProviderEndpoint::new(kind, None).unwrap().as_str(),
            default_endpoint
        );
    }
    assert_eq!(
        ProviderKind::from_storage("UNKNOWN")
            .expect_err("unknown provider storage kind should fail")
            .kind(),
        ProviderCoreErrorKind::CorruptData
    );

    let endpoint = ProviderEndpoint::new(
        ProviderKind::OpenAiResponses,
        Some("https://gateway.example/api/tenant-sentinel"),
    )
    .expect("path-prefixed HTTPS endpoint should normalize");
    assert_eq!(
        endpoint.as_str(),
        "https://gateway.example/api/tenant-sentinel/"
    );
    assert_eq!(
        endpoint
            .join_adapter_path("/v1/responses")
            .expect("adapter path should join")
            .as_str(),
        "https://gateway.example/api/tenant-sentinel/v1/responses"
    );
    assert_eq!(
        endpoint
            .join_adapter_path("/v1beta/models/model%2Fvariant:generateContent")
            .expect("encoded model separator should remain data")
            .as_str(),
        "https://gateway.example/api/tenant-sentinel/v1beta/models/model%2Fvariant:generateContent"
    );
    let formatted = format!("{endpoint:?}");
    assert!(formatted.contains("gateway.example"));
    assert!(!formatted.contains("tenant-sentinel"));
}

#[test]
fn provider_endpoint_rejects_ambiguous_or_private_routes() {
    for invalid in [
        "http://api.example.com/",
        "https://user:password@api.example.com/",
        "https://api.example.com/?tenant=secret",
        "https://api.example.com/#fragment",
        "https://api.example.com/a/../b",
        "https://api.example.com/a/%2e%2e/b",
        "https://api.example.com/a/%2f/b",
        "https://api.example.com/a/%5c/b",
        "https://api.example.com/a\\b",
        "https://127.0.0.1/",
        "https://10.0.0.1/",
        "https://[::1]/",
    ] {
        let error = ProviderEndpoint::new(ProviderKind::OpenAiResponses, Some(invalid))
            .expect_err("unsafe endpoint should fail");
        assert_eq!(error.kind(), ProviderCoreErrorKind::InvalidEndpoint);
        let formatted = format!("{error:?} {error}");
        assert!(!formatted.contains(invalid));
    }
    ProviderEndpoint::new(
        ProviderKind::OpenAiResponses,
        Some("https://93.184.216.34/"),
    )
    .expect("public literal endpoint should be admitted");

    let endpoint = ProviderEndpoint::new(ProviderKind::OpenAiResponses, None).unwrap();
    for invalid in [
        "",
        "v1/responses",
        "//other.example/path",
        "/../v1/responses",
        "/v1/./responses",
        "/v1/responses?debug=true",
        "/v1/responses#fragment",
        "/v1\\responses",
        "/v1/\nresponses",
    ] {
        assert_eq!(
            endpoint
                .join_adapter_path(invalid)
                .expect_err("unsafe adapter path should fail")
                .kind(),
            ProviderCoreErrorKind::InvalidEndpoint
        );
    }
}

#[test]
fn provider_scope_capability_policy_and_patch_bounds_are_exact() {
    assert!(ProviderScope::user(USER_A_ID).is_ok());
    assert_eq!(
        ProviderScope::user("invalid-user")
            .expect_err("invalid user scope should fail")
            .kind(),
        ProviderCoreErrorKind::InvalidUserId
    );

    let capabilities = ProviderCapabilities {
        supports_usage: true,
        supports_idempotency: true,
        supports_streaming: false,
    };
    capabilities.validate().unwrap();
    assert_eq!(
        ProviderCapabilities {
            supports_streaming: true,
            ..capabilities
        }
        .validate()
        .expect_err("streaming should remain unavailable")
        .kind(),
        ProviderCoreErrorKind::UnsupportedCapability
    );

    for policy in [
        ProviderPolicy {
            max_concurrency: 1,
            requests_per_minute: Some(1),
            max_input_tokens_per_request: 1,
            max_output_tokens_per_request: 1,
            input_cost_micros_per_million_tokens: Some(0),
            output_cost_micros_per_million_tokens: Some(0),
            max_cost_micros_per_request: Some(0),
        },
        ProviderPolicy {
            max_concurrency: 64,
            requests_per_minute: Some(1_000_000),
            max_input_tokens_per_request: 1_048_576,
            max_output_tokens_per_request: 16_384,
            input_cost_micros_per_million_tokens: Some(1_000_000_000_000),
            output_cost_micros_per_million_tokens: Some(1_000_000_000_000),
            max_cost_micros_per_request: Some(1_000_000_000_000),
        },
    ] {
        policy.validate().unwrap();
    }
    for policy in [
        ProviderPolicy {
            max_concurrency: 0,
            ..valid_policy()
        },
        ProviderPolicy {
            max_concurrency: 65,
            ..valid_policy()
        },
        ProviderPolicy {
            requests_per_minute: Some(0),
            ..valid_policy()
        },
        ProviderPolicy {
            requests_per_minute: Some(1_000_001),
            ..valid_policy()
        },
        ProviderPolicy {
            max_input_tokens_per_request: 0,
            ..valid_policy()
        },
        ProviderPolicy {
            max_output_tokens_per_request: 16_385,
            ..valid_policy()
        },
        ProviderPolicy {
            max_cost_micros_per_request: Some(1_000_000_000_001),
            ..valid_policy()
        },
    ] {
        assert_eq!(
            policy
                .validate()
                .expect_err("invalid policy should fail")
                .kind(),
            ProviderCoreErrorKind::InvalidPolicy
        );
    }

    assert_eq!(
        UpdateProvider::default()
            .validate(ProviderKind::OpenAiResponses)
            .expect_err("empty patch should fail")
            .kind(),
        ProviderCoreErrorKind::InvalidPatch
    );
}

#[test]
fn provider_create_validation_enforces_name_model_and_nested_contracts() {
    let valid = valid_create();
    valid.validate().unwrap();

    for display_name in [
        String::new(),
        " ".to_owned(),
        "x".repeat(81),
        "bad\nname".to_owned(),
    ] {
        assert_eq!(
            CreateProvider {
                display_name,
                ..valid_create()
            }
            .validate()
            .expect_err("invalid display name should fail")
            .kind(),
            ProviderCoreErrorKind::InvalidDisplayName
        );
    }
    CreateProvider {
        display_name: "x".repeat(80),
        ..valid_create()
    }
    .validate()
    .unwrap();

    for model in [String::new(), "x".repeat(201), "bad\u{0}model".to_owned()] {
        assert_eq!(
            CreateProvider {
                model,
                ..valid_create()
            }
            .validate()
            .expect_err("invalid model should fail")
            .kind(),
            ProviderCoreErrorKind::InvalidModel
        );
    }
    CreateProvider {
        model: "x".repeat(200),
        ..valid_create()
    }
    .validate()
    .unwrap();

    let redacted = CreateProvider {
        endpoint: Some("https://gateway.example/endpoint-sentinel/".to_owned()),
        model: "model-sentinel".to_owned(),
        credential: SecretString::from("credential-sentinel"),
        ..valid_create()
    };
    let formatted = format!("{redacted:?}");
    for sentinel in ["endpoint-sentinel", "model-sentinel", "credential-sentinel"] {
        assert!(!formatted.contains(sentinel));
    }
}

fn valid_policy() -> ProviderPolicy {
    ProviderPolicy {
        max_concurrency: 2,
        requests_per_minute: Some(60),
        max_input_tokens_per_request: 128_000,
        max_output_tokens_per_request: 16_384,
        input_cost_micros_per_million_tokens: Some(2_500),
        output_cost_micros_per_million_tokens: Some(10_000),
        max_cost_micros_per_request: Some(250_000),
    }
}

fn valid_create() -> CreateProvider {
    CreateProvider {
        scope: ProviderScope::Instance,
        display_name: "Provider".to_owned(),
        kind: ProviderKind::OpenAiResponses,
        endpoint: None,
        model: "gpt-test-model".to_owned(),
        credential: SecretString::from("test-provider-credential"),
        capabilities: ProviderCapabilities {
            supports_usage: true,
            supports_idempotency: true,
            supports_streaming: false,
        },
        policy: valid_policy(),
        is_enabled: true,
    }
}

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
    assert_repository_contract(&database).await;
}

async fn assert_repository_contract(database: &DatabaseConnection) {
    insert_user(database, USER_A_ID, "provider-user-a").await;
    insert_user(
        database,
        "00000000-0000-4000-8000-000000000002",
        "provider-user-b",
    )
    .await;
    insert_user(
        database,
        "00000000-0000-4000-8000-000000000003",
        "provider-user-disabled",
    )
    .await;
    let disabled = user::Entity::find_by_id("00000000-0000-4000-8000-000000000003")
        .one(database)
        .await
        .unwrap()
        .unwrap();
    let mut disabled = disabled.into_active_model();
    disabled.is_disabled = Set(true);
    disabled.update(database).await.unwrap();

    let repository = ProviderRepository::new(
        database.clone(),
        Some(Arc::new(provider_keyring("primary", 31))),
    );
    let instance = repository
        .create(CreateProvider {
            display_name: "Zulu instance".to_owned(),
            ..valid_create()
        })
        .await
        .expect("instance provider should create");
    let user_provider = repository
        .create(CreateProvider {
            scope: ProviderScope::user(USER_A_ID).unwrap(),
            display_name: "alpha user".to_owned(),
            endpoint: Some("https://gateway.example/api/tenant-sentinel/".to_owned()),
            credential: SecretString::from("repository-credential-sentinel"),
            ..valid_create()
        })
        .await
        .expect("user provider should create");
    let other_user_provider = repository
        .create(CreateProvider {
            scope: ProviderScope::user("00000000-0000-4000-8000-000000000002").unwrap(),
            display_name: "Other user".to_owned(),
            ..valid_create()
        })
        .await
        .expect("other user provider should create");
    let disabled_provider = repository
        .create(CreateProvider {
            display_name: "Disabled".to_owned(),
            is_enabled: false,
            ..valid_create()
        })
        .await
        .expect("disabled provider should create");

    assert_eq!(
        repository
            .create(CreateProvider {
                scope: ProviderScope::user("00000000-0000-4000-8000-000000000003").unwrap(),
                ..valid_create()
            })
            .await
            .expect_err("disabled owner should not receive a provider")
            .kind(),
        ProviderCoreErrorKind::NotFound
    );

    let stored = ai_provider::Entity::find_by_id(user_provider.id())
        .one(database)
        .await
        .expect("ciphertext row should query")
        .expect("ciphertext row should exist");
    assert!(stored.encrypted_secret.starts_with("rdsec1.primary."));
    assert!(
        !stored
            .encrypted_secret
            .contains("repository-credential-sentinel")
    );
    let original_ciphertext = stored.encrypted_secret.clone();

    assert_eq!(
        repository
            .get(user_provider.id(), &ProviderScope::Instance)
            .await
            .expect_err("wrong scope should be invisible")
            .kind(),
        ProviderCoreErrorKind::NotFound
    );
    assert_eq!(
        repository
            .get(
                instance.id(),
                &ProviderScope::user(USER_A_ID).expect("user scope should construct"),
            )
            .await
            .expect_err("instance provider should require instance scope for mutation")
            .kind(),
        ProviderCoreErrorKind::NotFound
    );
    let listed = repository
        .list_for_user(USER_A_ID)
        .await
        .expect("user-visible providers should list");
    assert_eq!(
        listed
            .iter()
            .map(|provider| provider.id())
            .collect::<Vec<_>>(),
        vec![user_provider.id(), disabled_provider.id(), instance.id(),]
    );
    assert!(
        !listed
            .iter()
            .any(|provider| provider.id() == other_user_provider.id())
    );

    let binding = repository
        .load_enabled_binding(user_provider.id(), USER_A_ID)
        .await
        .expect("owned provider binding should load and decrypt");
    let formatted = format!("{user_provider:?} {binding:?}");
    for sentinel in [
        "tenant-sentinel",
        "gpt-test-model",
        "repository-credential-sentinel",
        original_ciphertext.as_str(),
    ] {
        assert!(!formatted.contains(sentinel));
    }
    repository
        .load_enabled_binding(instance.id(), USER_A_ID)
        .await
        .expect("instance provider binding should be visible");
    assert_eq!(
        repository
            .load_enabled_binding(user_provider.id(), "00000000-0000-4000-8000-000000000002")
            .await
            .expect_err("another user should not load the binding")
            .kind(),
        ProviderCoreErrorKind::NotFound
    );
    assert_eq!(
        repository
            .load_enabled_binding(disabled_provider.id(), USER_A_ID)
            .await
            .expect_err("disabled provider should not bind")
            .kind(),
        ProviderCoreErrorKind::ProviderDisabled
    );
    assert_eq!(
        repository
            .load_enabled_binding(instance.id(), "00000000-0000-4000-8000-000000000003",)
            .await
            .expect_err("disabled user should not load an instance binding")
            .kind(),
        ProviderCoreErrorKind::NotFound
    );

    let renamed = repository
        .update(
            user_provider.id(),
            user_provider.scope(),
            UpdateProvider {
                expected_revision: user_provider.revision(),
                display_name: Some("  Renamed user  ".to_owned()),
                ..UpdateProvider::default()
            },
        )
        .await
        .expect("metadata update should succeed");
    assert_eq!(renamed.display_name(), "Renamed user");
    assert_eq!(renamed.kind(), ProviderKind::OpenAiResponses);
    assert_eq!(renamed.revision(), 1);
    let unchanged_ciphertext = ai_provider::Entity::find_by_id(user_provider.id())
        .one(database)
        .await
        .unwrap()
        .unwrap()
        .encrypted_secret;
    assert_eq!(unchanged_ciphertext, original_ciphertext);

    let rotated = repository
        .update(
            user_provider.id(),
            user_provider.scope(),
            UpdateProvider {
                expected_revision: renamed.revision(),
                credential: Some(SecretString::from("replacement-credential-sentinel")),
                ..UpdateProvider::default()
            },
        )
        .await
        .expect("credential replacement should succeed");
    assert_eq!(rotated.revision(), 2);
    let replacement_ciphertext = ai_provider::Entity::find_by_id(user_provider.id())
        .one(database)
        .await
        .unwrap()
        .unwrap()
        .encrypted_secret;
    assert_ne!(replacement_ciphertext, original_ciphertext);
    assert!(!replacement_ciphertext.contains("replacement-credential-sentinel"));
    repository
        .load_enabled_binding(user_provider.id(), USER_A_ID)
        .await
        .expect("replacement credential should decrypt");

    let metadata_only = ProviderRepository::new(database.clone(), None);
    assert_eq!(
        metadata_only
            .get(user_provider.id(), user_provider.scope())
            .await
            .expect("metadata-only repository should read providers")
            .id(),
        user_provider.id()
    );
    assert!(
        metadata_only
            .list_for_user(USER_A_ID)
            .await
            .expect("metadata-only repository should list providers")
            .iter()
            .any(|provider| provider.id() == user_provider.id())
    );
    let metadata_updated = metadata_only
        .update(
            user_provider.id(),
            user_provider.scope(),
            UpdateProvider {
                expected_revision: rotated.revision(),
                display_name: Some("Metadata only".to_owned()),
                ..UpdateProvider::default()
            },
        )
        .await
        .expect("metadata-only update should preserve encrypted credential");
    assert_eq!(metadata_updated.revision(), 3);
    assert_eq!(
        metadata_only
            .update(
                user_provider.id(),
                user_provider.scope(),
                UpdateProvider {
                    expected_revision: metadata_updated.revision(),
                    credential: Some(SecretString::from("unavailable-rotation")),
                    ..UpdateProvider::default()
                },
            )
            .await
            .expect_err("credential rotation should require a keyring")
            .kind(),
        ProviderCoreErrorKind::SecretUnavailable
    );
    assert_eq!(
        metadata_only
            .create(CreateProvider {
                scope: ProviderScope::user(USER_A_ID).unwrap(),
                ..valid_create()
            })
            .await
            .expect_err("provider creation should require a keyring")
            .kind(),
        ProviderCoreErrorKind::SecretUnavailable
    );
    assert_eq!(
        metadata_only
            .load_enabled_binding(user_provider.id(), USER_A_ID)
            .await
            .expect_err("binding load should require a keyring")
            .kind(),
        ProviderCoreErrorKind::SecretUnavailable
    );

    assert_eq!(
        repository
            .update(
                user_provider.id(),
                user_provider.scope(),
                UpdateProvider {
                    expected_revision: renamed.revision(),
                    is_enabled: Some(false),
                    ..UpdateProvider::default()
                },
            )
            .await
            .expect_err("stale revision should conflict")
            .kind(),
        ProviderCoreErrorKind::RevisionConflict
    );

    let unknown_key_repository = ProviderRepository::new(
        database.clone(),
        Some(Arc::new(provider_keyring("replacement", 32))),
    );
    assert_eq!(
        unknown_key_repository
            .load_enabled_binding(user_provider.id(), USER_A_ID)
            .await
            .expect_err("missing decryption key should fail closed")
            .kind(),
        ProviderCoreErrorKind::SecretUnavailable
    );

    corrupt_column(
        database,
        instance.id(),
        ai_provider::Column::Kind,
        "UNKNOWN",
    )
    .await;
    assert_eq!(
        repository
            .get(instance.id(), &ProviderScope::Instance)
            .await
            .expect_err("unknown kind should be corrupt")
            .kind(),
        ProviderCoreErrorKind::CorruptData
    );
    corrupt_integer_column(
        database,
        disabled_provider.id(),
        ai_provider::Column::MaxConcurrency,
        0,
    )
    .await;
    assert_eq!(
        repository
            .get(disabled_provider.id(), &ProviderScope::Instance)
            .await
            .expect_err("invalid policy should be corrupt")
            .kind(),
        ProviderCoreErrorKind::CorruptData
    );
    corrupt_column(
        database,
        user_provider.id(),
        ai_provider::Column::EncryptedSecret,
        "rdsec1.primary.invalid.invalid",
    )
    .await;
    assert_eq!(
        repository
            .load_enabled_binding(user_provider.id(), USER_A_ID)
            .await
            .expect_err("tampered ciphertext should fail closed")
            .kind(),
        ProviderCoreErrorKind::SecretUnavailable
    );
}

async fn corrupt_column(
    database: &DatabaseConnection,
    provider_id: &str,
    column: ai_provider::Column,
    value: &str,
) {
    ai_provider::Entity::update_many()
        .col_expr(column, sea_orm::sea_query::Expr::value(value.to_owned()))
        .filter(ai_provider::Column::Id.eq(provider_id))
        .exec(database)
        .await
        .expect("test corruption should update one row");
}

async fn corrupt_integer_column(
    database: &DatabaseConnection,
    provider_id: &str,
    column: ai_provider::Column,
    value: i32,
) {
    ai_provider::Entity::update_many()
        .col_expr(column, sea_orm::sea_query::Expr::value(value))
        .filter(ai_provider::Column::Id.eq(provider_id))
        .exec(database)
        .await
        .expect("test corruption should update one row");
}

fn provider_keyring(id: &str, byte: u8) -> ProviderSecretKeyring {
    let entry = SecretString::from(format!("{id}:{}", URL_SAFE_NO_PAD.encode([byte; 32])));
    ProviderSecretKeyring::from_entries(&[entry]).expect("test keyring should construct")
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
