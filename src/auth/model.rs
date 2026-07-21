use secrecy::SecretString;
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct User {
    pub id: String,
    pub username: String,
    pub email: Option<String>,
    pub is_disabled: bool,
    pub roles: Vec<Role>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserProfile {
    pub user_id: String,
    pub username: String,
    pub display_name: Option<String>,
    pub email: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct UpdateUserProfile {
    pub display_name: Option<Option<String>>,
    pub email: Option<Option<String>>,
}

impl User {
    #[must_use]
    pub fn is_admin(&self) -> bool {
        self.roles.contains(&Role::Admin)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Role {
    Admin,
    User,
}

#[derive(Debug)]
pub struct CreateAdminInput {
    pub username: String,
    pub password: SecretString,
    pub email: Option<String>,
}

#[derive(Debug, Clone)]
pub struct LoginIdentifier(String);

impl LoginIdentifier {
    #[must_use]
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into().trim().to_lowercase())
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum UsernameError {
    #[error("username must contain between 3 and 64 characters")]
    InvalidLength,
    #[error("username cannot contain whitespace or control characters")]
    InvalidCharacter,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
#[error("email must be a valid unquoted address of at most 320 bytes")]
pub struct EmailError;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum DisplayNameError {
    #[error("display name must contain at most 80 characters")]
    InvalidLength,
    #[error("display name cannot contain control characters")]
    InvalidCharacter,
}

#[derive(Debug, thiserror::Error)]
pub enum ProfileError {
    #[error("profile update must contain at least one field")]
    EmptyPatch,
    #[error(transparent)]
    InvalidDisplayName(#[from] DisplayNameError),
    #[error(transparent)]
    InvalidEmail(#[from] EmailError),
    #[error("email address is already used by another account")]
    EmailTaken,
    #[error("profile was not found")]
    NotFound,
    #[error("database operation failed")]
    Database(#[source] sea_orm::DbErr),
}

#[derive(Debug, thiserror::Error)]
pub enum CreateAdminError {
    #[error(transparent)]
    InvalidUsername(#[from] UsernameError),
    #[error(transparent)]
    InvalidEmail(#[from] EmailError),
    #[error("password must not be empty")]
    InvalidPassword,
    #[error("username is already taken")]
    UsernameTaken,
    #[error("the first administrator has already been claimed")]
    AlreadyClaimed,
    #[error("password hashing failed")]
    Password(#[source] crate::auth::password::PasswordError),
    #[error("database operation failed")]
    Database(#[source] sea_orm::DbErr),
}

#[derive(Debug, thiserror::Error)]
pub enum AuthenticateError {
    #[error("invalid username, email, or password")]
    InvalidCredentials,
    #[error("account is disabled")]
    Disabled,
    #[error("password verification failed")]
    Password(#[source] crate::auth::password::PasswordError),
    #[error("database operation failed")]
    Database(#[source] sea_orm::DbErr),
}
