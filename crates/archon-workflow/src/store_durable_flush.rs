//! The decisions of the Windows directory flush and fallback rename, apart
//! from the Win32 calls so every host tests them (Issue 262, round 9).

use std::io;
use std::path::Path;

/// Win32 error codes (`winerror.h`); the Windows module asserts they match
/// `windows-sys`.
pub(super) const ERROR_INVALID_FUNCTION: i32 = 1;
pub(super) const ERROR_ACCESS_DENIED: i32 = 5;
pub(super) const ERROR_NOT_SUPPORTED: i32 = 50;
pub(super) const ERROR_INVALID_PARAMETER: i32 = 87;

/// Flushes a directory: `open` it, then `flush` the handle. Only a flush
/// the volume refuses as an operation it does not have (an invalid
/// function or parameter, not supported) is no error: like `EINVAL` on
/// Unix, the volume has no directory flush to give. Every other failure --
/// the directory will not open (access denied included), or the flush is
/// denied -- is an error naming the directory: a flush not obtained is
/// never reported as obtained.
pub(super) fn flush_dir_with<H>(
    dir: &Path,
    open: impl FnOnce() -> io::Result<H>,
    flush: impl FnOnce(H) -> io::Result<()>,
) -> io::Result<()> {
    let handle = open().map_err(|error| {
        context(
            error,
            format!("cannot open directory {} to flush it", dir.display()),
        )
    })?;
    match flush(handle) {
        Err(error)
            if error
                .raw_os_error()
                .is_some_and(volume_has_no_directory_flush) =>
        {
            Ok(())
        }
        Err(error) => Err(context(
            error,
            format!("cannot flush directory {}", dir.display()),
        )),
        Ok(()) => Ok(()),
    }
}

fn volume_has_no_directory_flush(code: i32) -> bool {
    matches!(
        code,
        ERROR_INVALID_FUNCTION | ERROR_INVALID_PARAMETER | ERROR_NOT_SUPPORTED
    )
}

/// The rename the write-through call refused (`refused`), made by `rename`
/// without write-through and then made durable by `flush`ing its
/// directory. A flush that fails is an error saying the rename happened
/// but may not survive a system crash; it is never reported as durable.
pub(super) fn fallback_rename_with(
    from: &Path,
    to: &Path,
    refused: &io::Error,
    rename: impl FnOnce() -> io::Result<()>,
    flush: impl FnOnce() -> io::Result<()>,
) -> io::Result<()> {
    rename()?;
    flush().map_err(|error| {
        context(
            error,
            format!(
                "renamed {} to {} without write-through (MoveFileExW refused it: {refused}), and the directory flush that makes it durable failed, so the rename may not survive a system crash",
                from.display(),
                to.display()
            ),
        )
    })
}

fn context(error: io::Error, what: String) -> io::Error {
    io::Error::new(error.kind(), format!("{what}: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn denied() -> io::Error {
        io::Error::from_raw_os_error(ERROR_ACCESS_DENIED)
    }

    /// A directory that will not open (access denied) was never flushed:
    /// that is an error naming it, never a durable success.
    #[test]
    fn a_directory_that_will_not_open_is_an_error_never_a_flush() {
        let flushed = flush_dir_with(Path::new("d"), || Err::<(), _>(denied()), |()| Ok(()));
        let error = flushed.expect_err("an unopened directory was never flushed");
        assert!(error.to_string().contains("d"), "{error}");
    }

    /// A flush the access rights deny is an error, never a durable success.
    #[test]
    fn a_denied_flush_is_an_error() {
        let flushed = flush_dir_with(Path::new("d"), || Ok(()), |()| Err(denied()));
        assert!(flushed.is_err(), "{flushed:?}");
    }

    /// A volume with no directory flush has nothing more to give, as on
    /// Unix (`EINVAL`): that is no error.
    #[test]
    fn a_volume_without_a_directory_flush_is_no_error() {
        for code in [
            ERROR_INVALID_FUNCTION,
            ERROR_INVALID_PARAMETER,
            ERROR_NOT_SUPPORTED,
        ] {
            let flushed = flush_dir_with(
                Path::new("d"),
                || Ok(()),
                |()| Err(io::Error::from_raw_os_error(code)),
            );
            assert!(flushed.is_ok(), "{code}: {flushed:?}");
        }
    }

    /// The fallback rename, made without write-through, is durable only
    /// once its directory is flushed: a flush that fails is an error that
    /// says the rename may not survive a crash.
    #[test]
    fn a_fallback_rename_whose_flush_fails_reports_it_is_not_durable() {
        let renamed = fallback_rename_with(
            Path::new("a"),
            Path::new("b"),
            &denied(),
            || Ok(()),
            || flush_dir_with(Path::new("."), || Ok(()), |()| Err(denied())),
        );
        let error = renamed.expect_err("a rename never flushed is not durable");
        assert!(error.to_string().contains("may not survive"), "{error}");
    }
}
