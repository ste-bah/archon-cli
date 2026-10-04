//! The Windows half of durable publication: a write-through rename and a
//! directory flush (see `store_durable`).

use std::io;
use std::os::windows::ffi::OsStrExt;
use std::os::windows::fs::OpenOptionsExt;
use std::path::Path;

use windows_sys::Win32::Foundation::{
    ERROR_ACCESS_DENIED, ERROR_FILENAME_EXCED_RANGE, ERROR_INVALID_FUNCTION,
    ERROR_INVALID_PARAMETER, ERROR_NOT_SUPPORTED, ERROR_PATH_NOT_FOUND,
};
use windows_sys::Win32::Storage::FileSystem::{
    FILE_FLAG_BACKUP_SEMANTICS, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
};

fn wide(path: &Path) -> Vec<u16> {
    path.as_os_str().encode_wide().chain(Some(0)).collect()
}

/// `MoveFileExW(from, to, MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH)`:
/// replaces `to` and returns only once the move is flushed to disk.
///
/// Where that call refuses a rename std still makes, the rename is
/// `std::fs::rename` (published whole), then the directory is flushed:
/// access denied (std retries with a POSIX-semantics rename, which replaces
/// a read-only target or one another process holds open), and a path past
/// `MAX_PATH` (the paths go to Win32 as given, without the `\\?\` form
/// std builds).
pub(super) fn move_write_through(from: &Path, to: &Path) -> io::Result<()> {
    let (wide_from, wide_to) = (wide(from), wide(to));
    let flags = MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH;
    // SAFETY: both arguments are NUL-terminated UTF-16 buffers that outlive
    // the call; MoveFileExW only reads them.
    if unsafe { MoveFileExW(wide_from.as_ptr(), wide_to.as_ptr(), flags) } != 0 {
        return Ok(());
    }
    let error = io::Error::last_os_error();
    let std_renames = [
        ERROR_ACCESS_DENIED,
        ERROR_FILENAME_EXCED_RANGE,
        ERROR_PATH_NOT_FOUND,
    ];
    if !error
        .raw_os_error()
        .is_some_and(|code| is_one_of(code, &std_renames))
    {
        return Err(error);
    }
    std::fs::rename(from, to)?;
    match to.parent() {
        Some(dir) => flush_dir(dir),
        None => Ok(()),
    }
}

/// Flushes the directory `dir` (`FlushFileBuffers` on a handle opened with
/// `FILE_FLAG_BACKUP_SEMANTICS`, which a directory needs). A volume or
/// access right that will not flush a directory has nothing more to give.
pub(super) fn flush_dir(dir: &Path) -> io::Result<()> {
    let flushed = (std::fs::OpenOptions::new())
        .read(true)
        .write(true)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
        .open(dir)
        .and_then(|handle| handle.sync_all());
    match flushed {
        Err(error)
            if error.raw_os_error().is_some_and(|code| {
                is_one_of(
                    code,
                    &[
                        ERROR_ACCESS_DENIED,
                        ERROR_INVALID_FUNCTION,
                        ERROR_INVALID_PARAMETER,
                        ERROR_NOT_SUPPORTED,
                    ],
                )
            }) =>
        {
            Ok(())
        }
        other => other,
    }
}

fn is_one_of(code: i32, known: &[u32]) -> bool {
    known.iter().any(|known| i32::try_from(*known) == Ok(code))
}
