use secrecy::SecretString;

pub(crate) fn secret(value: String) -> SecretString {
    SecretString::from(value)
}
