//! Making a file write or a rename survive a machine crash (Issue-267,
//! round 2).
//!
//! A rename is atomic, but it is durable only after the directory that holds
//! the name is synced, and the renamed file's bytes only after the file is.
//! The run state is saved with a synced temporary file
//! (`store::write_atomic`); a restart's v2 cache invalidation must be at
//! least as durable, and on disk before that state save, or a crash could
//! keep the rewound state and lose the invalidation.
//!
//! Tests read back which paths were synced, in order, from a per-thread
//! journal ([`take_synced`]); production keeps no journal.

use std::fs::File;
use std::path::Path;

use crate::error::{WorkflowError, WorkflowResult};

/// Sync the bytes and the metadata of the file at `path`.
pub(crate) fn sync_file(path: &Path) -> WorkflowResult<()> {
    open_file_to_sync(path)
        .and_then(|file| file.sync_all())
        .map_err(|err| WorkflowError::io(path, err))?;
    note_synced(path);
    Ok(())
}

/// Sync the directory `dir`, so the names created, renamed or removed in it
/// are durable. Directories cannot be opened for syncing on Windows; there
/// this is a no-op.
pub(crate) fn sync_dir(dir: &Path) -> WorkflowResult<()> {
    #[cfg(unix)]
    {
        File::open(dir)
            .and_then(|file| file.sync_all())
            .map_err(|err| WorkflowError::io(dir, err))?;
    }
    note_synced(dir);
    Ok(())
}

#[cfg(test)]
thread_local! {
    static SYNCED: std::cell::RefCell<Vec<std::path::PathBuf>> = const {
        std::cell::RefCell::new(Vec::new())
    };
}

/// Record that `path` was synced (tests only).
pub(crate) fn note_synced(path: &Path) {
    #[cfg(test)]
    SYNCED.with(|synced| synced.borrow_mut().push(path.to_path_buf()));
    #[cfg(not(test))]
    let _ = path;
}

/// Every path this thread synced since the last call, in order.
#[cfg(test)]
pub(crate) fn take_synced() -> Vec<std::path::PathBuf> {
    SYNCED.with(|synced| std::mem::take(&mut *synced.borrow_mut()))
}

// Kept separate so the access required by Windows can be exercised on any OS.
fn open_file_to_sync(path: &Path) -> std::io::Result<File> {
    std::fs::OpenOptions::new().write(true).open(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn durable_file_handle_has_write_access_without_truncating() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("record");
        std::fs::write(&path, b"durable").unwrap();
        let mut file = open_file_to_sync(&path).unwrap();
        file.write_all(b"d")
            .expect("sync handle must have write access");
        file.sync_all().unwrap();
        sync_file(&path).unwrap();
        assert_eq!(std::fs::read(path).unwrap(), b"durable");
    }
}
