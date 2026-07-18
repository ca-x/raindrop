use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use raindrop::content::provider::{ProviderKind, ProviderSecretErrorKind, ProviderSecretKeyring};
use secrecy::{ExposeSecret, SecretString};

const PROVIDER_ID: &str = "00000000-0000-4000-8000-000000000901";
const OTHER_PROVIDER_ID: &str = "00000000-0000-4000-8000-000000000902";
const CREDENTIAL: &str = "credential-sentinel-ai-provider";

#[test]
fn provider_secret_round_trip_uses_versioned_random_nonce_envelopes() {
    let keyring = keyring(&[("primary", 1)]);
    let credential = SecretString::from(CREDENTIAL);

    let first = keyring
        .encrypt(PROVIDER_ID, ProviderKind::OpenAiResponses, &credential)
        .expect("credential should encrypt");
    let second = keyring
        .encrypt(PROVIDER_ID, ProviderKind::OpenAiResponses, &credential)
        .expect("credential should encrypt with a new nonce");

    assert_ne!(first, second);
    for envelope in [&first, &second] {
        let parts = envelope.split('.').collect::<Vec<_>>();
        assert_eq!(parts.len(), 4);
        assert_eq!(parts[0], "rdsec1");
        assert_eq!(parts[1], "primary");
        assert_eq!(
            URL_SAFE_NO_PAD
                .decode(parts[2])
                .expect("nonce should be canonical base64")
                .len(),
            12
        );
        assert!(
            URL_SAFE_NO_PAD
                .decode(parts[3])
                .expect("ciphertext should be canonical base64")
                .len()
                > CREDENTIAL.len()
        );
        assert!(!envelope.contains(CREDENTIAL));
        let decrypted = keyring
            .decrypt(PROVIDER_ID, ProviderKind::OpenAiResponses, envelope)
            .expect("credential should decrypt");
        assert_eq!(decrypted.expose_secret(), CREDENTIAL);
    }
}

#[test]
fn previous_key_decrypts_after_active_key_rotation() {
    let previous = keyring(&[("previous", 7)]);
    let envelope = previous
        .encrypt(
            PROVIDER_ID,
            ProviderKind::AnthropicMessages,
            &SecretString::from(CREDENTIAL),
        )
        .expect("previous key should encrypt");
    let rotated = keyring(&[("primary", 8), ("previous", 7)]);

    assert_eq!(rotated.active_key_id(), "primary");
    assert_eq!(
        rotated
            .decrypt(PROVIDER_ID, ProviderKind::AnthropicMessages, &envelope)
            .expect("retained previous key should decrypt")
            .expose_secret(),
        CREDENTIAL
    );
    let new_envelope = rotated
        .encrypt(
            PROVIDER_ID,
            ProviderKind::AnthropicMessages,
            &SecretString::from(CREDENTIAL),
        )
        .expect("rotated active key should encrypt");
    assert!(new_envelope.starts_with("rdsec1.primary."));
}

#[test]
fn provider_id_kind_and_envelope_bytes_are_authenticated() {
    let keyring = keyring(&[("primary", 11)]);
    let envelope = keyring
        .encrypt(
            PROVIDER_ID,
            ProviderKind::GoogleGemini,
            &SecretString::from(CREDENTIAL),
        )
        .expect("credential should encrypt");

    for result in [
        keyring.decrypt(OTHER_PROVIDER_ID, ProviderKind::GoogleGemini, &envelope),
        keyring.decrypt(PROVIDER_ID, ProviderKind::OpenAiChatCompletions, &envelope),
        keyring.decrypt(
            PROVIDER_ID,
            ProviderKind::GoogleGemini,
            &tamper_segment(&envelope, 2),
        ),
        keyring.decrypt(
            PROVIDER_ID,
            ProviderKind::GoogleGemini,
            &tamper_segment(&envelope, 3),
        ),
    ] {
        assert_eq!(
            result
                .expect_err("authentication mismatch should fail")
                .kind(),
            ProviderSecretErrorKind::DecryptFailed
        );
    }
}

#[test]
fn malformed_unknown_and_noncanonical_envelopes_fail_closed() {
    let keyring = keyring(&[("primary", 12)]);
    let valid = keyring
        .encrypt(
            PROVIDER_ID,
            ProviderKind::OpenAiResponses,
            &SecretString::from(CREDENTIAL),
        )
        .expect("credential should encrypt");
    let unknown = valid.replacen(".primary.", ".missing.", 1);
    let padded = format!("{valid}=");

    for envelope in [
        "",
        "rdsec2.primary.nonce.ciphertext",
        "rdsec1.primary.only-three",
        "rdsec1.primary.%%%.%%%",
        unknown.as_str(),
        padded.as_str(),
    ] {
        assert_eq!(
            keyring
                .decrypt(PROVIDER_ID, ProviderKind::OpenAiResponses, envelope)
                .expect_err("malformed envelope should fail")
                .kind(),
            ProviderSecretErrorKind::DecryptFailed
        );
    }
}

#[test]
fn keyring_and_credential_bounds_are_exact() {
    let empty = Vec::<SecretString>::new();
    assert_eq!(
        ProviderSecretKeyring::from_entries(&empty)
            .expect_err("empty keyring should fail")
            .kind(),
        ProviderSecretErrorKind::InvalidKeyring
    );

    for entries in [
        vec![entry("primary", 1), entry("primary", 2)],
        vec![entry("primary", 3), entry("previous", 3)],
        vec![SecretString::from(format!(
            "primary:{}=",
            URL_SAFE_NO_PAD.encode([4_u8; 32])
        ))],
        vec![SecretString::from(format!(
            "-invalid:{}",
            URL_SAFE_NO_PAD.encode([5_u8; 32])
        ))],
        vec![SecretString::from(format!(
            "primary:{}",
            URL_SAFE_NO_PAD.encode([6_u8; 31])
        ))],
    ] {
        assert_eq!(
            ProviderSecretKeyring::from_entries(&entries)
                .expect_err("invalid keyring should fail")
                .kind(),
            ProviderSecretErrorKind::InvalidKeyring
        );
    }

    let keyring = keyring(&[("primary", 9)]);
    for invalid in [String::new(), "x".repeat(8_193)] {
        assert_eq!(
            keyring
                .encrypt(
                    PROVIDER_ID,
                    ProviderKind::OpenAiResponses,
                    &SecretString::from(invalid),
                )
                .expect_err("credential bound should fail")
                .kind(),
            ProviderSecretErrorKind::InvalidCredential
        );
    }
    let exact = "x".repeat(8_192);
    let envelope = keyring
        .encrypt(
            PROVIDER_ID,
            ProviderKind::OpenAiResponses,
            &SecretString::from(exact.clone()),
        )
        .expect("maximum credential should encrypt");
    assert_eq!(
        keyring
            .decrypt(PROVIDER_ID, ProviderKind::OpenAiResponses, &envelope)
            .expect("maximum credential should decrypt")
            .expose_secret(),
        exact
    );
}

#[test]
fn secret_types_and_errors_are_redacted() {
    let key_entry = entry("key-sentinel-id", 21);
    let keyring = ProviderSecretKeyring::from_entries(std::slice::from_ref(&key_entry))
        .expect("valid keyring should construct");
    let envelope = keyring
        .encrypt(
            PROVIDER_ID,
            ProviderKind::OpenAiResponses,
            &SecretString::from(CREDENTIAL),
        )
        .expect("credential should encrypt");
    let error = keyring
        .decrypt(OTHER_PROVIDER_ID, ProviderKind::OpenAiResponses, &envelope)
        .expect_err("wrong AAD should fail");
    let formatted = format!("{keyring:?} {error:?} {error}");

    assert!(formatted.contains("key-sentinel-id"));
    assert!(!formatted.contains(key_entry.expose_secret()));
    assert!(!formatted.contains(CREDENTIAL));
    assert!(!formatted.contains(&envelope));
}

fn keyring(keys: &[(&str, u8)]) -> ProviderSecretKeyring {
    let entries = keys
        .iter()
        .map(|(id, byte)| entry(id, *byte))
        .collect::<Vec<_>>();
    ProviderSecretKeyring::from_entries(&entries).expect("test keyring should construct")
}

fn entry(id: &str, byte: u8) -> SecretString {
    SecretString::from(format!("{id}:{}", URL_SAFE_NO_PAD.encode([byte; 32])))
}

fn tamper_segment(envelope: &str, index: usize) -> String {
    let mut parts = envelope.split('.').map(str::to_owned).collect::<Vec<_>>();
    let bytes = URL_SAFE_NO_PAD
        .decode(&parts[index])
        .expect("test segment should decode");
    let mut bytes = bytes;
    bytes[0] ^= 1;
    parts[index] = URL_SAFE_NO_PAD.encode(bytes);
    parts.join(".")
}
