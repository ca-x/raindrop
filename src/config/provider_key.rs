use std::{
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    path::Path,
};

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use rand_core::{OsRng, RngCore};
use secrecy::SecretString;
use zeroize::Zeroize;

use crate::content::provider::ProviderSecretKeyring;

const LOCAL_KEY_FILE: &str = "provider-secret.key";
const LOCAL_KEY_ID: &str = "local-primary";
const MAX_KEY_FILE_BYTES: u64 = 256;

pub fn load_or_create_local_provider_secret_keys(
    data_dir: &Path,
) -> Result<Vec<SecretString>, LocalProviderKeyError> {
    fs::create_dir_all(data_dir).map_err(LocalProviderKeyError::Io)?;
    let path = data_dir.join(LOCAL_KEY_FILE);
    match read_key(&path) {
        Ok(key) => Ok(vec![key]),
        Err(LocalProviderKeyError::Io(error)) if error.kind() == io::ErrorKind::NotFound => {
            create_key(&path)
        }
        Err(error) => Err(error),
    }
}

pub fn load_existing_local_provider_secret_keys(
    data_dir: &Path,
) -> Result<Option<Vec<SecretString>>, LocalProviderKeyError> {
    let path = data_dir.join(LOCAL_KEY_FILE);
    match read_key(&path) {
        Ok(key) => Ok(Some(vec![key])),
        Err(LocalProviderKeyError::Io(error)) if error.kind() == io::ErrorKind::NotFound => {
            Ok(None)
        }
        Err(error) => Err(error),
    }
}

fn read_key(path: &Path) -> Result<SecretString, LocalProviderKeyError> {
    let path_metadata = fs::symlink_metadata(path).map_err(LocalProviderKeyError::Io)?;
    if !path_metadata.file_type().is_file() || path_metadata.len() > MAX_KEY_FILE_BYTES {
        return Err(LocalProviderKeyError::Invalid);
    }
    let mut file = OpenOptions::new()
        .read(true)
        .open(path)
        .map_err(LocalProviderKeyError::Io)?;
    let file_metadata = file.metadata().map_err(LocalProviderKeyError::Io)?;
    if !file_metadata.file_type().is_file()
        || file_metadata.len() > MAX_KEY_FILE_BYTES
        || !same_file(&path_metadata, &file_metadata)
    {
        return Err(LocalProviderKeyError::Invalid);
    }
    ensure_private_permissions(&file)?;
    let mut value = String::new();
    (&mut file)
        .take(MAX_KEY_FILE_BYTES + 1)
        .read_to_string(&mut value)
        .map_err(LocalProviderKeyError::Io)?;
    if value.len() as u64 > MAX_KEY_FILE_BYTES {
        value.zeroize();
        return Err(LocalProviderKeyError::Invalid);
    }
    let key = parse_key(value.trim());
    value.zeroize();
    key
}

fn create_key(path: &Path) -> Result<Vec<SecretString>, LocalProviderKeyError> {
    let mut bytes = [0_u8; 32];
    OsRng.fill_bytes(&mut bytes);
    let mut encoded = URL_SAFE_NO_PAD.encode(bytes);
    bytes.zeroize();
    let entry = format!("{LOCAL_KEY_ID}:{encoded}");
    encoded.zeroize();

    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = match options.open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
            let mut entry = entry;
            entry.zeroize();
            return read_key(path).map(|key| vec![key]);
        }
        Err(error) => {
            let mut entry = entry;
            entry.zeroize();
            return Err(LocalProviderKeyError::Io(error));
        }
    };
    if let Err(error) = writeln!(file, "{entry}").and_then(|()| file.sync_all()) {
        let _ = fs::remove_file(path);
        let mut entry = entry;
        entry.zeroize();
        return Err(LocalProviderKeyError::Io(error));
    }
    ensure_private_permissions(&file)?;
    sync_parent(path)?;
    let key = parse_key(&entry);
    let mut entry = entry;
    entry.zeroize();
    key.map(|key| vec![key])
}

fn parse_key(value: &str) -> Result<SecretString, LocalProviderKeyError> {
    let key = SecretString::from(value.to_owned());
    ProviderSecretKeyring::validate_entries(std::slice::from_ref(&key))
        .map_err(|_| LocalProviderKeyError::Invalid)?;
    Ok(key)
}

#[cfg(unix)]
fn ensure_private_permissions(file: &File) -> Result<(), LocalProviderKeyError> {
    use std::os::unix::fs::PermissionsExt;

    file.set_permissions(fs::Permissions::from_mode(0o600))
        .map_err(LocalProviderKeyError::Io)
}

#[cfg(not(unix))]
fn ensure_private_permissions(_file: &File) -> Result<(), LocalProviderKeyError> {
    Ok(())
}

#[cfg(unix)]
fn same_file(path_metadata: &fs::Metadata, file_metadata: &fs::Metadata) -> bool {
    use std::os::unix::fs::MetadataExt;

    path_metadata.dev() == file_metadata.dev() && path_metadata.ino() == file_metadata.ino()
}

#[cfg(not(unix))]
fn same_file(_path_metadata: &fs::Metadata, _file_metadata: &fs::Metadata) -> bool {
    true
}

#[cfg(unix)]
fn sync_parent(path: &Path) -> Result<(), LocalProviderKeyError> {
    let parent = path.parent().ok_or(LocalProviderKeyError::Invalid)?;
    File::open(parent)
        .and_then(|directory| directory.sync_all())
        .map_err(LocalProviderKeyError::Io)
}

#[cfg(not(unix))]
fn sync_parent(_path: &Path) -> Result<(), LocalProviderKeyError> {
    Ok(())
}

#[derive(Debug, thiserror::Error)]
pub enum LocalProviderKeyError {
    #[error("local provider secret key storage is unavailable")]
    Io(#[source] io::Error),
    #[error("local provider secret key storage is invalid")]
    Invalid,
}

#[cfg(test)]
mod tests {
    use secrecy::ExposeSecret;

    use super::*;

    #[test]
    fn local_provider_key_is_stable_private_and_redacted() {
        let data = tempfile::tempdir().expect("temporary key directory");
        let first = load_or_create_local_provider_secret_keys(data.path())
            .expect("local provider key should create");
        let second = load_or_create_local_provider_secret_keys(data.path())
            .expect("local provider key should reload");
        let existing = load_existing_local_provider_secret_keys(data.path())
            .expect("existing local provider key should inspect")
            .expect("existing local provider key should load");
        assert_eq!(first[0].expose_secret(), second[0].expose_secret());
        assert_eq!(first[0].expose_secret(), existing[0].expose_secret());
        assert!(first[0].expose_secret().starts_with("local-primary:"));

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(data.path().join(LOCAL_KEY_FILE))
                .expect("local provider key metadata")
                .permissions()
                .mode();
            assert_eq!(mode & 0o077, 0);
        }

        let debug = format!("{:?}", LocalProviderKeyError::Invalid);
        assert!(!debug.contains(first[0].expose_secret()));
    }

    #[test]
    fn missing_existing_local_provider_key_does_not_create_one() {
        let data = tempfile::tempdir().expect("temporary key directory");
        assert!(
            load_existing_local_provider_secret_keys(data.path())
                .expect("missing local provider key should inspect")
                .is_none()
        );
        assert!(!data.path().join(LOCAL_KEY_FILE).exists());
    }

    #[cfg(unix)]
    #[test]
    fn local_provider_key_rejects_symbolic_links() {
        use std::os::unix::fs::symlink;

        let data = tempfile::tempdir().expect("temporary key directory");
        let target = data.path().join("target.key");
        fs::write(
            &target,
            "local-primary:QUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUE\n",
        )
        .expect("target key should write");
        symlink(&target, data.path().join(LOCAL_KEY_FILE)).expect("key symlink should create");

        assert!(matches!(
            load_existing_local_provider_secret_keys(data.path()),
            Err(LocalProviderKeyError::Invalid)
        ));
    }
}
