use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, Condition, DatabaseConnection, EntityTrait,
    QueryFilter, TransactionTrait,
};
use secrecy::{ExposeSecret, SecretString};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::db::entities::{bootstrap_state, user, user_role};

use super::{
    model::{
        AuthenticateError, CreateAdminError, CreateAdminInput, EmailError, LoginIdentifier, Role,
        User, UsernameError,
    },
    password::PasswordService,
};

pub fn normalize_username(value: &str) -> Result<String, UsernameError> {
    let trimmed = value.trim();
    let length = trimmed.chars().count();
    if !(3..=64).contains(&length) {
        return Err(UsernameError::InvalidLength);
    }
    if trimmed
        .chars()
        .any(|character| character.is_control() || character.is_whitespace())
    {
        return Err(UsernameError::InvalidCharacter);
    }
    Ok(trimmed.chars().flat_map(char::to_lowercase).collect())
}

pub async fn create_admin(
    database: &DatabaseConnection,
    passwords: &PasswordService,
    input: CreateAdminInput,
) -> Result<User, CreateAdminError> {
    let normalized_username = normalize_username(&input.username)?;
    if input.password.expose_secret().len() < 12 {
        return Err(CreateAdminError::InvalidPassword);
    }
    let email = normalize_optional_email(input.email)?;
    if username_exists(database, &normalized_username).await? {
        return Err(CreateAdminError::UsernameTaken);
    }

    let password_hash = passwords
        .hash(&input.password)
        .map_err(CreateAdminError::Password)?;
    let id = Uuid::new_v4().to_string();
    let transaction = database.begin().await.map_err(CreateAdminError::Database)?;
    if let Err(error) = (bootstrap_state::ActiveModel {
        id: Set(1),
        administrator_user_id: Set(id.clone()),
    })
    .insert(&transaction)
    .await
    {
        transaction
            .rollback()
            .await
            .map_err(CreateAdminError::Database)?;
        if bootstrap_state::Entity::find_by_id(1)
            .one(database)
            .await
            .map_err(CreateAdminError::Database)?
            .is_some()
        {
            return Err(CreateAdminError::AlreadyClaimed);
        }
        return Err(CreateAdminError::Database(error));
    }
    let inserted = user::ActiveModel {
        id: Set(id.clone()),
        username: Set(input.username.trim().to_owned()),
        normalized_username: Set(normalized_username.clone()),
        email: Set(email.clone()),
        password_hash: Set(password_hash),
        is_disabled: Set(false),
        created_at: Set(OffsetDateTime::now_utc()),
        last_login_at: Set(None),
    }
    .insert(&transaction)
    .await;

    if let Err(error) = inserted {
        let _ = transaction.rollback().await;
        if username_exists(database, &normalized_username).await? {
            return Err(CreateAdminError::UsernameTaken);
        }
        return Err(CreateAdminError::Database(error));
    }

    for role in [Role::Admin, Role::User] {
        user_role::ActiveModel {
            user_id: Set(id.clone()),
            role: Set(role_name(role).to_owned()),
        }
        .insert(&transaction)
        .await
        .map_err(CreateAdminError::Database)?;
    }
    transaction
        .commit()
        .await
        .map_err(CreateAdminError::Database)?;

    Ok(User {
        id,
        username: input.username.trim().to_owned(),
        email,
        is_disabled: false,
        roles: vec![Role::Admin, Role::User],
    })
}

pub(crate) fn validate_create_admin_input(
    input: &CreateAdminInput,
) -> Result<(), CreateAdminError> {
    normalize_username(&input.username)?;
    if input.password.expose_secret().len() < 12 {
        return Err(CreateAdminError::InvalidPassword);
    }
    normalize_optional_email(input.email.clone())?;
    Ok(())
}

pub async fn authenticate(
    database: &DatabaseConnection,
    passwords: &PasswordService,
    login: LoginIdentifier,
    password: &SecretString,
) -> Result<User, AuthenticateError> {
    let stored = user::Entity::find()
        .filter(
            Condition::any()
                .add(user::Column::NormalizedUsername.eq(login.as_str()))
                .add(user::Column::Email.eq(login.as_str())),
        )
        .one(database)
        .await
        .map_err(AuthenticateError::Database)?;

    let Some(stored) = stored else {
        let _ = passwords.hash(password);
        return Err(AuthenticateError::InvalidCredentials);
    };
    let valid = passwords
        .verify(&stored.password_hash, password)
        .map_err(AuthenticateError::Password)?;
    if !valid {
        return Err(AuthenticateError::InvalidCredentials);
    }
    if stored.is_disabled {
        return Err(AuthenticateError::Disabled);
    }

    let roles = load_roles(database, &stored.id)
        .await
        .map_err(AuthenticateError::Database)?;
    let mut active: user::ActiveModel = stored.clone().into();
    active.last_login_at = Set(Some(OffsetDateTime::now_utc()));
    active
        .update(database)
        .await
        .map_err(AuthenticateError::Database)?;

    Ok(User {
        id: stored.id,
        username: stored.username,
        email: stored.email,
        is_disabled: stored.is_disabled,
        roles,
    })
}

async fn username_exists(
    database: &DatabaseConnection,
    normalized_username: &str,
) -> Result<bool, CreateAdminError> {
    user::Entity::find()
        .filter(user::Column::NormalizedUsername.eq(normalized_username))
        .one(database)
        .await
        .map(|value| value.is_some())
        .map_err(CreateAdminError::Database)
}

pub(crate) async fn load_user_by_id(
    database: &DatabaseConnection,
    user_id: &str,
) -> Result<Option<User>, sea_orm::DbErr> {
    let stored = user::Entity::find_by_id(user_id).one(database).await?;
    let Some(stored) = stored else {
        return Ok(None);
    };
    let roles = load_roles(database, &stored.id).await?;
    Ok(Some(User {
        id: stored.id,
        username: stored.username,
        email: stored.email,
        is_disabled: stored.is_disabled,
        roles,
    }))
}

async fn load_roles(
    database: &DatabaseConnection,
    user_id: &str,
) -> Result<Vec<Role>, sea_orm::DbErr> {
    let stored = user_role::Entity::find()
        .filter(user_role::Column::UserId.eq(user_id))
        .all(database)
        .await?;
    Ok(stored
        .into_iter()
        .filter_map(|role| match role.role.as_str() {
            "ADMIN" => Some(Role::Admin),
            "USER" => Some(Role::User),
            _ => None,
        })
        .collect())
}

const fn role_name(role: Role) -> &'static str {
    match role {
        Role::Admin => "ADMIN",
        Role::User => "USER",
    }
}

fn normalize_optional_email(value: Option<String>) -> Result<Option<String>, EmailError> {
    let Some(value) = value else {
        return Ok(None);
    };
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    if trimmed.len() > 320
        || trimmed
            .chars()
            .any(|character| character.is_whitespace() || character.is_control())
    {
        return Err(EmailError);
    }
    let mut parts = trimmed.split('@');
    let local = parts.next().unwrap_or_default();
    let domain = parts.next().unwrap_or_default();
    if parts.next().is_some()
        || local.is_empty()
        || domain.is_empty()
        || local.len() > 64
        || domain.len() > 255
    {
        return Err(EmailError);
    }
    Ok(Some(trimmed.to_lowercase()))
}
