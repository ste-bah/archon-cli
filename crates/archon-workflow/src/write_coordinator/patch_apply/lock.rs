use std::path::{Path, PathBuf};
use std::time::Duration;

use super::ApplyError;

const MAX_RETRIES: u32 = 600;
const BASE_DELAY_MS: u64 = 100;

/// Lock path: `<canonical_root>/.archon/workflows/write-locks/<blake3>.lock`.
pub fn lock_path_for(canonical_root: &Path) -> PathBuf {
    let hex = blake3::hash(canonical_root.to_string_lossy().as_bytes())
        .to_hex()
        .to_string();
    canonical_root
        .join(".archon")
        .join("workflows")
        .join("write-locks")
        .join(format!("{hex}.lock"))
}

pub(super) fn with_repo_lock_tuned<R, F>(
    canonical_root: &Path,
    max_retries: u32,
    base_delay_ms: u64,
    f: F,
) -> Result<R, ApplyError>
where
    F: FnOnce() -> Result<R, ApplyError>,
{
    let lock_path = lock_path_for(canonical_root);
    if let Some(parent) = lock_path.parent() {
        std::fs::create_dir_all(parent).map_err(ApplyError::LockIo)?;
    }
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&lock_path)
        .map_err(ApplyError::LockIo)?;
    let mut lock = fd_lock::RwLock::new(file);
    for attempt in 0..max_retries {
        match lock.try_write() {
            Ok(_guard) => return f(),
            Err(_) => std::thread::sleep(Duration::from_millis(
                base_delay_ms + (u64::from(attempt) * 7 % 30),
            )),
        }
    }
    Err(ApplyError::LockTimeout {
        lock_path,
        waited: Duration::from_millis(u64::from(max_retries) * base_delay_ms),
    })
}

pub(super) fn with_repo_lock_default<R, F>(canonical_root: &Path, f: F) -> Result<R, ApplyError>
where
    F: FnOnce() -> Result<R, ApplyError>,
{
    with_repo_lock_tuned(canonical_root, MAX_RETRIES, BASE_DELAY_MS, f)
}
