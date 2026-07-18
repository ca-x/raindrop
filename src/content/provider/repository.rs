use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, Condition, ConnectionTrait,
    DatabaseConnection, DbErr, EntityTrait, QueryFilter, TransactionTrait, sea_query::Expr,
};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::db::entities::{ai_provider, user};

use super::{
    CreateProvider, ProviderBinding, ProviderCapabilities, ProviderCoreError,
    ProviderCoreErrorKind, ProviderEndpoint, ProviderKind, ProviderMetadata, ProviderPolicy,
    ProviderScope, ProviderSecretError, ProviderSecretKeyring, UpdateProvider,
    model::{normalize_display_name, validate_model, validate_provider_id},
};

pub struct ProviderRepository {
    database: DatabaseConnection,
    keyring: ProviderSecretKeyring,
}

impl ProviderRepository {
    #[must_use]
    pub fn new(database: DatabaseConnection, keyring: ProviderSecretKeyring) -> Self {
        Self { database, keyring }
    }

    pub async fn create(
        &self,
        input: CreateProvider,
    ) -> Result<ProviderMetadata, ProviderCoreError> {
        input.validate()?;
        let id = Uuid::new_v4().to_string();
        let display_name = normalize_display_name(&input.display_name)?;
        let endpoint = ProviderEndpoint::new(input.kind, input.endpoint.as_deref())?;
        let encrypted_secret = self
            .keyring
            .encrypt(&id, input.kind, &input.credential)
            .map_err(secret_error)?;
        let owner_user_id = input.scope.owner_user_id().map(str::to_owned);
        let requests_per_minute = optional_u32_to_i32(input.policy.requests_per_minute)?;
        let max_input_tokens_per_request = u32_to_i32(input.policy.max_input_tokens_per_request)?;
        let max_output_tokens_per_request = u32_to_i32(input.policy.max_output_tokens_per_request)?;
        let input_cost_micros_per_million_tokens =
            optional_u64_to_i64(input.policy.input_cost_micros_per_million_tokens)?;
        let output_cost_micros_per_million_tokens =
            optional_u64_to_i64(input.policy.output_cost_micros_per_million_tokens)?;
        let max_cost_micros_per_request =
            optional_u64_to_i64(input.policy.max_cost_micros_per_request)?;
        let now = OffsetDateTime::now_utc();
        let transaction = self.database.begin().await.map_err(database_error)?;
        let result = async {
            if let Some(user_id) = owner_user_id.as_deref() {
                ensure_active_user(&transaction, user_id).await?;
            }
            let stored = ai_provider::ActiveModel {
                id: Set(id),
                owner_user_id: Set(owner_user_id),
                display_name: Set(display_name),
                kind: Set(input.kind.as_storage().to_owned()),
                endpoint: Set(endpoint.as_str().to_owned()),
                model: Set(input.model),
                encrypted_secret: Set(encrypted_secret),
                supports_usage: Set(input.capabilities.supports_usage),
                supports_idempotency: Set(input.capabilities.supports_idempotency),
                supports_streaming: Set(input.capabilities.supports_streaming),
                max_concurrency: Set(i32::from(input.policy.max_concurrency)),
                requests_per_minute: Set(requests_per_minute),
                max_input_tokens_per_request: Set(max_input_tokens_per_request),
                max_output_tokens_per_request: Set(max_output_tokens_per_request),
                input_cost_micros_per_million_tokens: Set(input_cost_micros_per_million_tokens),
                output_cost_micros_per_million_tokens: Set(output_cost_micros_per_million_tokens),
                max_cost_micros_per_request: Set(max_cost_micros_per_request),
                is_enabled: Set(input.is_enabled),
                revision: Set(0),
                created_at: Set(now),
                updated_at: Set(now),
            }
            .insert(&transaction)
            .await
            .map_err(database_error)?;
            metadata_from_model(&stored)
        }
        .await;
        finish_transaction(transaction, result).await
    }

    pub async fn get(
        &self,
        id: &str,
        scope: &ProviderScope,
    ) -> Result<ProviderMetadata, ProviderCoreError> {
        validate_provider_id(id)?;
        scope.validate()?;
        if let Some(user_id) = scope.owner_user_id() {
            ensure_active_user(&self.database, user_id).await?;
        }
        let model = scoped_query(id, scope)
            .one(&self.database)
            .await
            .map_err(database_error)?
            .ok_or_else(not_found)?;
        metadata_from_model(&model)
    }

    pub async fn list_for_user(
        &self,
        user_id: &str,
    ) -> Result<Vec<ProviderMetadata>, ProviderCoreError> {
        let scope = ProviderScope::user(user_id)?;
        let user_id = scope.owner_user_id().ok_or_else(not_found)?;
        ensure_active_user(&self.database, user_id).await?;
        let models = ai_provider::Entity::find()
            .filter(
                Condition::any()
                    .add(ai_provider::Column::OwnerUserId.is_null())
                    .add(ai_provider::Column::OwnerUserId.eq(user_id)),
            )
            .all(&self.database)
            .await
            .map_err(database_error)?;
        let mut providers = models
            .iter()
            .map(metadata_from_model)
            .collect::<Result<Vec<_>, _>>()?;
        providers.sort_by(|left, right| {
            left.display_name()
                .to_lowercase()
                .cmp(&right.display_name().to_lowercase())
                .then_with(|| left.id().cmp(right.id()))
        });
        Ok(providers)
    }

    pub async fn update(
        &self,
        id: &str,
        scope: &ProviderScope,
        patch: UpdateProvider,
    ) -> Result<ProviderMetadata, ProviderCoreError> {
        validate_provider_id(id)?;
        scope.validate()?;
        let transaction = self.database.begin().await.map_err(database_error)?;
        let result = async {
            if let Some(user_id) = scope.owner_user_id() {
                ensure_active_user(&transaction, user_id).await?;
            }
            let stored = scoped_query(id, scope)
                .one(&transaction)
                .await
                .map_err(database_error)?
                .ok_or_else(not_found)?;
            let current = metadata_from_model(&stored)?;
            if current.revision() != patch.expected_revision {
                return Err(ProviderCoreError::new(
                    ProviderCoreErrorKind::RevisionConflict,
                ));
            }
            patch.validate(current.kind())?;

            let display_name = patch
                .display_name
                .as_deref()
                .map(normalize_display_name)
                .transpose()?
                .unwrap_or_else(|| current.display_name().to_owned());
            let endpoint = patch
                .endpoint
                .as_deref()
                .map(|value| ProviderEndpoint::new(current.kind(), Some(value)))
                .transpose()?
                .unwrap_or_else(|| current.endpoint().clone());
            let model = patch
                .model
                .as_deref()
                .map(|value| {
                    validate_model(value)?;
                    Ok::<_, ProviderCoreError>(value.to_owned())
                })
                .transpose()?
                .unwrap_or_else(|| current.model().to_owned());
            let capabilities = patch.capabilities.unwrap_or_else(|| current.capabilities());
            let policy = patch.policy.unwrap_or_else(|| current.policy());
            capabilities.validate()?;
            policy.validate()?;
            let requests_per_minute = optional_u32_to_i32(policy.requests_per_minute)?;
            let max_input_tokens_per_request = u32_to_i32(policy.max_input_tokens_per_request)?;
            let max_output_tokens_per_request = u32_to_i32(policy.max_output_tokens_per_request)?;
            let input_cost_micros_per_million_tokens =
                optional_u64_to_i64(policy.input_cost_micros_per_million_tokens)?;
            let output_cost_micros_per_million_tokens =
                optional_u64_to_i64(policy.output_cost_micros_per_million_tokens)?;
            let max_cost_micros_per_request =
                optional_u64_to_i64(policy.max_cost_micros_per_request)?;
            let encrypted_secret = patch.credential.as_ref().map_or_else(
                || Ok(stored.encrypted_secret.clone()),
                |credential| {
                    self.keyring
                        .encrypt(id, current.kind(), credential)
                        .map_err(secret_error)
                },
            )?;
            let is_enabled = patch.is_enabled.unwrap_or_else(|| current.is_enabled());
            let next_revision = current
                .revision()
                .checked_add(1)
                .and_then(|value| i64::try_from(value).ok())
                .ok_or_else(corrupt_data)?;
            let now = OffsetDateTime::now_utc();

            let update = ai_provider::Entity::update_many()
                .col_expr(ai_provider::Column::DisplayName, Expr::value(display_name))
                .col_expr(
                    ai_provider::Column::Endpoint,
                    Expr::value(endpoint.as_str().to_owned()),
                )
                .col_expr(ai_provider::Column::Model, Expr::value(model))
                .col_expr(
                    ai_provider::Column::EncryptedSecret,
                    Expr::value(encrypted_secret),
                )
                .col_expr(
                    ai_provider::Column::SupportsUsage,
                    Expr::value(capabilities.supports_usage),
                )
                .col_expr(
                    ai_provider::Column::SupportsIdempotency,
                    Expr::value(capabilities.supports_idempotency),
                )
                .col_expr(
                    ai_provider::Column::SupportsStreaming,
                    Expr::value(capabilities.supports_streaming),
                )
                .col_expr(
                    ai_provider::Column::MaxConcurrency,
                    Expr::value(i32::from(policy.max_concurrency)),
                )
                .col_expr(
                    ai_provider::Column::RequestsPerMinute,
                    Expr::value(requests_per_minute),
                )
                .col_expr(
                    ai_provider::Column::MaxInputTokensPerRequest,
                    Expr::value(max_input_tokens_per_request),
                )
                .col_expr(
                    ai_provider::Column::MaxOutputTokensPerRequest,
                    Expr::value(max_output_tokens_per_request),
                )
                .col_expr(
                    ai_provider::Column::InputCostMicrosPerMillionTokens,
                    Expr::value(input_cost_micros_per_million_tokens),
                )
                .col_expr(
                    ai_provider::Column::OutputCostMicrosPerMillionTokens,
                    Expr::value(output_cost_micros_per_million_tokens),
                )
                .col_expr(
                    ai_provider::Column::MaxCostMicrosPerRequest,
                    Expr::value(max_cost_micros_per_request),
                )
                .col_expr(ai_provider::Column::IsEnabled, Expr::value(is_enabled))
                .col_expr(ai_provider::Column::Revision, Expr::value(next_revision))
                .col_expr(ai_provider::Column::UpdatedAt, Expr::value(now))
                .filter(ai_provider::Column::Id.eq(id))
                .filter(ai_provider::Column::Revision.eq(stored.revision));
            let update = match scope {
                ProviderScope::Instance => {
                    update.filter(ai_provider::Column::OwnerUserId.is_null())
                }
                ProviderScope::User(user_id) => {
                    update.filter(ai_provider::Column::OwnerUserId.eq(user_id))
                }
            };
            let result = update.exec(&transaction).await.map_err(database_error)?;
            if result.rows_affected != 1 {
                return Err(ProviderCoreError::new(
                    ProviderCoreErrorKind::RevisionConflict,
                ));
            }
            let updated = scoped_query(id, scope)
                .one(&transaction)
                .await
                .map_err(database_error)?
                .ok_or_else(corrupt_data)?;
            metadata_from_model(&updated)
        }
        .await;
        finish_transaction(transaction, result).await
    }

    pub async fn load_enabled_binding(
        &self,
        id: &str,
        user_id: &str,
    ) -> Result<ProviderBinding, ProviderCoreError> {
        validate_provider_id(id)?;
        let scope = ProviderScope::user(user_id)?;
        let user_id = scope.owner_user_id().ok_or_else(not_found)?;
        ensure_active_user(&self.database, user_id).await?;
        let stored = ai_provider::Entity::find()
            .filter(ai_provider::Column::Id.eq(id))
            .filter(
                Condition::any()
                    .add(ai_provider::Column::OwnerUserId.is_null())
                    .add(ai_provider::Column::OwnerUserId.eq(user_id)),
            )
            .one(&self.database)
            .await
            .map_err(database_error)?
            .ok_or_else(not_found)?;
        let metadata = metadata_from_model(&stored)?;
        if !metadata.is_enabled() {
            return Err(ProviderCoreError::new(
                ProviderCoreErrorKind::ProviderDisabled,
            ));
        }
        let credential = self
            .keyring
            .decrypt(id, metadata.kind(), &stored.encrypted_secret)
            .map_err(secret_error)?;
        Ok(ProviderBinding {
            metadata,
            credential,
        })
    }
}

fn scoped_query(id: &str, scope: &ProviderScope) -> sea_orm::Select<ai_provider::Entity> {
    let query = ai_provider::Entity::find().filter(ai_provider::Column::Id.eq(id));
    match scope {
        ProviderScope::Instance => query.filter(ai_provider::Column::OwnerUserId.is_null()),
        ProviderScope::User(user_id) => query.filter(ai_provider::Column::OwnerUserId.eq(user_id)),
    }
}

async fn ensure_active_user<C>(connection: &C, user_id: &str) -> Result<(), ProviderCoreError>
where
    C: ConnectionTrait,
{
    let stored = user::Entity::find_by_id(user_id)
        .one(connection)
        .await
        .map_err(database_error)?;
    if stored.is_some_and(|user| !user.is_disabled) {
        Ok(())
    } else {
        Err(not_found())
    }
}

fn metadata_from_model(model: &ai_provider::Model) -> Result<ProviderMetadata, ProviderCoreError> {
    Uuid::parse_str(&model.id).map_err(|_| corrupt_data())?;
    let scope = ProviderScope::from_owner_user_id(model.owner_user_id.clone())
        .map_err(|_| corrupt_data())?;
    let display_name = normalize_display_name(&model.display_name).map_err(|_| corrupt_data())?;
    if display_name != model.display_name {
        return Err(corrupt_data());
    }
    let kind = ProviderKind::from_storage(&model.kind)?;
    let endpoint =
        ProviderEndpoint::new(kind, Some(&model.endpoint)).map_err(|_| corrupt_data())?;
    if endpoint.as_str() != model.endpoint {
        return Err(corrupt_data());
    }
    validate_model(&model.model).map_err(|_| corrupt_data())?;
    let capabilities = ProviderCapabilities {
        supports_usage: model.supports_usage,
        supports_idempotency: model.supports_idempotency,
        supports_streaming: model.supports_streaming,
    };
    capabilities.validate().map_err(|_| corrupt_data())?;
    let policy = ProviderPolicy {
        max_concurrency: u16::try_from(model.max_concurrency).map_err(|_| corrupt_data())?,
        requests_per_minute: model
            .requests_per_minute
            .map(|value| u32::try_from(value).map_err(|_| corrupt_data()))
            .transpose()?,
        max_input_tokens_per_request: u32::try_from(model.max_input_tokens_per_request)
            .map_err(|_| corrupt_data())?,
        max_output_tokens_per_request: u32::try_from(model.max_output_tokens_per_request)
            .map_err(|_| corrupt_data())?,
        input_cost_micros_per_million_tokens: nonnegative_cost(
            model.input_cost_micros_per_million_tokens,
        )?,
        output_cost_micros_per_million_tokens: nonnegative_cost(
            model.output_cost_micros_per_million_tokens,
        )?,
        max_cost_micros_per_request: nonnegative_cost(model.max_cost_micros_per_request)?,
    };
    policy.validate().map_err(|_| corrupt_data())?;
    let revision = u64::try_from(model.revision).map_err(|_| corrupt_data())?;
    Ok(ProviderMetadata {
        id: model.id.clone(),
        scope,
        display_name,
        kind,
        endpoint,
        model: model.model.clone(),
        capabilities,
        policy,
        is_enabled: model.is_enabled,
        revision,
        created_at: model.created_at,
        updated_at: model.updated_at,
    })
}

fn nonnegative_cost(value: Option<i64>) -> Result<Option<u64>, ProviderCoreError> {
    value
        .map(|value| u64::try_from(value).map_err(|_| corrupt_data()))
        .transpose()
}

fn u32_to_i32(value: u32) -> Result<i32, ProviderCoreError> {
    i32::try_from(value).map_err(|_| ProviderCoreError::new(ProviderCoreErrorKind::InvalidPolicy))
}

fn optional_u32_to_i32(value: Option<u32>) -> Result<Option<i32>, ProviderCoreError> {
    value.map(u32_to_i32).transpose()
}

fn optional_u64_to_i64(value: Option<u64>) -> Result<Option<i64>, ProviderCoreError> {
    value
        .map(|value| {
            i64::try_from(value)
                .map_err(|_| ProviderCoreError::new(ProviderCoreErrorKind::InvalidPolicy))
        })
        .transpose()
}

async fn finish_transaction<T>(
    transaction: sea_orm::DatabaseTransaction,
    result: Result<T, ProviderCoreError>,
) -> Result<T, ProviderCoreError> {
    match result {
        Ok(value) => {
            transaction.commit().await.map_err(database_error)?;
            Ok(value)
        }
        Err(error) => {
            transaction.rollback().await.map_err(database_error)?;
            Err(error)
        }
    }
}

fn database_error(_error: DbErr) -> ProviderCoreError {
    ProviderCoreError::new(ProviderCoreErrorKind::Database)
}

fn secret_error(_error: ProviderSecretError) -> ProviderCoreError {
    ProviderCoreError::new(ProviderCoreErrorKind::SecretUnavailable)
}

const fn not_found() -> ProviderCoreError {
    ProviderCoreError::new(ProviderCoreErrorKind::NotFound)
}

const fn corrupt_data() -> ProviderCoreError {
    ProviderCoreError::new(ProviderCoreErrorKind::CorruptData)
}
