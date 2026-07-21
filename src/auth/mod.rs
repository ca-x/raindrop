mod cookies;
mod extractor;
mod model;
mod password;
mod sessions;
mod users;

pub use cookies::{SESSION_COOKIE_NAME, build_clear_session_cookie, build_session_cookie};
pub use extractor::{AuthRejection, CsrfGuard, CurrentUser, SessionToken};
pub use model::{
    AuthenticateError, CreateAdminError, CreateAdminInput, DisplayNameError, EmailError,
    LoginIdentifier, ProfileError, Role, UpdateUserProfile, User, UserProfile, UsernameError,
};
pub use password::PasswordService;
pub use sessions::{CreatedSession, SessionDetails, SessionError, SessionService};
pub(crate) use users::validate_create_admin_input;
pub use users::{
    authenticate, create_admin, load_user_profile, normalize_username, update_user_profile,
};
