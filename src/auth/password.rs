use argon2::{
    Algorithm, Argon2, Params, Version,
    password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
};
use rand_core::OsRng;
use secrecy::{ExposeSecret, SecretString};

const MEMORY_COST_KIB: u32 = 19_456;
const TIME_COST: u32 = 2;
const PARALLELISM: u32 = 1;

pub struct PasswordService {
    argon2: Argon2<'static>,
}

impl Default for PasswordService {
    fn default() -> Self {
        let params = Params::new(MEMORY_COST_KIB, TIME_COST, PARALLELISM, None)
            .expect("constant Argon2id parameters must be valid");
        Self {
            argon2: Argon2::new(Algorithm::Argon2id, Version::V0x13, params),
        }
    }
}

impl PasswordService {
    pub(crate) fn hash(&self, password: &SecretString) -> Result<String, PasswordError> {
        let salt = SaltString::generate(&mut OsRng);
        self.argon2
            .hash_password(password.expose_secret().as_bytes(), &salt)
            .map(|hash| hash.to_string())
            .map_err(PasswordError::Hash)
    }

    pub(crate) fn verify(
        &self,
        encoded: &str,
        password: &SecretString,
    ) -> Result<bool, PasswordError> {
        let hash = PasswordHash::new(encoded).map_err(PasswordError::Hash)?;
        Ok(self
            .argon2
            .verify_password(password.expose_secret().as_bytes(), &hash)
            .is_ok())
    }
}

#[derive(Debug, thiserror::Error)]
pub enum PasswordError {
    #[error("Argon2id password operation failed")]
    Hash(#[source] argon2::password_hash::Error),
}
