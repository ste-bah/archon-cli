//! A task set's publish lock file (`<pin>.publish.lock`) and the process's
//! record of which thread holds it (Issue 294).
//!
//! The host's journaled publishes, its recovery and every consistent read of
//! the frozen chain hold this one lock, as do this crate's check-source
//! repins and reads. Two opens of the file lock each other even inside one
//! process, so a thread that already holds it and asks again would wait on
//! itself forever: a reader nested in a holder on the same thread reads
//! under the holder's lock instead, and a second acquisition is refused.

use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::thread::ThreadId;

/// Every publish lock this process holds: its canonical path and the thread
/// that took it.
static HELD: Mutex<Vec<(PathBuf, ThreadId)>> = Mutex::new(Vec::new());

fn held() -> MutexGuard<'static, Vec<(PathBuf, ThreadId)>> {
    HELD.lock().unwrap_or_else(PoisonError::into_inner)
}

fn key(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

/// The publish lock file beside the acceptance pin at `pin_path`.
pub fn lock_path(pin_path: &Path) -> PathBuf {
    pin_path.with_extension("publish.lock")
}

/// Whether this thread holds the publish lock at `path`.
pub fn held_here(path: &Path) -> bool {
    if !path.exists() {
        return false;
    }
    let (key, here) = (key(path), std::thread::current().id());
    held()
        .iter()
        .any(|(held, owner)| *held == key && *owner == here)
}

/// A held publish lock. Dropping it unlocks the file at once, even while a
/// forked child still shares it, then forgets the holder.
pub struct PublishLockFile {
    file: std::fs::File,
    key: PathBuf,
    owner: ThreadId,
}

impl PublishLockFile {
    /// Block until the lock at `path` is free: a holder keeps it only for one
    /// publish transaction or one read. The parent directory must exist.
    pub fn acquire(path: &Path) -> Result<Self, String> {
        if held_here(path) {
            return Err(format!(
                "this thread already holds the publish lock {}; a nested acquisition would wait on itself",
                path.display()
            ));
        }
        let file = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(path)
            .map_err(|error| format!("opening publish lock {}: {error}", path.display()))?;
        file.lock()
            .map_err(|error| format!("locking publish lock {}: {error}", path.display()))?;
        let lock = Self {
            file,
            key: key(path),
            owner: std::thread::current().id(),
        };
        held().push((lock.key.clone(), lock.owner));
        Ok(lock)
    }

    /// Hold the publish lock of the set pinned at `pin_path` for one read or
    /// one repin: `None` when this thread already holds it (the work runs
    /// under its holder) or when no set of this project was ever frozen (the
    /// pin's directory does not exist, so nothing is published to read).
    pub fn hold(pin_path: &Path) -> Result<Option<Self>, String> {
        let path = lock_path(pin_path);
        if held_here(&path) || !path.parent().is_some_and(Path::is_dir) {
            return Ok(None);
        }
        Self::acquire(&path).map(Some)
    }
}

impl Drop for PublishLockFile {
    fn drop(&mut self) {
        let mut held = held();
        if let Some(index) = held
            .iter()
            .position(|(key, owner)| *key == self.key && *owner == self.owner)
        {
            held.swap_remove(index);
        }
        drop(held);
        if let Err(error) = self.file.unlock() {
            tracing::warn!(%error, "publish lock could not be unlocked; it is released when closed");
        }
    }
}
