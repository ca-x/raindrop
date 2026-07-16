use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, Condition, DatabaseConnection, EntityTrait,
    QueryFilter, TransactionTrait,
};
use secrecy::{ExposeSecret, SecretString};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::db::entities::{user, user_role};

use super::{
    model::{
        AuthenticateError, CreateAdminError, CreateAdminInput, LoginIdentifier, Role, User,
        UsernameError,
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
    if username_exists(database, &normalized_username).await? {
        return Err(CreateAdminError::UsernameTaken);
    }

    let password_hash = passwords
        .hash(&input.password)
        .map_err(CreateAdminError::Password)?;
    let id = Uuid::new_v4().to_string();
    let email = input.email.and_then(normalize_optional_email);
    let transaction = database.begin().await.map_err(CreateAdminError::Database)?;
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

    let roles = load_roles(database, &stored.id).await?;
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

async fn load_roles(
    database: &DatabaseConnection,
    user_id: &str,
) -> Result<Vec<Role>, AuthenticateError> {
    let stored = user_role::Entity::find()
        .filter(user_role::Column::UserId.eq(user_id))
        .all(database)
        .await
        .map_err(AuthenticateError::Database)?;
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

fn normalize_optional_email(value: String) -> Option<String> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_lowercase())
}
