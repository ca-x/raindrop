use std::{collections::HashSet, fmt, str};

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use ring::{
    aead::{AES_256_GCM, Aad, LessSafeKey, Nonce, UnboundKey},
    rand::{SecureRandom, SystemRandom},
};
use secrecy::{ExposeSecret, SecretString};
use zeroize::Zeroize;

use super::ProviderKind;

const ENVELOPE_VERSION: &str = "rdsec1";
const AAD_PREFIX: &[u8] = b"raindrop.ai-provider-secret.v1\0";
const KEY_BYTES: usize = 32;
const NONCE_BYTES: usize = 12;
const TAG_BYTES: usize = 16;
const MAX_CREDENTIAL_BYTES: usize = 8_192;
const MAX_KEY_ID_BYTES: usize = 32;

struct KeySlot {
    id: String,
    key: LessSafeKey,
}

pub struct ProviderSecretKeyring {
    keys: Vec<KeySlot>,
    random: SystemRandom,
}

impl ProviderSecretKeyring {
    pub fn from_entries(entries: &[SecretString]) -> Result<Self, ProviderSecretError> {
        let keys = parse_entries(entries)?;
        Ok(Self {
            keys,
            random: SystemRandom::new(),
        })
    }

    pub fn validate_entries(entries: &[SecretString]) -> Result<(), ProviderSecretError> {
        parse_entries(entries).map(|_| ())
    }

    #[must_use]
    pub fn active_key_id(&self) -> &str {
        &self.keys[0].id
    }

    pub fn encrypt(
        &self,
        provider_id: &str,
        kind: ProviderKind,
        credential: &SecretString,
    ) -> Result<String, ProviderSecretError> {
        let plaintext = credential.expose_secret().as_bytes();
        if !(1..=MAX_CREDENTIAL_BYTES).contains(&plaintext.len()) {
            return Err(ProviderSecretError::new(
                ProviderSecretErrorKind::InvalidCredential,
            ));
        }

        let active = self
            .keys
            .first()
            .ok_or_else(|| ProviderSecretError::new(ProviderSecretErrorKind::InvalidKeyring))?;
        let mut nonce_bytes = [0_u8; NONCE_BYTES];
        self.random
            .fill(&mut nonce_bytes)
            .map_err(|_| ProviderSecretError::new(ProviderSecretErrorKind::EncryptFailed))?;
        let nonce = Nonce::assume_unique_for_key(nonce_bytes);
        let mut ciphertext = plaintext.to_vec();
        let aad = associated_data(provider_id, kind);
        if active
            .key
            .seal_in_place_append_tag(nonce, Aad::from(aad.as_slice()), &mut ciphertext)
            .is_err()
        {
            ciphertext.zeroize();
            return Err(ProviderSecretError::new(
                ProviderSecretErrorKind::EncryptFailed,
            ));
        }

        let nonce = URL_SAFE_NO_PAD.encode(nonce_bytes);
        let encoded = URL_SAFE_NO_PAD.encode(&ciphertext);
        ciphertext.zeroize();
        Ok(format!(
            "{ENVELOPE_VERSION}.{}.{nonce}.{encoded}",
            active.id
        ))
    }

    pub fn decrypt(
        &self,
        provider_id: &str,
        kind: ProviderKind,
        envelope: &str,
    ) -> Result<SecretString, ProviderSecretError> {
        let mut parts = envelope.split('.');
        let Some(version) = parts.next() else {
            return Err(decrypt_failed());
        };
        let Some(key_id) = parts.next() else {
            return Err(decrypt_failed());
        };
        let Some(nonce) = parts.next() else {
            return Err(decrypt_failed());
        };
        let Some(ciphertext) = parts.next() else {
            return Err(decrypt_failed());
        };
        if parts.next().is_some() || version != ENVELOPE_VERSION {
            return Err(decrypt_failed());
        }
        let slot = self
            .keys
            .iter()
            .find(|slot| slot.id == key_id)
            .ok_or_else(decrypt_failed)?;

        let nonce_bytes = decode_canonical(nonce).map_err(|_| decrypt_failed())?;
        if nonce_bytes.len() != NONCE_BYTES {
            return Err(decrypt_failed());
        }
        let nonce = Nonce::try_assume_unique_for_key(&nonce_bytes).map_err(|_| decrypt_failed())?;
        let mut ciphertext = decode_canonical(ciphertext).map_err(|_| decrypt_failed())?;
        if !(TAG_BYTES + 1..=MAX_CREDENTIAL_BYTES + TAG_BYTES).contains(&ciphertext.len()) {
            ciphertext.zeroize();
            return Err(decrypt_failed());
        }
        let aad = associated_data(provider_id, kind);
        let plaintext =
            match slot
                .key
                .open_in_place(nonce, Aad::from(aad.as_slice()), &mut ciphertext)
            {
                Ok(plaintext) => plaintext,
                Err(_) => {
                    ciphertext.zeroize();
                    return Err(decrypt_failed());
                }
            };
        if !(1..=MAX_CREDENTIAL_BYTES).contains(&plaintext.len()) {
            ciphertext.zeroize();
            return Err(decrypt_failed());
        }
        let value = match str::from_utf8(plaintext) {
            Ok(value) => value.to_owned(),
            Err(_) => {
                ciphertext.zeroize();
                return Err(decrypt_failed());
            }
        };
        ciphertext.zeroize();
        Ok(SecretString::from(value))
    }
}

impl fmt::Debug for ProviderSecretKeyring {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProviderSecretKeyring")
            .field("active_key_id", &self.active_key_id())
            .field("key_count", &self.keys.len())
            .finish()
    }
}

fn parse_entries(entries: &[SecretString]) -> Result<Vec<KeySlot>, ProviderSecretError> {
    if entries.is_empty() {
        return Err(ProviderSecretError::new(
            ProviderSecretErrorKind::InvalidKeyring,
        ));
    }
    let mut ids = HashSet::new();
    let mut key_hashes = HashSet::new();
    let mut keys = Vec::with_capacity(entries.len());
    for entry in entries {
        let (id, encoded) = entry
            .expose_secret()
            .split_once(':')
            .ok_or_else(invalid_keyring)?;
        if !valid_key_id(id) || !ids.insert(id.to_owned()) {
            return Err(invalid_keyring());
        }
        let mut decoded = decode_canonical(encoded).map_err(|_| invalid_keyring())?;
        if decoded.len() != KEY_BYTES {
            decoded.zeroize();
            return Err(invalid_keyring());
        }
        let mut key_bytes = [0_u8; KEY_BYTES];
        key_bytes.copy_from_slice(&decoded);
        decoded.zeroize();
        let key_hash = *blake3::hash(&key_bytes).as_bytes();
        if !key_hashes.insert(key_hash) {
            key_bytes.zeroize();
            return Err(invalid_keyring());
        }
        let key = UnboundKey::new(&AES_256_GCM, &key_bytes)
            .map(LessSafeKey::new)
            .map_err(|_| invalid_keyring());
        key_bytes.zeroize();
        keys.push(KeySlot {
            id: id.to_owned(),
            key: key?,
        });
    }
    Ok(keys)
}

fn valid_key_id(value: &str) -> bool {
    let bytes = value.as_bytes();
    (1..=MAX_KEY_ID_BYTES).contains(&bytes.len())
        && bytes[0].is_ascii_alphanumeric()
        && bytes
            .iter()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
}

fn decode_canonical(value: &str) -> Result<Vec<u8>, ()> {
    let mut decoded = URL_SAFE_NO_PAD.decode(value).map_err(|_| ())?;
    let mut canonical = URL_SAFE_NO_PAD.encode(&decoded);
    let matches = canonical == value;
    canonical.zeroize();
    if matches {
        Ok(decoded)
    } else {
        decoded.zeroize();
        Err(())
    }
}

fn associated_data(provider_id: &str, kind: ProviderKind) -> Vec<u8> {
    let mut aad =
        Vec::with_capacity(AAD_PREFIX.len() + provider_id.len() + 1 + kind.as_storage().len());
    aad.extend_from_slice(AAD_PREFIX);
    aad.extend_from_slice(provider_id.as_bytes());
    aad.push(0);
    aad.extend_from_slice(kind.as_storage().as_bytes());
    aad
}

const fn invalid_keyring() -> ProviderSecretError {
    ProviderSecretError::new(ProviderSecretErrorKind::InvalidKeyring)
}

const fn decrypt_failed() -> ProviderSecretError {
    ProviderSecretError::new(ProviderSecretErrorKind::DecryptFailed)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProviderSecretErrorKind {
    InvalidKeyring,
    InvalidCredential,
    EncryptFailed,
    DecryptFailed,
}

pub struct ProviderSecretError {
    kind: ProviderSecretErrorKind,
}

impl ProviderSecretError {
    const fn new(kind: ProviderSecretErrorKind) -> Self {
        Self { kind }
    }

    #[must_use]
    pub const fn kind(&self) -> ProviderSecretErrorKind {
        self.kind
    }
}

impl fmt::Debug for ProviderSecretError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProviderSecretError")
            .field("kind", &self.kind)
            .finish()
    }
}

impl fmt::Display for ProviderSecretError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self.kind {
            ProviderSecretErrorKind::InvalidKeyring => {
                "AI provider secret key configuration is invalid"
            }
            ProviderSecretErrorKind::InvalidCredential => "AI provider credential is invalid",
            ProviderSecretErrorKind::EncryptFailed => "AI provider credential encryption failed",
            ProviderSecretErrorKind::DecryptFailed => "AI provider credential decryption failed",
        })
    }
}

impl std::error::Error for ProviderSecretError {}
