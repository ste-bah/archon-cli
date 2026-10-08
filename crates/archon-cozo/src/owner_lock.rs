//! An exclusive, non-blocking ownership lock on a file.
//!
//! A long job (an ingest of one document, say) holds the lock for as long as
//! it runs. Another process or thread asks with [`OwnerLockFile::try_own`]
//! and learns at once whether a live owner exists: `None` means one does.
//! The operating system releases the lock when the owner ends, however it
//! ends, so a job that paused or crashed leaves a free lock behind and the
//! next run can take over its work.
use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

/// The lock file. It lives as long as any [`OwnerLock`] taken from it.
pub struct OwnerLockFile {
    path: PathBuf,
    lock: fd_lock::RwLock<File>,
}

/// Proof of ownership. Dropping it releases the lock and keeps the file.
pub struct OwnerLock<'a> {
    path: &'a Path,
    _guard: fd_lock::RwLockWriteGuard<'a, File>,
}

impl OwnerLockFile {
    /// Open (or create) the lock file at `path`, and its directory.
    pub fn open(path: impl Into<PathBuf>) -> Result<Self> {
        let path = path.into();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).with_context(|| {
                format!("create the directory of owner lock {}", path.display())
            })?;
        }
        let file = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(&path)
            .with_context(|| format!("open owner lock {}", path.display()))?;
        Ok(Self {
            path,
            lock: fd_lock::RwLock::new(file),
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Take ownership now, or `None` when a live owner holds the lock. Any
    /// other lock error is returned with the lock path, never read as "held".
    pub fn try_own(&mut self) -> Result<Option<OwnerLock<'_>>> {
        match self.lock.try_write() {
            Ok(guard) => Ok(Some(OwnerLock {
                path: &self.path,
                _guard: guard,
            })),
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => Ok(None),
            Err(error) => Err(anyhow::Error::new(error)
                .context(format!("take owner lock {}", self.path.display()))),
        }
    }
}

impl OwnerLock<'_> {
    /// The owned job is complete: remove the lock file while still owning it,
    /// so completed jobs leave no files behind. A waiter that checks the job's
    /// recorded state first never opens a file that is being removed.
    pub fn complete(self) {
        if let Err(error) = std::fs::remove_file(self.path)
            && error.kind() != std::io::ErrorKind::NotFound
        {
            tracing::warn!(path = %self.path.display(), %error, "could not remove a completed owner lock");
        }
    }
}

impl std::fmt::Debug for OwnerLock<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OwnerLock")
            .field("path", &self.path)
            .finish()
    }
}
