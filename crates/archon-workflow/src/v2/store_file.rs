//! Opening the v2 store's own files safely (Issue-292).
//!
//! The store's directories sit inside the run's working tree, where the
//! agents a run dispatches can write (see `result_store_scan.rs`). An entry
//! there can be something no record ever is: a FIFO (opening one waits for
//! a writer, so a resume that read one waited forever), a socket or device,
//! a directory, a link out of its directory, or a file too large to be a
//! record. Every reader of `branches/`, its `superseded/` archives,
//! `results/` and its archives takes bytes only through [`read_store_file`]:
//! a regular file of at most [`MAX_STORE_FILE_BYTES`], or a link that
//! resolves to one in the link's own directory or directly in the store
//! directory its reader names. A call's `superseded/` archive is read with
//! the call's directory as its store directory, where restart and the
//! archive move its records; a link deeper (into `revoked/`) is refused.
//! Every directory from the store directory down to the entry must be a
//! real directory, never a link.
//!
//! The kind is checked before the open, so a special file is never opened,
//! and again on the opened handle, which is opened without blocking and
//! without following a final link: an entry swapped after the check is
//! refused, never read. A link loop (too many levels of links) is refused
//! like any other entry that is no record: it is never fatal. A refused
//! entry is never a record and never counts as one. Each refusal is
//! reported as a warning that names the entry and the reason, once per
//! entry in a process ([`report_skipped_store_entry`]);
//! [`classify_store_entry`] gives the same verdict unread.

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

/// The largest file a store reader opens. A record the store writes is far
/// smaller; a larger file is not one of its records.
pub const MAX_STORE_FILE_BYTES: u64 = 64 << 20;

/// Why a store entry is never opened as a record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StoreRefusal {
    /// A FIFO, socket or device, or a link that resolves to one.
    Special,
    /// A directory, or a link that resolves to one.
    Directory,
    /// A link that resolves outside its store directory.
    OutsideLink,
    /// A directory between the store directory and the entry is a link.
    LinkedDirectory,
    /// Larger than [`MAX_STORE_FILE_BYTES`].
    Oversized,
    /// The entry changed between its check and its open (it became a link).
    Changed,
    /// A link loop, or a chain of links too long to resolve (`ELOOP`).
    LinkLoop,
}

impl std::fmt::Display for StoreRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Special => "not a regular file (a FIFO, socket or device)",
            Self::Directory => "a directory, not a file",
            Self::OutsideLink => "a link that resolves outside its store directory",
            Self::LinkedDirectory => "inside a directory that is a link",
            Self::Oversized => "larger than the store's file bound",
            Self::Changed => "changed between its check and its open",
            Self::LinkLoop => "a link loop (too many levels of links)",
        })
    }
}

#[derive(Debug)]
struct Refused(StoreRefusal);

impl std::fmt::Display for Refused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "store entry refused unread: {}", self.0)
    }
}

impl std::error::Error for Refused {}

/// The refusal `error` carries, when [`read_store_file`] refused the entry
/// rather than failed to read it.
pub fn store_file_refusal(error: &std::io::Error) -> Option<StoreRefusal> {
    error
        .get_ref()?
        .downcast_ref::<Refused>()
        .map(|refused| refused.0)
}

/// Report that `path` was skipped unread, and why. Every reader that skips
/// a store entry says so here; none skips one silently. A stuck entry is
/// met again on every read and every fan-out, so each path is reported
/// once in this process: the first time any reader skips it.
pub fn report_skipped_store_entry(path: &Path, reason: &dyn std::fmt::Display) {
    static REPORTED: std::sync::OnceLock<std::sync::Mutex<std::collections::HashSet<PathBuf>>> =
        std::sync::OnceLock::new();
    let first = REPORTED
        .get_or_init(Default::default)
        .lock()
        .map(|mut seen| seen.insert(path.to_path_buf()))
        .unwrap_or(true);
    if first {
        tracing::warn!(
            path = %path.display(),
            %reason,
            "store entry skipped unread; it is never counted as a record"
        );
    }
}

/// Whether `error` says a link loop, or a chain of links too long to
/// resolve. Only Unix names it (`ELOOP`).
fn is_link_loop(error: &std::io::Error) -> bool {
    #[cfg(unix)]
    {
        error.raw_os_error() == Some(libc::ELOOP)
    }
    #[cfg(not(unix))]
    {
        let _ = error;
        false
    }
}

fn refuse(path: &Path, refusal: StoreRefusal) -> std::io::Error {
    report_skipped_store_entry(path, &refusal);
    std::io::Error::new(std::io::ErrorKind::InvalidData, Refused(refusal))
}

fn kind_refusal(meta: &fs::Metadata) -> Option<StoreRefusal> {
    if meta.is_dir() {
        Some(StoreRefusal::Directory)
    } else if !meta.is_file() {
        Some(StoreRefusal::Special)
    } else if meta.len() > MAX_STORE_FILE_BYTES {
        Some(StoreRefusal::Oversized)
    } else {
        None
    }
}

/// The directory of `path`: its store directory when its reader names no
/// other.
fn own_dir(path: &Path) -> &Path {
    path.parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(Path::new("."))
}

/// Whether a directory from `root` down to `dir` is a link. `root` must hold
/// `dir`; when it does not, `dir` alone is checked.
fn linked_dir_between(root: &Path, dir: &Path) -> std::io::Result<bool> {
    let root = if dir.starts_with(root) { root } else { dir };
    let mut at = dir;
    loop {
        if fs::symlink_metadata(at)?.file_type().is_symlink() {
            return Ok(true);
        }
        match at.parent() {
            Some(parent) if at != root => at = parent,
            _ => return Ok(false),
        }
    }
}

/// What a reader would open for `path`, judged unread: `Ok(path)` for a
/// regular file, `Ok(target)` for a link to one in its own directory or
/// directly in `root`, `Err(refusal)` otherwise (a link loop too). A link
/// whose target is missing is an I/O error.
fn resolve(path: &Path, root: &Path) -> std::io::Result<Result<PathBuf, StoreRefusal>> {
    let own = own_dir(path);
    if linked_dir_between(root, own)? {
        return Ok(Err(StoreRefusal::LinkedDirectory));
    }
    let meta = fs::symlink_metadata(path)?;
    if !meta.file_type().is_symlink() {
        return Ok(kind_refusal(&meta).map_or_else(|| Ok(path.to_path_buf()), Err));
    }
    let target = match fs::canonicalize(path) {
        Ok(target) => target,
        Err(error) if is_link_loop(&error) => return Ok(Err(StoreRefusal::LinkLoop)),
        Err(error) => return Err(error),
    };
    let target_meta = fs::metadata(&target)?;
    // What it is comes first: a link to a FIFO is a special entry wherever
    // the FIFO lives.
    if let Some(refusal @ (StoreRefusal::Special | StoreRefusal::Directory)) =
        kind_refusal(&target_meta)
    {
        return Ok(Err(refusal));
    }
    let into = target.parent();
    let root = if own.starts_with(root) { root } else { own };
    if into != Some(fs::canonicalize(own)?.as_path())
        && into != Some(fs::canonicalize(root)?.as_path())
    {
        return Ok(Err(StoreRefusal::OutsideLink));
    }
    Ok(kind_refusal(&target_meta).map_or(Ok(target), Err))
}

/// How a reader would treat the store entry `path` of the store directory
/// `root`, without opening it: `Ok(None)` when [`read_store_file_in`] may
/// read it, `Ok(Some(refusal))` when it never would (a link loop too), an
/// I/O error when it cannot be resolved (a broken link). Nothing is
/// reported.
pub fn classify_store_entry(path: &Path, root: &Path) -> std::io::Result<Option<StoreRefusal>> {
    Ok(resolve(path, root)?.err())
}

/// Read the store file `path`: a regular file, or a link resolving to one
/// in the link's own directory, of at most [`MAX_STORE_FILE_BYTES`]. Any
/// other entry is refused unread and reported; the error then carries the
/// refusal ([`store_file_refusal`]). A missing entry is `NotFound`.
pub fn read_store_file(path: &Path) -> std::io::Result<Vec<u8>> {
    read_store_file_in(path, own_dir(path))
}

/// [`read_store_file`] for an entry of the store directory `root`, which
/// holds the entry's own directory: a link may also resolve directly into
/// `root`.
pub fn read_store_file_in(path: &Path, root: &Path) -> std::io::Result<Vec<u8>> {
    let open_at = match resolve(path, root)? {
        Ok(open_at) => open_at,
        Err(refusal) => return Err(refuse(path, refusal)),
    };
    #[cfg(test)]
    tests::before_open(&open_at);
    let mut options = fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        // A FIFO swapped in after the check opens at once instead of
        // waiting for a writer; a link swapped in fails to open.
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let file = match options.open(&open_at) {
        Ok(file) => file,
        #[cfg(unix)]
        Err(error) if error.raw_os_error() == Some(libc::ELOOP) => {
            return Err(refuse(path, StoreRefusal::Changed));
        }
        Err(error) => return Err(error),
    };
    let meta = file.metadata()?;
    if let Some(refusal) = kind_refusal(&meta) {
        return Err(refuse(path, refusal));
    }
    let mut bytes = Vec::with_capacity(usize::try_from(meta.len()).unwrap_or_default());
    file.take(MAX_STORE_FILE_BYTES + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_STORE_FILE_BYTES {
        return Err(refuse(path, StoreRefusal::Oversized));
    }
    Ok(bytes)
}

/// [`read_store_file_in`] for a reader that skips what it cannot read:
/// `None` for an absent entry, and for any other failure, which is
/// reported (a refusal already was). Nothing is skipped silently.
pub fn read_store_file_or_report(path: &Path, root: &Path) -> Option<Vec<u8>> {
    match read_store_file_in(path, root) {
        Ok(bytes) => Some(bytes),
        Err(error) if store_file_refusal(&error).is_some() => None,
        Err(_) if fs::symlink_metadata(path).is_err() => None,
        Err(error) => {
            report_skipped_store_entry(path, &error);
            None
        }
    }
}

/// The entries of the store directory `dir`, for a reader that skips what
/// it cannot read: none when `dir` does not exist; a directory or an entry
/// that cannot be listed is reported, never skipped silently.
pub fn store_dir_entries(dir: &Path) -> Vec<PathBuf> {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Vec::new(),
        Err(error) => {
            report_skipped_store_entry(dir, &error);
            return Vec::new();
        }
    };
    let mut paths = Vec::new();
    for entry in entries {
        match entry {
            Ok(entry) => paths.push(entry.path()),
            Err(error) => report_skipped_store_entry(dir, &error),
        }
    }
    paths
}

/// [`read_store_file`] as text: bytes that are not UTF-8 are an
/// `InvalidData` error, as `fs::read_to_string` gives.
pub fn read_store_text(path: &Path) -> std::io::Result<String> {
    String::from_utf8(read_store_file(path)?)
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))
}

#[cfg(test)]
#[path = "store_file_tests.rs"]
mod tests;
