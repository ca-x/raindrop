use std::collections::HashSet;

use blake3::Hasher;
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, ConnectionTrait, DatabaseBackend,
    DatabaseConnection, DbBackend, DbErr, EntityTrait, QueryFilter, QueryOrder, QuerySelect,
    RuntimeErr, SqlxError, SqlxMySqlError, SqlxPostgresError, SqlxSqliteError, Statement,
    TransactionTrait, TryGetable, sea_query::Expr,
};
use serde_json::Value;
use time::OffsetDateTime;
use uuid::Uuid;

use crate::db::entities::{
    plugin_capability_grant, plugin_config, plugin_installation, plugin_kv, user,
};

use super::{
    AiContentConfig, BundledOfficialPlugin, CapabilityGrantInput, PluginCapabilityGrant,
    PluginConfig, PluginInstallation, PluginKvValue, PluginRegistryError, PluginRegistryErrorKind,
    PluginSystemState,
    json::{
        canonical_json, parse_unique_json, validate_lower_hex_hash, validate_uuid,
        validate_visible_ascii,
    },
    manifest::validate_persisted_manifest,
};

const OFFICIAL_DISTRIBUTION: &str = "BUNDLED_OFFICIAL";
const MAX_CONSTRAINTS_BYTES: usize = 64 * 1024;
const MAX_KV_VALUE_BYTES: usize = 64 * 1024;
const MAX_KV_KEYS: usize = 128;
const MAX_KV_TOTAL_BYTES: usize = 1024 * 1024;
const GRANT_HASH_CONTEXT: &str = "raindrop.plugin-grant-key.v1";

#[derive(Clone)]
pub struct PluginRegistryRepository {
    database: DatabaseConnection,
}

impl PluginRegistryRepository {
    #[must_use]
    pub fn new(database: DatabaseConnection) -> Self {
        Self { database }
    }

    pub async fn sync_bundled(
        &self,
        bundle: &BundledOfficialPlugin,
    ) -> Result<PluginInstallation, PluginRegistryError> {
        for attempt in 0..3 {
            match self.sync_bundled_once(bundle).await {
                Err(error)
                    if error.kind() == PluginRegistryErrorKind::RevisionConflict && attempt < 2 => {
                }
                result => return result,
            }
        }
        Err(PluginRegistryError::new(
            PluginRegistryErrorKind::RevisionConflict,
        ))
    }

    pub async fn get_installation(
        &self,
        plugin_key: &str,
    ) -> Result<PluginInstallation, PluginRegistryError> {
        validate_plugin_key(plugin_key)?;
        let model = plugin_installation::Entity::find()
            .filter(plugin_installation::Column::PluginKey.eq(plugin_key))
            .one(&self.database)
            .await
            .map_err(database_error)?
            .ok_or_else(not_found)?;
        installation_from_model(model)
    }

    pub async fn replace_ai_config(
        &self,
        plugin_key: &str,
        user_id: &str,
        expected_revision: Option<u64>,
        is_enabled: bool,
        config_json: &[u8],
    ) -> Result<PluginConfig, PluginRegistryError> {
        validate_plugin_key(plugin_key)?;
        validate_uuid(user_id, PluginRegistryErrorKind::InvalidInput)?;
        let config = AiContentConfig::parse(config_json)?;
        let transaction = self.database.begin().await.map_err(database_error)?;
        let backend = self.database.get_database_backend();
        let result = async {
            lock_active_user(&transaction, backend, user_id).await?;
            let installation = find_installation(&transaction, plugin_key).await?;
            if is_enabled && installation.system_state() != PluginSystemState::Enabled {
                return Err(PluginRegistryError::new(
                    PluginRegistryErrorKind::InvalidInput,
                ));
            }
            let existing = plugin_config::Entity::find()
                .filter(plugin_config::Column::PluginId.eq(installation.id()))
                .filter(plugin_config::Column::OwnerUserId.eq(user_id))
                .one(&transaction)
                .await
                .map_err(database_error)?;
            let now = OffsetDateTime::now_utc();
            let model = match existing {
                None => {
                    if expected_revision.is_some() {
                        return Err(not_found());
                    }
                    plugin_config::ActiveModel {
                        id: Set(Uuid::new_v4().to_string()),
                        plugin_id: Set(installation.id().to_owned()),
                        owner_user_id: Set(user_id.to_owned()),
                        schema_version: Set(1),
                        config_json: Set(config.canonical_json().to_owned()),
                        config_hash: Set(config.config_hash().to_owned()),
                        is_enabled: Set(is_enabled),
                        revision: Set(0),
                        created_at: Set(now),
                        updated_at: Set(now),
                    }
                    .insert(&transaction)
                    .await
                    .map_err(database_error)?
                }
                Some(existing) => {
                    let current = config_from_model(existing.clone())?;
                    if expected_revision != Some(current.revision()) {
                        return Err(revision_conflict());
                    }
                    let next_revision = next_revision(current.revision())?;
                    let update = plugin_config::Entity::update_many()
                        .col_expr(
                            plugin_config::Column::ConfigJson,
                            Expr::value(config.canonical_json().to_owned()),
                        )
                        .col_expr(
                            plugin_config::Column::ConfigHash,
                            Expr::value(config.config_hash().to_owned()),
                        )
                        .col_expr(plugin_config::Column::SchemaVersion, Expr::value(1))
                        .col_expr(plugin_config::Column::IsEnabled, Expr::value(is_enabled))
                        .col_expr(
                            plugin_config::Column::Revision,
                            Expr::value(u64_to_i64(next_revision)?),
                        )
                        .col_expr(plugin_config::Column::UpdatedAt, Expr::value(now))
                        .filter(plugin_config::Column::Id.eq(&existing.id))
                        .filter(plugin_config::Column::Revision.eq(u64_to_i64(current.revision())?))
                        .exec(&transaction)
                        .await
                        .map_err(database_error)?;
                    if update.rows_affected != 1 {
                        return Err(revision_conflict());
                    }
                    plugin_config::Entity::find_by_id(existing.id)
                        .one(&transaction)
                        .await
                        .map_err(database_error)?
                        .ok_or_else(corrupt_data)?
                }
            };
            config_from_model(model)
        }
        .await;
        finish_transaction(transaction, result).await
    }

    pub async fn get_ai_config(
        &self,
        plugin_key: &str,
        user_id: &str,
    ) -> Result<Option<PluginConfig>, PluginRegistryError> {
        validate_plugin_key(plugin_key)?;
        validate_uuid(user_id, PluginRegistryErrorKind::InvalidInput)?;
        ensure_active_user(&self.database, user_id).await?;
        let installation = find_installation(&self.database, plugin_key).await?;
        plugin_config::Entity::find()
            .filter(plugin_config::Column::PluginId.eq(installation.id()))
            .filter(plugin_config::Column::OwnerUserId.eq(user_id))
            .one(&self.database)
            .await
            .map_err(database_error)?
            .map(config_from_model)
            .transpose()
    }

    pub async fn grant_capability(
        &self,
        input: CapabilityGrantInput,
    ) -> Result<PluginCapabilityGrant, PluginRegistryError> {
        let validated = ValidatedGrantInput::new(input)?;
        let transaction = self.database.begin().await.map_err(database_error)?;
        let backend = self.database.get_database_backend();
        let result = async {
            lock_active_user(&transaction, backend, &validated.owner_user_id).await?;
            let installation = find_installation(&transaction, &validated.plugin_key).await?;
            let existing = plugin_capability_grant::Entity::find()
                .filter(plugin_capability_grant::Column::PluginId.eq(installation.id()))
                .filter(plugin_capability_grant::Column::OwnerUserId.eq(&validated.owner_user_id))
                .filter(plugin_capability_grant::Column::GrantKeyHash.eq(&validated.grant_key_hash))
                .one(&transaction)
                .await
                .map_err(database_error)?;
            let now = OffsetDateTime::now_utc();
            let model = match existing {
                None => {
                    if validated.expected_revision.is_some() {
                        return Err(not_found());
                    }
                    plugin_capability_grant::ActiveModel {
                        id: Set(Uuid::new_v4().to_string()),
                        plugin_id: Set(installation.id().to_owned()),
                        owner_user_id: Set(validated.owner_user_id.clone()),
                        capability: Set(validated.capability.clone()),
                        operation: Set(validated.operation.clone()),
                        resource_type: Set(validated.resource_type.clone()),
                        resource_id: Set(validated.resource_id.clone()),
                        grant_key_hash: Set(validated.grant_key_hash.clone()),
                        constraints_json: Set(validated.constraints_json.clone()),
                        revision: Set(0),
                        created_at: Set(now),
                        updated_at: Set(now),
                        revoked_at: Set(None),
                    }
                    .insert(&transaction)
                    .await
                    .map_err(database_error)?
                }
                Some(existing) => {
                    let current = grant_from_model(existing.clone())?;
                    validated.matches(&current)?;
                    if validated.expected_revision != Some(current.revision()) {
                        return Err(revision_conflict());
                    }
                    let next_revision = next_revision(current.revision())?;
                    let update = plugin_capability_grant::Entity::update_many()
                        .col_expr(
                            plugin_capability_grant::Column::ConstraintsJson,
                            Expr::value(validated.constraints_json.clone()),
                        )
                        .col_expr(
                            plugin_capability_grant::Column::Revision,
                            Expr::value(u64_to_i64(next_revision)?),
                        )
                        .col_expr(plugin_capability_grant::Column::UpdatedAt, Expr::value(now))
                        .col_expr(
                            plugin_capability_grant::Column::RevokedAt,
                            Expr::value(Option::<OffsetDateTime>::None),
                        )
                        .filter(plugin_capability_grant::Column::Id.eq(&existing.id))
                        .filter(
                            plugin_capability_grant::Column::Revision
                                .eq(u64_to_i64(current.revision())?),
                        )
                        .exec(&transaction)
                        .await
                        .map_err(database_error)?;
                    if update.rows_affected != 1 {
                        return Err(revision_conflict());
                    }
                    plugin_capability_grant::Entity::find_by_id(existing.id)
                        .one(&transaction)
                        .await
                        .map_err(database_error)?
                        .ok_or_else(corrupt_data)?
                }
            };
            grant_from_model(model)
        }
        .await;
        finish_transaction(transaction, result).await
    }

    pub async fn revoke_capability(
        &self,
        plugin_key: &str,
        user_id: &str,
        grant_id: &str,
        expected_revision: u64,
    ) -> Result<PluginCapabilityGrant, PluginRegistryError> {
        validate_plugin_key(plugin_key)?;
        validate_uuid(user_id, PluginRegistryErrorKind::InvalidInput)?;
        validate_uuid(grant_id, PluginRegistryErrorKind::InvalidInput)?;
        let transaction = self.database.begin().await.map_err(database_error)?;
        let backend = self.database.get_database_backend();
        let result = async {
            lock_active_user(&transaction, backend, user_id).await?;
            let installation = find_installation(&transaction, plugin_key).await?;
            let stored = plugin_capability_grant::Entity::find_by_id(grant_id)
                .filter(plugin_capability_grant::Column::PluginId.eq(installation.id()))
                .filter(plugin_capability_grant::Column::OwnerUserId.eq(user_id))
                .one(&transaction)
                .await
                .map_err(database_error)?
                .ok_or_else(not_found)?;
            let current = grant_from_model(stored.clone())?;
            if current.revision() != expected_revision {
                return Err(revision_conflict());
            }
            if current.is_revoked() {
                return Ok(current);
            }
            let next_revision = next_revision(current.revision())?;
            let now = OffsetDateTime::now_utc();
            let update = plugin_capability_grant::Entity::update_many()
                .col_expr(
                    plugin_capability_grant::Column::Revision,
                    Expr::value(u64_to_i64(next_revision)?),
                )
                .col_expr(plugin_capability_grant::Column::UpdatedAt, Expr::value(now))
                .col_expr(
                    plugin_capability_grant::Column::RevokedAt,
                    Expr::value(Some(now)),
                )
                .filter(plugin_capability_grant::Column::Id.eq(grant_id))
                .filter(
                    plugin_capability_grant::Column::Revision.eq(u64_to_i64(current.revision())?),
                )
                .exec(&transaction)
                .await
                .map_err(database_error)?;
            if update.rows_affected != 1 {
                return Err(revision_conflict());
            }
            let model = plugin_capability_grant::Entity::find_by_id(grant_id)
                .one(&transaction)
                .await
                .map_err(database_error)?
                .ok_or_else(corrupt_data)?;
            grant_from_model(model)
        }
        .await;
        finish_transaction(transaction, result).await
    }

    pub async fn list_active_grants(
        &self,
        plugin_key: &str,
        user_id: &str,
    ) -> Result<Vec<PluginCapabilityGrant>, PluginRegistryError> {
        validate_plugin_key(plugin_key)?;
        validate_uuid(user_id, PluginRegistryErrorKind::InvalidInput)?;
        ensure_active_user(&self.database, user_id).await?;
        let installation = find_installation(&self.database, plugin_key).await?;
        plugin_capability_grant::Entity::find()
            .filter(plugin_capability_grant::Column::PluginId.eq(installation.id()))
            .filter(plugin_capability_grant::Column::OwnerUserId.eq(user_id))
            .filter(plugin_capability_grant::Column::RevokedAt.is_null())
            .order_by_asc(plugin_capability_grant::Column::Capability)
            .order_by_asc(plugin_capability_grant::Column::Operation)
            .order_by_asc(plugin_capability_grant::Column::ResourceType)
            .order_by_asc(plugin_capability_grant::Column::ResourceId)
            .order_by_asc(plugin_capability_grant::Column::Id)
            .all(&self.database)
            .await
            .map_err(database_error)?
            .into_iter()
            .map(grant_from_model)
            .collect()
    }

    pub async fn get_kv(
        &self,
        plugin_key: &str,
        user_id: &str,
        key: &str,
    ) -> Result<Option<PluginKvValue>, PluginRegistryError> {
        validate_plugin_key(plugin_key)?;
        validate_uuid(user_id, PluginRegistryErrorKind::InvalidInput)?;
        validate_kv_key(key)?;
        ensure_active_user(&self.database, user_id).await?;
        let installation = find_installation(&self.database, plugin_key).await?;
        plugin_kv::Entity::find_by_id((
            installation.id().to_owned(),
            user_id.to_owned(),
            key.to_owned(),
        ))
        .one(&self.database)
        .await
        .map_err(database_error)?
        .map(kv_from_model)
        .transpose()
    }

    pub async fn put_kv(
        &self,
        plugin_key: &str,
        user_id: &str,
        key: &str,
        expected_revision: Option<u64>,
        value: Vec<u8>,
    ) -> Result<PluginKvValue, PluginRegistryError> {
        validate_plugin_key(plugin_key)?;
        validate_uuid(user_id, PluginRegistryErrorKind::InvalidInput)?;
        validate_kv_key(key)?;
        if value.len() > MAX_KV_VALUE_BYTES {
            return Err(quota_exceeded());
        }
        let transaction = self.database.begin().await.map_err(database_error)?;
        let backend = self.database.get_database_backend();
        let result = async {
            lock_active_user(&transaction, backend, user_id).await?;
            let installation = find_installation(&transaction, plugin_key).await?;
            let id = (
                installation.id().to_owned(),
                user_id.to_owned(),
                key.to_owned(),
            );
            let existing = plugin_kv::Entity::find_by_id(id.clone())
                .one(&transaction)
                .await
                .map_err(database_error)?;
            let current = existing.clone().map(kv_from_model).transpose()?;
            match (&current, expected_revision) {
                (None, Some(_)) => return Err(not_found()),
                (Some(_), None) => return Err(revision_conflict()),
                (Some(current), Some(expected)) if current.revision() != expected => {
                    return Err(revision_conflict());
                }
                (None, None) | (Some(_), Some(_)) => {}
            }

            validate_kv_quota(
                &transaction,
                installation.id(),
                user_id,
                current.as_ref(),
                value.len(),
            )
            .await?;
            let now = OffsetDateTime::now_utc();
            let value_size = usize_to_i32(value.len())?;
            let model = match current {
                None => plugin_kv::ActiveModel {
                    plugin_id: Set(installation.id().to_owned()),
                    owner_user_id: Set(user_id.to_owned()),
                    key: Set(key.to_owned()),
                    value: Set(value),
                    value_size_bytes: Set(value_size),
                    revision: Set(0),
                    created_at: Set(now),
                    updated_at: Set(now),
                }
                .insert(&transaction)
                .await
                .map_err(database_error)?,
                Some(current) => {
                    let next_revision = next_revision(current.revision())?;
                    let update = plugin_kv::Entity::update_many()
                        .col_expr(plugin_kv::Column::Value, Expr::value(value))
                        .col_expr(plugin_kv::Column::ValueSizeBytes, Expr::value(value_size))
                        .col_expr(
                            plugin_kv::Column::Revision,
                            Expr::value(u64_to_i64(next_revision)?),
                        )
                        .col_expr(plugin_kv::Column::UpdatedAt, Expr::value(now))
                        .filter(plugin_kv::Column::PluginId.eq(installation.id()))
                        .filter(plugin_kv::Column::OwnerUserId.eq(user_id))
                        .filter(plugin_kv::Column::Key.eq(key))
                        .filter(plugin_kv::Column::Revision.eq(u64_to_i64(current.revision())?))
                        .exec(&transaction)
                        .await
                        .map_err(database_error)?;
                    if update.rows_affected != 1 {
                        return Err(revision_conflict());
                    }
                    plugin_kv::Entity::find_by_id(id)
                        .one(&transaction)
                        .await
                        .map_err(database_error)?
                        .ok_or_else(corrupt_data)?
                }
            };
            kv_from_model(model)
        }
        .await;
        finish_transaction(transaction, result).await
    }

    pub async fn delete_kv(
        &self,
        plugin_key: &str,
        user_id: &str,
        key: &str,
        expected_revision: u64,
    ) -> Result<(), PluginRegistryError> {
        validate_plugin_key(plugin_key)?;
        validate_uuid(user_id, PluginRegistryErrorKind::InvalidInput)?;
        validate_kv_key(key)?;
        let transaction = self.database.begin().await.map_err(database_error)?;
        let backend = self.database.get_database_backend();
        let result = async {
            lock_active_user(&transaction, backend, user_id).await?;
            let installation = find_installation(&transaction, plugin_key).await?;
            let model = plugin_kv::Entity::find_by_id((
                installation.id().to_owned(),
                user_id.to_owned(),
                key.to_owned(),
            ))
            .one(&transaction)
            .await
            .map_err(database_error)?
            .ok_or_else(not_found)?;
            let current = kv_from_model(model)?;
            if current.revision() != expected_revision {
                return Err(revision_conflict());
            }
            let deleted = plugin_kv::Entity::delete_many()
                .filter(plugin_kv::Column::PluginId.eq(installation.id()))
                .filter(plugin_kv::Column::OwnerUserId.eq(user_id))
                .filter(plugin_kv::Column::Key.eq(key))
                .filter(plugin_kv::Column::Revision.eq(u64_to_i64(expected_revision)?))
                .exec(&transaction)
                .await
                .map_err(database_error)?;
            if deleted.rows_affected != 1 {
                return Err(revision_conflict());
            }
            Ok(())
        }
        .await;
        finish_transaction(transaction, result).await
    }

    async fn sync_bundled_once(
        &self,
        bundle: &BundledOfficialPlugin,
    ) -> Result<PluginInstallation, PluginRegistryError> {
        let transaction = self.database.begin().await.map_err(database_error)?;
        let result = async {
            let existing = plugin_installation::Entity::find()
                .filter(plugin_installation::Column::PluginKey.eq(bundle.plugin_key()))
                .one(&transaction)
                .await
                .map_err(database_error)?;
            let now = OffsetDateTime::now_utc();
            let model = match existing {
                None => {
                    let insert = plugin_installation::ActiveModel {
                        id: Set(Uuid::new_v4().to_string()),
                        plugin_key: Set(bundle.plugin_key().to_owned()),
                        version: Set(bundle.version().to_owned()),
                        abi_version: Set(bundle.abi_version().to_owned()),
                        distribution: Set(OFFICIAL_DISTRIBUTION.to_owned()),
                        component_digest: Set(bundle.component_digest().to_owned()),
                        manifest_json: Set(bundle.manifest_json().to_owned()),
                        signature_key_id: Set(bundle.signature_key_id().to_owned()),
                        signature: Set(bundle.signature().to_owned()),
                        system_state: Set(PluginSystemState::Enabled.as_storage().to_owned()),
                        revision: Set(0),
                        installed_at: Set(now),
                        updated_at: Set(now),
                    }
                    .insert(&transaction)
                    .await;
                    match insert {
                        Ok(model) => model,
                        Err(error) if is_unique_violation(&error) => {
                            return Err(revision_conflict());
                        }
                        Err(error) => return Err(database_error(error)),
                    }
                }
                Some(existing) => {
                    let current = installation_from_model(existing.clone())?;
                    if existing.distribution != OFFICIAL_DISTRIBUTION {
                        return Err(corrupt_data());
                    }
                    if existing.version == bundle.version()
                        && existing.abi_version == bundle.abi_version()
                        && existing.component_digest == bundle.component_digest()
                        && existing.manifest_json == bundle.manifest_json()
                        && existing.signature_key_id == bundle.signature_key_id()
                        && existing.signature == bundle.signature()
                    {
                        return Ok(current);
                    }
                    let next_revision = next_revision(current.revision())?;
                    let update = plugin_installation::Entity::update_many()
                        .col_expr(
                            plugin_installation::Column::Version,
                            Expr::value(bundle.version().to_owned()),
                        )
                        .col_expr(
                            plugin_installation::Column::AbiVersion,
                            Expr::value(bundle.abi_version().to_owned()),
                        )
                        .col_expr(
                            plugin_installation::Column::ComponentDigest,
                            Expr::value(bundle.component_digest().to_owned()),
                        )
                        .col_expr(
                            plugin_installation::Column::ManifestJson,
                            Expr::value(bundle.manifest_json().to_owned()),
                        )
                        .col_expr(
                            plugin_installation::Column::SignatureKeyId,
                            Expr::value(bundle.signature_key_id().to_owned()),
                        )
                        .col_expr(
                            plugin_installation::Column::Signature,
                            Expr::value(bundle.signature().to_owned()),
                        )
                        .col_expr(
                            plugin_installation::Column::Revision,
                            Expr::value(u64_to_i64(next_revision)?),
                        )
                        .col_expr(plugin_installation::Column::UpdatedAt, Expr::value(now))
                        .filter(plugin_installation::Column::Id.eq(&existing.id))
                        .filter(
                            plugin_installation::Column::Revision
                                .eq(u64_to_i64(current.revision())?),
                        )
                        .exec(&transaction)
                        .await
                        .map_err(database_error)?;
                    if update.rows_affected != 1 {
                        return Err(revision_conflict());
                    }
                    plugin_installation::Entity::find_by_id(existing.id)
                        .one(&transaction)
                        .await
                        .map_err(database_error)?
                        .ok_or_else(corrupt_data)?
                }
            };
            installation_from_model(model)
        }
        .await;
        finish_transaction(transaction, result).await
    }
}

struct ValidatedGrantInput {
    plugin_key: String,
    owner_user_id: String,
    expected_revision: Option<u64>,
    capability: String,
    operation: String,
    resource_type: String,
    resource_id: String,
    grant_key_hash: String,
    constraints_json: String,
}

impl ValidatedGrantInput {
    fn new(input: CapabilityGrantInput) -> Result<Self, PluginRegistryError> {
        validate_plugin_key(&input.plugin_key)?;
        validate_uuid(&input.owner_user_id, PluginRegistryErrorKind::InvalidInput)?;
        validate_grant_identity(
            &input.capability,
            &input.operation,
            &input.resource_type,
            &input.resource_id,
            PluginRegistryErrorKind::InvalidInput,
        )?;
        let constraints_json = canonical_constraints(
            &input.constraints_json,
            PluginRegistryErrorKind::InvalidInput,
        )?;
        let grant_key_hash = grant_key_hash(
            &input.plugin_key,
            &input.owner_user_id,
            &input.capability,
            &input.operation,
            &input.resource_type,
            &input.resource_id,
        );
        Ok(Self {
            plugin_key: input.plugin_key,
            owner_user_id: input.owner_user_id,
            expected_revision: input.expected_revision,
            capability: input.capability,
            operation: input.operation,
            resource_type: input.resource_type,
            resource_id: input.resource_id,
            grant_key_hash,
            constraints_json,
        })
    }

    fn matches(&self, grant: &PluginCapabilityGrant) -> Result<(), PluginRegistryError> {
        if self.capability == grant.capability
            && self.operation == grant.operation
            && self.resource_type == grant.resource_type
            && self.resource_id == grant.resource_id
            && self.grant_key_hash == grant.grant_key_hash
        {
            Ok(())
        } else {
            Err(corrupt_data())
        }
    }
}

async fn find_installation<C>(
    connection: &C,
    plugin_key: &str,
) -> Result<PluginInstallation, PluginRegistryError>
where
    C: ConnectionTrait,
{
    let model = plugin_installation::Entity::find()
        .filter(plugin_installation::Column::PluginKey.eq(plugin_key))
        .one(connection)
        .await
        .map_err(database_error)?
        .ok_or_else(not_found)?;
    installation_from_model(model)
}

fn installation_from_model(
    model: plugin_installation::Model,
) -> Result<PluginInstallation, PluginRegistryError> {
    validate_uuid(&model.id, PluginRegistryErrorKind::CorruptData)?;
    validate_plugin_key(&model.plugin_key).map_err(|_| corrupt_data())?;
    if model.distribution != OFFICIAL_DISTRIBUTION
        || model.abi_version != "raindrop:content-plugin@1.0.0"
        || model.version.is_empty()
        || model.version.len() > 64
        || model.installed_at > model.updated_at
    {
        return Err(corrupt_data());
    }
    validate_lower_hex_hash(
        &model.component_digest,
        PluginRegistryErrorKind::CorruptData,
    )?;
    validate_visible_ascii(
        &model.signature_key_id,
        128,
        PluginRegistryErrorKind::CorruptData,
    )?;
    validate_visible_ascii(&model.signature, 128, PluginRegistryErrorKind::CorruptData)?;
    validate_persisted_manifest(
        &model.manifest_json,
        &model.plugin_key,
        &model.version,
        &model.abi_version,
        &model.component_digest,
        &model.signature_key_id,
        &model.signature,
    )?;
    let revision = i64_to_u64(model.revision)?;
    let system_state =
        PluginSystemState::from_storage(&model.system_state).ok_or_else(corrupt_data)?;
    Ok(PluginInstallation {
        id: model.id,
        plugin_key: model.plugin_key,
        version: model.version,
        abi_version: model.abi_version,
        component_digest: model.component_digest,
        system_state,
        revision,
        installed_at: model.installed_at,
        updated_at: model.updated_at,
    })
}

fn config_from_model(model: plugin_config::Model) -> Result<PluginConfig, PluginRegistryError> {
    validate_uuid(&model.id, PluginRegistryErrorKind::CorruptData)?;
    validate_uuid(&model.plugin_id, PluginRegistryErrorKind::CorruptData)?;
    validate_uuid(&model.owner_user_id, PluginRegistryErrorKind::CorruptData)?;
    if model.schema_version != 1 || model.created_at > model.updated_at {
        return Err(corrupt_data());
    }
    let config =
        AiContentConfig::parse(model.config_json.as_bytes()).map_err(|_| corrupt_data())?;
    if config.canonical_json() != model.config_json || config.config_hash() != model.config_hash {
        return Err(corrupt_data());
    }
    Ok(PluginConfig {
        id: model.id,
        plugin_id: model.plugin_id,
        owner_user_id: model.owner_user_id,
        is_enabled: model.is_enabled,
        revision: i64_to_u64(model.revision)?,
        config,
        created_at: model.created_at,
        updated_at: model.updated_at,
    })
}

fn grant_from_model(
    model: plugin_capability_grant::Model,
) -> Result<PluginCapabilityGrant, PluginRegistryError> {
    validate_uuid(&model.id, PluginRegistryErrorKind::CorruptData)?;
    validate_uuid(&model.plugin_id, PluginRegistryErrorKind::CorruptData)?;
    validate_uuid(&model.owner_user_id, PluginRegistryErrorKind::CorruptData)?;
    validate_grant_identity(
        &model.capability,
        &model.operation,
        &model.resource_type,
        &model.resource_id,
        PluginRegistryErrorKind::CorruptData,
    )?;
    validate_lower_hex_hash(&model.grant_key_hash, PluginRegistryErrorKind::CorruptData)?;
    let expected_hash = grant_key_hash(
        "raindrop.ai-content",
        &model.owner_user_id,
        &model.capability,
        &model.operation,
        &model.resource_type,
        &model.resource_id,
    );
    if expected_hash != model.grant_key_hash
        || model.created_at > model.updated_at
        || model
            .revoked_at
            .is_some_and(|revoked| revoked < model.created_at)
    {
        return Err(corrupt_data());
    }
    let constraints = canonical_constraints(
        model.constraints_json.as_bytes(),
        PluginRegistryErrorKind::CorruptData,
    )?;
    if constraints != model.constraints_json {
        return Err(corrupt_data());
    }
    Ok(PluginCapabilityGrant {
        id: model.id,
        plugin_id: model.plugin_id,
        owner_user_id: model.owner_user_id,
        capability: model.capability,
        operation: model.operation,
        resource_type: model.resource_type,
        resource_id: model.resource_id,
        grant_key_hash: model.grant_key_hash,
        constraints_json: model.constraints_json,
        revision: i64_to_u64(model.revision)?,
        created_at: model.created_at,
        updated_at: model.updated_at,
        revoked_at: model.revoked_at,
    })
}

fn kv_from_model(model: plugin_kv::Model) -> Result<PluginKvValue, PluginRegistryError> {
    validate_uuid(&model.plugin_id, PluginRegistryErrorKind::CorruptData)?;
    validate_uuid(&model.owner_user_id, PluginRegistryErrorKind::CorruptData)?;
    validate_kv_key(&model.key).map_err(|_| corrupt_data())?;
    if model.value_size_bytes < 0
        || usize::try_from(model.value_size_bytes).ok() != Some(model.value.len())
        || model.value.len() > MAX_KV_VALUE_BYTES
        || model.created_at > model.updated_at
    {
        return Err(corrupt_data());
    }
    Ok(PluginKvValue {
        plugin_id: model.plugin_id,
        owner_user_id: model.owner_user_id,
        key: model.key,
        value: model.value,
        revision: i64_to_u64(model.revision)?,
        created_at: model.created_at,
        updated_at: model.updated_at,
    })
}

async fn validate_kv_quota<C>(
    connection: &C,
    plugin_id: &str,
    user_id: &str,
    existing: Option<&PluginKvValue>,
    replacement_size: usize,
) -> Result<(), PluginRegistryError>
where
    C: ConnectionTrait,
{
    let rows = plugin_kv::Entity::find()
        .filter(plugin_kv::Column::PluginId.eq(plugin_id))
        .filter(plugin_kv::Column::OwnerUserId.eq(user_id))
        .select_only()
        .column(plugin_kv::Column::Key)
        .column(plugin_kv::Column::ValueSizeBytes)
        .into_tuple::<(String, i32)>()
        .all(connection)
        .await
        .map_err(database_error)?;
    let mut total = 0_usize;
    let mut seen = HashSet::new();
    for (key, size) in &rows {
        validate_kv_key(key).map_err(|_| corrupt_data())?;
        let size = usize::try_from(*size).map_err(|_| corrupt_data())?;
        if size > MAX_KV_VALUE_BYTES || !seen.insert(key) {
            return Err(corrupt_data());
        }
        total = total.checked_add(size).ok_or_else(corrupt_data)?;
    }
    if rows.len() > MAX_KV_KEYS || total > MAX_KV_TOTAL_BYTES {
        return Err(corrupt_data());
    }
    let next_count = rows.len() + usize::from(existing.is_none());
    let previous_size = existing.map_or(0, |value| value.value.len());
    let next_total = total
        .checked_sub(previous_size)
        .and_then(|value| value.checked_add(replacement_size))
        .ok_or_else(corrupt_data)?;
    if next_count > MAX_KV_KEYS || next_total > MAX_KV_TOTAL_BYTES {
        return Err(quota_exceeded());
    }
    Ok(())
}

fn canonical_constraints(
    input: &[u8],
    kind: PluginRegistryErrorKind,
) -> Result<String, PluginRegistryError> {
    let value = parse_unique_json(input, MAX_CONSTRAINTS_BYTES)
        .map_err(|_| PluginRegistryError::new(kind))?;
    if !value.is_object() || contains_sensitive_key(&value) {
        return Err(PluginRegistryError::new(kind));
    }
    canonical_json(value, MAX_CONSTRAINTS_BYTES).map_err(|_| PluginRegistryError::new(kind))
}

fn contains_sensitive_key(value: &Value) -> bool {
    match value {
        Value::Object(object) => object.iter().any(|(key, value)| {
            let normalized = key
                .bytes()
                .filter(|byte| byte.is_ascii_alphanumeric())
                .map(|byte| byte.to_ascii_lowercase())
                .collect::<Vec<_>>();
            [
                b"secret".as_slice(),
                b"token".as_slice(),
                b"password".as_slice(),
                b"credential".as_slice(),
                b"apikey".as_slice(),
                b"authorization".as_slice(),
                b"header".as_slice(),
            ]
            .iter()
            .any(|needle| normalized == *needle || normalized.ends_with(needle))
                || contains_sensitive_key(value)
        }),
        Value::Array(values) => values.iter().any(contains_sensitive_key),
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => false,
    }
}

fn validate_grant_identity(
    capability: &str,
    operation: &str,
    resource_type: &str,
    resource_id: &str,
    kind: PluginRegistryErrorKind,
) -> Result<(), PluginRegistryError> {
    let valid = matches!(operation, "SUMMARIZE" | "TRANSLATE")
        && matches!(
            (capability, resource_type),
            ("ai.generate_structured", "AI_PROVIDER") | ("mcp.call_tool", "MCP_TOOL")
        );
    if !valid {
        return Err(PluginRegistryError::new(kind));
    }
    validate_uuid(resource_id, kind)
}

fn grant_key_hash(
    plugin_key: &str,
    user_id: &str,
    capability: &str,
    operation: &str,
    resource_type: &str,
    resource_id: &str,
) -> String {
    let mut hasher = Hasher::new_derive_key(GRANT_HASH_CONTEXT);
    for frame in [
        plugin_key.as_bytes(),
        user_id.as_bytes(),
        capability.as_bytes(),
        operation.as_bytes(),
        resource_type.as_bytes(),
        resource_id.as_bytes(),
    ] {
        hasher.update(&(frame.len() as u64).to_be_bytes());
        hasher.update(frame);
    }
    hasher.finalize().to_hex().to_string()
}

fn validate_plugin_key(value: &str) -> Result<(), PluginRegistryError> {
    if !value.is_empty()
        && value.len() <= 128
        && value.as_bytes()[0].is_ascii_lowercase()
        && value.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'.' | b'-' | b'_')
        })
    {
        Ok(())
    } else {
        Err(PluginRegistryError::new(
            PluginRegistryErrorKind::InvalidInput,
        ))
    }
}

fn validate_kv_key(value: &str) -> Result<(), PluginRegistryError> {
    let first_is_valid = value
        .as_bytes()
        .first()
        .is_some_and(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit());
    if !value.is_empty()
        && value.len() <= 128
        && first_is_valid
        && value.bytes().all(|byte| {
            byte.is_ascii_lowercase()
                || byte.is_ascii_digit()
                || matches!(byte, b'.' | b'_' | b'/' | b'-')
        })
    {
        Ok(())
    } else {
        Err(PluginRegistryError::new(
            PluginRegistryErrorKind::InvalidInput,
        ))
    }
}

async fn ensure_active_user<C>(connection: &C, user_id: &str) -> Result<(), PluginRegistryError>
where
    C: ConnectionTrait,
{
    let model = user::Entity::find_by_id(user_id)
        .one(connection)
        .await
        .map_err(database_error)?
        .ok_or_else(not_found)?;
    if model.is_disabled {
        Err(not_found())
    } else {
        Ok(())
    }
}

async fn lock_active_user<C>(
    connection: &C,
    backend: DbBackend,
    user_id: &str,
) -> Result<(), PluginRegistryError>
where
    C: ConnectionTrait,
{
    match backend {
        DatabaseBackend::Sqlite => {
            let result = connection
                .execute(Statement::from_sql_and_values(
                    backend,
                    "UPDATE users SET is_disabled = is_disabled WHERE id = ? AND is_disabled = FALSE",
                    [user_id.into()],
                ))
                .await
                .map_err(database_error)?;
            if result.rows_affected() != 1 {
                return Err(not_found());
            }
        }
        DatabaseBackend::Postgres | DatabaseBackend::MySql => {
            let sql = if backend == DatabaseBackend::Postgres {
                "SELECT is_disabled FROM users WHERE id = $1 FOR UPDATE"
            } else {
                "SELECT is_disabled FROM users WHERE id = ? FOR UPDATE"
            };
            let row = connection
                .query_one(Statement::from_sql_and_values(
                    backend,
                    sql,
                    [user_id.into()],
                ))
                .await
                .map_err(database_error)?
                .ok_or_else(not_found)?;
            let is_disabled = bool::try_get(&row, "", "is_disabled").map_err(|_| corrupt_data())?;
            if is_disabled {
                return Err(not_found());
            }
        }
    }
    Ok(())
}

async fn finish_transaction<T>(
    transaction: sea_orm::DatabaseTransaction,
    result: Result<T, PluginRegistryError>,
) -> Result<T, PluginRegistryError> {
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

fn next_revision(current: u64) -> Result<u64, PluginRegistryError> {
    current.checked_add(1).ok_or_else(corrupt_data)
}

fn u64_to_i64(value: u64) -> Result<i64, PluginRegistryError> {
    i64::try_from(value).map_err(|_| corrupt_data())
}

fn i64_to_u64(value: i64) -> Result<u64, PluginRegistryError> {
    u64::try_from(value).map_err(|_| corrupt_data())
}

fn usize_to_i32(value: usize) -> Result<i32, PluginRegistryError> {
    i32::try_from(value).map_err(|_| corrupt_data())
}

fn not_found() -> PluginRegistryError {
    PluginRegistryError::new(PluginRegistryErrorKind::NotFound)
}

fn revision_conflict() -> PluginRegistryError {
    PluginRegistryError::new(PluginRegistryErrorKind::RevisionConflict)
}

fn quota_exceeded() -> PluginRegistryError {
    PluginRegistryError::new(PluginRegistryErrorKind::QuotaExceeded)
}

fn corrupt_data() -> PluginRegistryError {
    PluginRegistryError::new(PluginRegistryErrorKind::CorruptData)
}

fn database_error(_: DbErr) -> PluginRegistryError {
    PluginRegistryError::new(PluginRegistryErrorKind::Database)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum UniqueViolationProvenance<'a> {
    Postgres(&'a str),
    MySql(u16),
    Sqlite(i32),
    Other,
}

fn is_unique_violation(error: &DbErr) -> bool {
    let runtime = match error {
        DbErr::Conn(runtime) | DbErr::Exec(runtime) | DbErr::Query(runtime) => runtime,
        _ => return false,
    };
    let RuntimeErr::SqlxError(SqlxError::Database(database_error)) = runtime else {
        return false;
    };
    let provenance = if let Some(error) = database_error.try_downcast_ref::<SqlxPostgresError>() {
        UniqueViolationProvenance::Postgres(error.code())
    } else if let Some(error) = database_error.try_downcast_ref::<SqlxMySqlError>() {
        UniqueViolationProvenance::MySql(error.number())
    } else if database_error
        .try_downcast_ref::<SqlxSqliteError>()
        .is_some()
    {
        database_error
            .code()
            .as_deref()
            .and_then(|code| code.parse::<i32>().ok())
            .map_or(
                UniqueViolationProvenance::Other,
                UniqueViolationProvenance::Sqlite,
            )
    } else {
        UniqueViolationProvenance::Other
    };
    matches!(
        provenance,
        UniqueViolationProvenance::Postgres("23505")
            | UniqueViolationProvenance::MySql(1062)
            | UniqueViolationProvenance::Sqlite(1555 | 2067)
    )
}
