use std::fs;
#[cfg(unix)]
use std::fs::OpenOptions;
#[cfg(unix)]
use std::io::{Read, Write};
use std::path::Path;

/// Load the session's opaque approval secret or create it atomically.
///
/// The secret is never serialized into plans or the database. Callers must keep
/// the returned bytes process-private and pass them directly to PlanStore.
pub fn load_or_create_approval_secret(path: &Path) -> Result<[u8; 32], std::io::Error> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    match read_existing_secret(path) {
        Ok(secret) => Ok(secret),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => create_secret(path),
        Err(error) => Err(error),
    }
}

#[cfg(unix)]
fn read_existing_secret(path: &Path) -> Result<[u8; 32], std::io::Error> {
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt};

    let mut file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)?;
    let metadata = file.metadata()?;
    if !metadata.file_type().is_file() || metadata.mode() & 0o077 != 0 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "plan approval secret must be a regular owner-only file",
        ));
    }
    read_secret_file(&mut file)
}

#[cfg(unix)]
fn create_secret(path: &Path) -> Result<[u8; 32], std::io::Error> {
    use std::os::unix::fs::OpenOptionsExt;

    let secret = random_secret()?;
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)?;
    file.write_all(&secret)?;
    file.sync_all()?;
    drop(file);
    read_existing_secret(path)
}

#[cfg(windows)]
fn read_existing_secret(path: &Path) -> Result<[u8; 32], std::io::Error> {
    windows_secret::read_owner_only_file(path)
}

#[cfg(windows)]
fn create_secret(path: &Path) -> Result<[u8; 32], std::io::Error> {
    windows_secret::create_owner_only_file(path, &random_secret()?)
}

#[cfg(unix)]
fn read_secret_file(file: &mut std::fs::File) -> Result<[u8; 32], std::io::Error> {
    let mut bytes = [0_u8; 32];
    file.read_exact(&mut bytes)?;
    let mut trailing = [0_u8; 1];
    if file.read(&mut trailing)? != 0 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "plan approval secret has an invalid length",
        ));
    }
    Ok(bytes)
}

fn random_secret() -> Result<[u8; 32], std::io::Error> {
    let mut secret = [0_u8; 32];
    getrandom::fill(&mut secret).map_err(std::io::Error::other)?;
    Ok(secret)
}

#[cfg(windows)]
fn owner_only_sddl(user_sid: &str) -> String {
    format!("O:{user_sid}G:{user_sid}D:P(A;;FA;;;{user_sid})(A;;FA;;;SY)")
}

#[cfg(windows)]
#[path = "plan_authority_secret_windows.rs"]
mod windows_secret;

#[cfg(all(test, windows))]
#[path = "plan_authority_secret_windows_tests.rs"]
mod windows_tests;

#[cfg(test)]
mod tests {
    #[cfg(unix)]
    use super::{fs, load_or_create_approval_secret};

    #[cfg(unix)]
    #[test]
    fn creates_an_owner_only_restart_stable_secret() {
        use std::os::unix::fs::PermissionsExt;
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("approval.secret");
        let first = load_or_create_approval_secret(&path).unwrap();
        let second = load_or_create_approval_secret(&path).unwrap();
        assert_eq!(first, second);
        assert_eq!(
            fs::metadata(path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }

    #[cfg(unix)]
    #[test]
    fn rejects_an_existing_group_readable_secret() {
        use std::os::unix::fs::PermissionsExt;
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("approval.secret");
        fs::write(&path, [9_u8; 32]).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).unwrap();
        assert_eq!(
            load_or_create_approval_secret(&path).unwrap_err().kind(),
            std::io::ErrorKind::PermissionDenied
        );
    }
}
