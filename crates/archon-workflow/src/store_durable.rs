//! Durable file publication: a write lands whole or not at all, and the
//! rename that publishes it survives a system crash.
//!
//! Unix: the staged file is synced, renamed over the target, and the caller
//! syncs the directory ([`sync_dir`]) so the new entry is on disk.
//!
//! Windows: `std::fs::rename` is `MoveFileExW(MOVEFILE_REPLACE_EXISTING)`
//! with no `MOVEFILE_WRITE_THROUGH`, so it may return before the move is on
//! disk, and `File::open` cannot open a directory to sync it. Here the
//! rename is `MoveFileExW(MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH)`,
//! which returns only once the move is flushed, and [`sync_dir`] opens the
//! directory with `FILE_FLAG_BACKUP_SEMANTICS` and flushes it
//! (`FlushFileBuffers`). A volume with no directory flush (an invalid
//! function or parameter, not supported) has nothing more to give, and that
//! is no error; a directory that will not open, or a denied flush, is an
//! error (round 9: durability not obtained is never claimed). Where the
//! write-through call refuses a rename std still makes (access denied,
//! which std retries with a POSIX-semantics rename; a path past
//! `MAX_PATH`), the rename is std's and the directory is flushed after it;
//! a flush that fails there is an error saying the rename may not be
//! durable. This is built from the Win32 contract, not proven by a crash
//! test on Windows.

use std::fs::{self, File};
use std::io::Write;
use std::path::Path;

use crate::error::{WorkflowError, WorkflowResult};

/// Writes `bytes` to `target` whole or not at all: staged in `tmp`, synced,
/// then renamed over it ([`rename_durable`]). The new directory entry is
/// not synced: a caller that needs it to survive a system crash also calls
/// [`sync_dir`].
pub(crate) fn write_atomic(tmp: &Path, target: &Path, bytes: &[u8]) -> WorkflowResult<()> {
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent).map_err(|e| WorkflowError::io(parent, e))?;
    }
    {
        let mut file = File::create(tmp).map_err(|e| WorkflowError::io(tmp, e))?;
        file.write_all(bytes)
            .map_err(|e| WorkflowError::io(tmp, e))?;
        file.sync_all().map_err(|e| WorkflowError::io(tmp, e))?;
    }
    crate::durable_io::note_synced(tmp);
    rename_durable(tmp, target)
}

/// Renames `from` over `to` (replacing it). On Windows the call returns
/// only once the move is on disk; on Unix the caller syncs the directory.
pub(crate) fn rename_durable(from: &Path, to: &Path) -> WorkflowResult<()> {
    #[cfg(windows)]
    let renamed = windows::move_write_through(from, to);
    #[cfg(not(windows))]
    let renamed = fs::rename(from, to);
    renamed.map_err(|e| WorkflowError::io(to, e))
}

/// Syncs the directory `dir`, so the entries created, renamed or removed in
/// it survive a system crash. A file system that cannot sync a directory
/// (`EINVAL` or `ENOTSUP` on Unix; an invalid function or parameter, or not
/// supported, on Windows) has nothing more to give, and that is no error.
/// A directory that will not open, or a denied flush, is an error.
pub(crate) fn sync_dir(dir: &Path) -> WorkflowResult<()> {
    #[cfg(unix)]
    let synced = File::open(dir).and_then(|handle| match handle.sync_all() {
        Err(e)
            if matches!(
                e.kind(),
                std::io::ErrorKind::InvalidInput | std::io::ErrorKind::Unsupported
            ) =>
        {
            Ok(())
        }
        other => other,
    });
    #[cfg(windows)]
    let synced = windows::flush_dir(dir);
    #[cfg(not(any(unix, windows)))]
    let synced: std::io::Result<()> = {
        let _ = dir;
        Ok(())
    };
    synced.map_err(|e| WorkflowError::io(dir, e))
}

#[cfg(windows)]
#[path = "store_durable_windows.rs"]
mod windows;

#[cfg(any(windows, test))]
#[path = "store_durable_flush.rs"]
mod flush;

#[cfg(test)]
#[path = "store_durable_tests.rs"]
mod tests;
