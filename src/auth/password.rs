use std::sync::{Arc, LazyLock};

use argon2::{
    Algorithm, Argon2, Params, Version,
    password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
};
use rand_core::OsRng;
use secrecy::{ExposeSecret, SecretString};
use tokio::sync::Semaphore;

const MEMORY_COST_KIB: u32 = 19_456;
const TIME_COST: u32 = 2;
const PARALLELISM: u32 = 1;
// Four Argon2 operations bound simultaneous working memory to about 76 MiB.
static PASSWORD_TASK_SLOTS: LazyLock<Arc<Semaphore>> =
    LazyLock::new(|| Arc::new(Semaphore::new(4)));

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
    pub(crate) async fn hash(&self, password: &SecretString) -> Result<String, PasswordError> {
        let argon2 = self.argon2.clone();
        let password = password.clone();
        run_password_task(Arc::clone(&PASSWORD_TASK_SLOTS), move || {
            let salt = SaltString::generate(&mut OsRng);
            argon2
                .hash_password(password.expose_secret().as_bytes(), &salt)
                .map(|hash| hash.to_string())
                .map_err(PasswordError::Hash)
        })
        .await
    }

    pub(crate) async fn verify(
        &self,
        encoded: &str,
        password: &SecretString,
    ) -> Result<bool, PasswordError> {
        let argon2 = self.argon2.clone();
        let encoded = encoded.to_owned();
        let password = password.clone();
        run_password_task(Arc::clone(&PASSWORD_TASK_SLOTS), move || {
            let hash = PasswordHash::new(&encoded).map_err(PasswordError::Hash)?;
            match argon2.verify_password(password.expose_secret().as_bytes(), &hash) {
                Ok(()) => Ok(true),
                Err(argon2::password_hash::Error::Password) => Ok(false),
                Err(error) => Err(PasswordError::Hash(error)),
            }
        })
        .await
    }
}

async fn run_password_task<T: Send + 'static>(
    slots: Arc<Semaphore>,
    operation: impl FnOnce() -> Result<T, PasswordError> + Send + 'static,
) -> Result<T, PasswordError> {
    let permit = slots
        .acquire_owned()
        .await
        .map_err(PasswordError::Unavailable)?;
    tokio::task::spawn_blocking(move || {
        // Cancelling the request cannot cancel started blocking work. Keep its capacity reserved.
        let _permit = permit;
        operation()
    })
    .await
    .map_err(PasswordError::Worker)?
}

#[derive(Debug, thiserror::Error)]
pub enum PasswordError {
    #[error("Argon2id password operation failed")]
    Hash(#[source] argon2::password_hash::Error),
    #[error("password worker failed")]
    Worker(#[source] tokio::task::JoinError),
    #[error("password worker capacity is closed")]
    Unavailable(#[source] tokio::sync::AcquireError),
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::{Semaphore, oneshot};

    #[tokio::test]
    async fn password_verification_preserves_invalid_hash_parameter_errors() {
        let encoded =
            "$argon2id$v=19$m=1,t=2,p=1$c29tZXNhbHQ$AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";
        PasswordHash::new(encoded).expect("the PHC format parses before parameter validation");
        let result = PasswordService::default()
            .verify(encoded, &SecretString::from("password"))
            .await;

        assert!(
            matches!(result, Err(PasswordError::Hash(_))),
            "invalid stored Argon2 parameters must remain an operational error: {result:?}"
        );
    }

    #[tokio::test]
    async fn password_verification_distinguishes_matches_from_incorrect_passwords() {
        let passwords = PasswordService::default();
        let correct = SecretString::from("correct password");
        let encoded = passwords
            .hash(&correct)
            .await
            .expect("password should hash");

        assert!(passwords.verify(&encoded, &correct).await.unwrap());
        assert!(
            !passwords
                .verify(&encoded, &SecretString::from("incorrect password"))
                .await
                .unwrap(),
            "an incorrect password remains a normal authentication failure"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn blocking_password_work_leaves_the_runtime_responsive() {
        let (started, ready) = oneshot::channel();
        let (release, gate) = std::sync::mpsc::channel();
        let task = tokio::spawn(run_password_task(Arc::new(Semaphore::new(1)), move || {
            started.send(()).expect("test receiver is waiting");
            gate.recv().expect("test releases the worker");
            Ok(42)
        }));
        ready
            .await
            .expect("blocking task starts without blocking the runtime");
        tokio::task::yield_now().await;
        assert!(
            !task.is_finished(),
            "worker waits independently of the runtime"
        );
        release.send(()).expect("blocking worker is still alive");
        assert_eq!(task.await.unwrap().unwrap(), 42);
    }

    #[tokio::test]
    async fn cancelled_password_work_keeps_its_slot_until_the_worker_finishes() {
        let slots = Arc::new(Semaphore::new(1));
        let (started, ready) = oneshot::channel();
        let (release, gate) = std::sync::mpsc::channel();
        let task = tokio::spawn(run_password_task(Arc::clone(&slots), move || {
            started.send(()).expect("test receiver is waiting");
            gate.recv().expect("test releases the worker");
            Ok(())
        }));
        ready.await.unwrap();
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        assert!(
            slots.try_acquire().is_err(),
            "aborting the request must not release running CPU work"
        );
        release.send(()).unwrap();
        let permit = tokio::time::timeout(std::time::Duration::from_secs(5), slots.acquire())
            .await
            .expect("the worker releases its slot when finished")
            .unwrap();
        drop(permit);
        assert_eq!(slots.available_permits(), 1);
    }
}
